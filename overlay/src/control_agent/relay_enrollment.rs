use crate::pairing::{relay_claim_request_digest, relay_configuration_digest};
use base64::{decode_config, encode_config, URL_SAFE_NO_PAD};
use fs2::FileExt;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, CertificateSigningRequest, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use x509_parser::{
    certification_request::X509CertificationRequest, parse_x509_certificate, pem::parse_x509_pem,
    prelude::FromDer,
};

const REGISTRY_VERSION: u32 = 1;
const MAX_RECORDS: usize = 2_048;
const DEFAULT_EXPIRY_SECONDS: u64 = 600;
const MAX_EXPIRY_SECONDS: u64 = 3_600;
const MAX_CSR_BYTES: usize = 32 * 1024;
const CERTIFICATE_LIFETIME_SECONDS: u64 = 365 * 24 * 60 * 60;
const CLAIM_RECOVERY_SECONDS: u64 = 10 * 60;

pub(super) struct RelayEnrollmentStore {
    registry_dir: PathBuf,
    secret_root: PathBuf,
    identity_dir: PathBuf,
    center_public_key_file: PathBuf,
    instance_id: String,
    guard: Mutex<()>,
    _process_lock: File,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RelayEnrollmentPrepareRequest {
    pub(super) version: u32,
    pub(super) node_id: String,
    pub(super) relay_server: String,
    pub(super) public_endpoint: String,
    pub(super) relay_pool: String,
    pub(super) profile: String,
    #[serde(default)]
    pub(super) wss_endpoint: Option<String>,
    #[serde(default)]
    pub(super) activate_after_health: bool,
    pub(super) max_sessions: u32,
    pub(super) capacity_bandwidth_bps: u64,
    #[serde(default)]
    pub(super) draining: bool,
    #[serde(default)]
    pub(super) fast_media_udp_port: Option<u16>,
    #[serde(default = "default_expiry_seconds")]
    pub(super) expires_in_seconds: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RelayEnrollmentCompleteRequest {
    pub(super) version: u32,
    pub(super) enrollment_id: String,
    pub(super) configuration_digest: String,
    pub(super) request_digest: String,
    pub(super) key_fingerprint: String,
    pub(super) csr_pem: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RelayEnrollmentRevokeRequest {
    pub(super) version: u32,
    pub(super) enrollment_id: String,
    pub(super) configuration_digest: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RelayEnrollmentActivateRequest {
    pub(super) version: u32,
    pub(super) enrollment_id: String,
    pub(super) configuration_digest: String,
    pub(super) operation_id: String,
    pub(super) config_generation: u64,
    pub(super) health_snapshot_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct RelayEnrollmentPrepareResponse {
    pub(super) version: u32,
    pub(super) enrollment_id: String,
    pub(super) configuration_digest: String,
    pub(super) expires_at_unix: u64,
    pub(super) state: String,
    pub(super) reused: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct RelayEnrollmentCompleteResponse {
    pub(super) version: u32,
    pub(super) enrollment_id: String,
    pub(super) configuration_digest: String,
    pub(super) request_digest: String,
    pub(super) key_fingerprint: String,
    pub(super) state: String,
    pub(super) bundle: RelayEnrollmentBundle,
    pub(super) reused: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RelayEnrollmentBundle {
    pub(super) node_id: String,
    pub(super) relay_server: String,
    pub(super) public_endpoint: String,
    pub(super) node_certificate_pem: String,
    pub(super) relay_ca_pem: String,
    pub(super) center_public_key: String,
    pub(super) telemetry_secret: String,
    pub(super) max_sessions: u32,
    pub(super) capacity_bandwidth_bps: u64,
    pub(super) draining: bool,
    pub(super) relay_pool: String,
    pub(super) profile: String,
    pub(super) wss_endpoint: Option<String>,
    pub(super) activate_after_health: bool,
    pub(super) fast_media_udp_port: Option<u16>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct RelayEnrollmentSummary {
    pub(super) version: u32,
    pub(super) enrollment_id: String,
    pub(super) node_id: String,
    pub(super) relay_server: String,
    pub(super) relay_pool: String,
    pub(super) profile: String,
    pub(super) configuration_digest: String,
    pub(super) expires_at_unix: u64,
    pub(super) state: String,
    pub(super) activate_after_health: bool,
    pub(super) key_fingerprint: Option<String>,
    pub(super) activation_operation_id: Option<String>,
    pub(super) activation_config_generation: Option<u64>,
    pub(super) activation_health_snapshot_id: Option<String>,
    pub(super) activated_at_unix: Option<u64>,
}

#[derive(Clone, Debug)]
pub(super) struct EnrollmentError {
    pub(super) status: u16,
    pub(super) code: &'static str,
    pub(super) detail: &'static str,
    pub(super) retryable: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentRecord {
    registry_version: u32,
    instance_id: String,
    enrollment_id: String,
    prepare_idempotency_key_digest: String,
    configuration_digest: String,
    created_at_unix: u64,
    expires_at_unix: u64,
    state: String,
    approved: RelayEnrollmentPrepareRequest,
    #[serde(default)]
    request_digest: Option<String>,
    #[serde(default)]
    key_fingerprint: Option<String>,
    #[serde(default)]
    csr_digest: Option<String>,
    #[serde(default)]
    completed_at_unix: Option<u64>,
    #[serde(default)]
    activation_operation_id: Option<String>,
    #[serde(default)]
    activation_config_generation: Option<u64>,
    #[serde(default)]
    activation_health_snapshot_id: Option<String>,
    #[serde(default)]
    activated_at_unix: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimMarker {
    version: u32,
    enrollment_id: String,
    configuration_digest: String,
    request_digest: String,
    key_fingerprint: String,
    csr_digest: String,
}

impl RelayEnrollmentStore {
    pub(super) fn open(
        instance_id: String,
        instance_id_file: &Path,
        tls_key_file: &Path,
        local_token_file: &Path,
    ) -> Result<Option<Self>, String> {
        let identity_dir = tls_key_file
            .parent()
            .ok_or_else(|| "relay enrollment identity path is invalid".to_owned())?
            .to_path_buf();
        let ca_cert = identity_dir.join("relay-ca.pem");
        let ca_key = identity_dir.join("relay-ca-key.pem");
        if !ca_cert.exists() && !ca_key.exists() {
            return Ok(None);
        }
        if !ca_cert.exists() || !ca_key.exists() {
            return Err("relay enrollment CA is incomplete".to_owned());
        }
        validate_private_file(&ca_key, 1024 * 1024)?;
        validate_regular_file(&ca_cert, 1024 * 1024)?;

        let state_dir = instance_id_file
            .parent()
            .ok_or_else(|| "relay enrollment state path is invalid".to_owned())?;
        let registry_dir = state_dir.join("relay-enrollments");
        let secret_root = paired_persist_root(state_dir)
            .map(|root| root.join("relay-secrets"))
            .unwrap_or_else(|| state_dir.join("relay-secrets"));
        let center_public_key_file = std::env::var_os("STARRY_CENTER_PUBLIC_KEY_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                local_token_file
                    .parent()
                    .unwrap_or(state_dir)
                    .join("center-public-key")
            });
        validate_regular_file(&center_public_key_file, 4096)?;
        validate_center_public_key(
            &fs::read_to_string(&center_public_key_file)
                .map_err(|_| "cannot read Relay enrollment center public key".to_owned())?,
        )?;
        create_private_directory(&registry_dir)?;
        create_private_directory(&secret_root)?;
        let lock_path = registry_dir.join("registry.lock");
        let process_lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|_| "cannot open Relay enrollment registry lock".to_owned())?;
        process_lock
            .try_lock_exclusive()
            .map_err(|_| "Relay enrollment registry is active in another process".to_owned())?;
        Ok(Some(Self {
            registry_dir,
            secret_root,
            identity_dir,
            center_public_key_file,
            instance_id,
            guard: Mutex::new(()),
            _process_lock: process_lock,
        }))
    }

    pub(super) fn prepare(
        &self,
        request: RelayEnrollmentPrepareRequest,
        idempotency_key: &str,
    ) -> Result<RelayEnrollmentPrepareResponse, EnrollmentError> {
        validate_prepare(&request)?;
        validate_idempotency_key(idempotency_key)?;
        let _guard = self.guard.lock().map_err(|_| EnrollmentError::internal())?;
        let records = self.records()?;
        let idempotency_digest = digest(idempotency_key.as_bytes());
        let configuration_digest = approved_digest(&request);
        if let Some(record) = records.iter().find(|record| {
            record.prepare_idempotency_key_digest == idempotency_digest
                && record.configuration_digest == configuration_digest
        }) {
            return Ok(prepare_response(record, true));
        }
        if records.iter().any(|record| {
            record.prepare_idempotency_key_digest == idempotency_digest
                || (record.approved.node_id == request.node_id
                    && !matches!(record.state.as_str(), "revoked" | "expired"))
        }) {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_CONFLICT",
                "The idempotency key or Relay node identity is already bound.",
            ));
        }
        if records.len() >= MAX_RECORDS {
            return Err(EnrollmentError::capacity());
        }
        let now = unix_seconds();
        let record = EnrollmentRecord {
            registry_version: REGISTRY_VERSION,
            instance_id: self.instance_id.clone(),
            enrollment_id: uuid::Uuid::now_v7().to_string(),
            prepare_idempotency_key_digest: idempotency_digest,
            configuration_digest,
            created_at_unix: now,
            expires_at_unix: now.saturating_add(request.expires_in_seconds),
            state: "pending_claim".to_owned(),
            approved: request,
            request_digest: None,
            key_fingerprint: None,
            csr_digest: None,
            completed_at_unix: None,
            activation_operation_id: None,
            activation_config_generation: None,
            activation_health_snapshot_id: None,
            activated_at_unix: None,
        };
        atomic_json(&self.record_path(&record.enrollment_id), &record, false)
            .map_err(|_| EnrollmentError::internal())?;
        Ok(prepare_response(&record, false))
    }

    pub(super) fn complete(
        &self,
        request: RelayEnrollmentCompleteRequest,
    ) -> Result<RelayEnrollmentCompleteResponse, EnrollmentError> {
        validate_complete(&request)?;
        let _guard = self.guard.lock().map_err(|_| EnrollmentError::internal())?;
        let path = self.record_path(&request.enrollment_id);
        let mut record = read_record(&path)?;
        if record.registry_version != REGISTRY_VERSION || record.instance_id != self.instance_id {
            return Err(EnrollmentError::invalid(
                "Relay enrollment registry binding is invalid.",
            ));
        }
        if record.state == "revoked" {
            return Err(EnrollmentError::gone("The Relay enrollment was revoked."));
        }
        if record.expires_at_unix <= unix_seconds() && record.completed_at_unix.is_none() {
            record.state = "expired".to_owned();
            let _ = atomic_json(&path, &record, true);
            return Err(EnrollmentError::gone("The Relay enrollment expired."));
        }
        if record.configuration_digest != request.configuration_digest {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_BINDING_MISMATCH",
                "The approved Relay configuration digest does not match.",
            ));
        }
        let csr_digest = digest(request.csr_pem.as_bytes());
        let csr_key_fingerprint = csr_fingerprint(&request.csr_pem)?;
        if csr_key_fingerprint != request.key_fingerprint
            || relay_claim_request_digest(
                &request.enrollment_id,
                &request.configuration_digest,
                &request.key_fingerprint,
                &request.csr_pem,
            ) != request.request_digest
        {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_BINDING_MISMATCH",
                "The Relay CSR or claim digest does not match the enrollment.",
            ));
        }
        let reused = record.completed_at_unix.is_some();
        if reused
            && (record.request_digest.as_deref() != Some(request.request_digest.as_str())
                || record.key_fingerprint.as_deref() != Some(request.key_fingerprint.as_str())
                || record.csr_digest.as_deref() != Some(csr_digest.as_str()))
        {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_REPLAYED",
                "The enrollment was already claimed by another key or CSR.",
            ));
        }
        if reused
            && record.completed_at_unix.is_none_or(|completed| {
                completed.saturating_add(CLAIM_RECOVERY_SECONDS) < unix_seconds()
            })
        {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_RECOVERY_EXPIRED",
                "The bounded lost-response recovery window has expired.",
            ));
        }
        let relay_dir = self.secret_root.join(&record.approved.node_id);
        create_private_directory(&relay_dir).map_err(|_| EnrollmentError::internal())?;
        let marker = ClaimMarker {
            version: 1,
            enrollment_id: record.enrollment_id.clone(),
            configuration_digest: record.configuration_digest.clone(),
            request_digest: request.request_digest.clone(),
            key_fingerprint: request.key_fingerprint.clone(),
            csr_digest: csr_digest.clone(),
        };
        self.retire_revoked_claim(&relay_dir, &record, &marker)?;
        validate_or_write_claim(&relay_dir.join("claim.json"), &marker)?;
        let certificate_path = relay_dir.join("node-cert.pem");
        if !certificate_path.exists() {
            let certificate = self.issue_certificate(&request.csr_pem)?;
            atomic_write(&certificate_path, certificate.as_bytes(), 0o640, false)
                .map_err(|_| EnrollmentError::internal())?;
        }
        let telemetry_path = relay_dir.join("telemetry.secret");
        if !telemetry_path.exists() {
            let secret = encode_config(sodiumoxide::randombytes::randombytes(32), URL_SAFE_NO_PAD);
            atomic_write(
                &telemetry_path,
                format!("{secret}\n").as_bytes(),
                0o600,
                false,
            )
            .map_err(|_| EnrollmentError::internal())?;
        }
        record.request_digest = Some(request.request_digest.clone());
        record.key_fingerprint = Some(request.key_fingerprint.clone());
        record.csr_digest = Some(csr_digest);
        record.completed_at_unix.get_or_insert_with(unix_seconds);
        if !matches!(record.state.as_str(), "active") {
            record.state = if record.approved.activate_after_health {
                "claimed_pending_health"
            } else {
                "pending_approval"
            }
            .to_owned();
        }
        atomic_json(&path, &record, true).map_err(|_| EnrollmentError::internal())?;
        let bundle = self.bundle(&record, &relay_dir)?;
        Ok(RelayEnrollmentCompleteResponse {
            version: 1,
            enrollment_id: record.enrollment_id,
            configuration_digest: record.configuration_digest,
            request_digest: request.request_digest,
            key_fingerprint: request.key_fingerprint,
            state: record.state,
            bundle,
            reused,
        })
    }

    pub(super) fn activate(
        &self,
        request: RelayEnrollmentActivateRequest,
        activation_ack: &Value,
        relay_snapshot: &Value,
    ) -> Result<RelayEnrollmentSummary, EnrollmentError> {
        validate_activate(&request)?;
        let _guard = self.guard.lock().map_err(|_| EnrollmentError::internal())?;
        let path = self.record_path(&request.enrollment_id);
        let mut record = read_record(&path)?;
        if record.registry_version != REGISTRY_VERSION || record.instance_id != self.instance_id {
            return Err(EnrollmentError::invalid(
                "Relay enrollment registry binding is invalid.",
            ));
        }
        if record.configuration_digest != request.configuration_digest {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_BINDING_MISMATCH",
                "The approved Relay configuration digest does not match.",
            ));
        }
        if !record.approved.activate_after_health {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_PREAUTHORIZATION_REQUIRED",
                "This Relay enrollment was not pre-authorized for health-gated activation.",
            ));
        }
        if record.state == "active" {
            if record.activation_operation_id.as_deref() == Some(request.operation_id.as_str())
                && record.activation_config_generation == Some(request.config_generation)
                && record.activation_health_snapshot_id.as_deref()
                    == Some(request.health_snapshot_id.as_str())
            {
                return Ok(summary(&record));
            }
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_ACTIVATION_CONFLICT",
                "The Relay enrollment is already active under different evidence.",
            ));
        }
        if record.state != "claimed_pending_health" {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_STATE_MISMATCH",
                "The Relay enrollment is not awaiting health-gated activation.",
            ));
        }
        validate_activation_evidence(&record, &request, activation_ack, relay_snapshot)?;
        record.state = "active".to_owned();
        record.activation_operation_id = Some(request.operation_id);
        record.activation_config_generation = Some(request.config_generation);
        record.activation_health_snapshot_id = Some(request.health_snapshot_id);
        record.activated_at_unix = Some(unix_seconds());
        atomic_json(&path, &record, true).map_err(|_| EnrollmentError::internal())?;
        Ok(summary(&record))
    }

    pub(super) fn revoke(
        &self,
        request: RelayEnrollmentRevokeRequest,
    ) -> Result<RelayEnrollmentSummary, EnrollmentError> {
        if request.version != 1 || uuid::Uuid::parse_str(&request.enrollment_id).is_err() {
            return Err(EnrollmentError::invalid(
                "The Relay enrollment request is invalid.",
            ));
        }
        let _guard = self.guard.lock().map_err(|_| EnrollmentError::internal())?;
        let path = self.record_path(&request.enrollment_id);
        let mut record = read_record(&path)?;
        if record.configuration_digest != request.configuration_digest {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_BINDING_MISMATCH",
                "The approved Relay configuration digest does not match.",
            ));
        }
        record.state = "revoked".to_owned();
        atomic_json(&path, &record, true).map_err(|_| EnrollmentError::internal())?;
        self.remove_relay_credentials_if_current(&record)?;
        Ok(summary(&record))
    }

    pub(super) fn list(&self) -> Result<Vec<RelayEnrollmentSummary>, EnrollmentError> {
        let _guard = self.guard.lock().map_err(|_| EnrollmentError::internal())?;
        let mut records = self.records()?;
        records.sort_by(|left, right| left.enrollment_id.cmp(&right.enrollment_id));
        Ok(records.iter().map(summary).collect())
    }

    pub(super) fn get(&self, id: &str) -> Result<RelayEnrollmentSummary, EnrollmentError> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(EnrollmentError::invalid(
                "The Relay enrollment ID is invalid.",
            ));
        }
        let _guard = self.guard.lock().map_err(|_| EnrollmentError::internal())?;
        read_record(&self.record_path(id)).map(|record| summary(&record))
    }

    fn records(&self) -> Result<Vec<EnrollmentRecord>, EnrollmentError> {
        let mut records = Vec::new();
        let entries = fs::read_dir(&self.registry_dir).map_err(|_| EnrollmentError::internal())?;
        for entry in entries {
            let entry = entry.map_err(|_| EnrollmentError::internal())?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if uuid::Uuid::parse_str(id).is_err() {
                continue;
            }
            records.push(read_record(&entry.path())?);
            if records.len() > MAX_RECORDS {
                return Err(EnrollmentError::capacity());
            }
        }
        Ok(records)
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.registry_dir.join(format!("{id}.json"))
    }

    fn retire_revoked_claim(
        &self,
        relay_dir: &Path,
        current: &EnrollmentRecord,
        expected: &ClaimMarker,
    ) -> Result<(), EnrollmentError> {
        let claim_path = relay_dir.join("claim.json");
        if !claim_path.exists() {
            return Ok(());
        }
        let existing: ClaimMarker = serde_json::from_slice(
            &fs::read(&claim_path).map_err(|_| EnrollmentError::internal())?,
        )
        .map_err(|_| EnrollmentError::internal())?;
        if existing == *expected {
            return Ok(());
        }
        let previous = read_record(&self.record_path(&existing.enrollment_id)).map_err(|_| {
            EnrollmentError::conflict(
                "RELAY_ENROLLMENT_REPLAYED",
                "The Relay identity is still bound to another enrollment.",
            )
        })?;
        if previous.approved.node_id != current.approved.node_id
            || !matches!(previous.state.as_str(), "revoked" | "expired")
        {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_REPLAYED",
                "The Relay identity is still bound to another enrollment.",
            ));
        }
        remove_relay_credentials(relay_dir, Some(&existing))
    }

    fn remove_relay_credentials_if_current(
        &self,
        record: &EnrollmentRecord,
    ) -> Result<(), EnrollmentError> {
        let relay_dir = self.secret_root.join(&record.approved.node_id);
        let claim_path = relay_dir.join("claim.json");
        if !claim_path.exists() {
            return Ok(());
        }
        let marker: ClaimMarker = serde_json::from_slice(
            &fs::read(&claim_path).map_err(|_| EnrollmentError::internal())?,
        )
        .map_err(|_| EnrollmentError::internal())?;
        if marker.enrollment_id != record.enrollment_id
            || marker.configuration_digest != record.configuration_digest
        {
            return Ok(());
        }
        remove_relay_credentials(&relay_dir, Some(&marker))
    }

    fn issue_certificate(&self, csr_pem: &str) -> Result<String, EnrollmentError> {
        let mut request = CertificateSigningRequest::from_pem(csr_pem)
            .map_err(|_| EnrollmentError::invalid("The Relay CSR is invalid."))?;
        if request.params.alg != &rcgen::PKCS_ED25519 {
            return Err(EnrollmentError::invalid("The Relay CSR must use Ed25519."));
        }
        let now = SystemTime::now();
        request.params.not_before = now
            .checked_sub(Duration::from_secs(300))
            .unwrap_or(now)
            .into();
        request.params.not_after = now
            .checked_add(Duration::from_secs(CERTIFICATE_LIFETIME_SECONDS))
            .ok_or_else(EnrollmentError::internal)?
            .into();
        request.params.is_ca = IsCa::NoCa;
        request.params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        request.params.extended_key_usages = vec![
            ExtendedKeyUsagePurpose::ClientAuth,
            ExtendedKeyUsagePurpose::ServerAuth,
        ];
        let signer = self.ca_signer()?;
        request
            .serialize_pem_with_signer(&signer)
            .map_err(|_| EnrollmentError::internal())
    }

    fn ca_signer(&self) -> Result<Certificate, EnrollmentError> {
        let private_key = fs::read_to_string(self.identity_dir.join("relay-ca-key.pem"))
            .map_err(|_| EnrollmentError::internal())?;
        let key_pair = KeyPair::from_pem(&private_key).map_err(|_| EnrollmentError::internal())?;
        let certificate = fs::read(self.identity_dir.join("relay-ca.pem"))
            .map_err(|_| EnrollmentError::internal())?;
        let (_, pem) = parse_x509_pem(&certificate).map_err(|_| EnrollmentError::internal())?;
        let (_, parsed) =
            parse_x509_certificate(&pem.contents).map_err(|_| EnrollmentError::internal())?;
        let now = unix_seconds() as i64;
        if digest(parsed.tbs_certificate.subject_pki.raw) != digest(&key_pair.public_key_der())
            || parsed.validity().not_before.timestamp() > now
            || parsed.validity().not_after.timestamp() <= now
            || parsed.verify_signature(None).is_err()
        {
            return Err(EnrollmentError::internal());
        }
        let mut params = CertificateParams::new(Vec::<String>::new());
        params.alg = &rcgen::PKCS_ED25519;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_pair = Some(key_pair);
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, "Starry Relay Enrollment CA v1");
        params.distinguished_name = name;
        Certificate::from_params(params).map_err(|_| EnrollmentError::internal())
    }

    fn bundle(
        &self,
        record: &EnrollmentRecord,
        relay_dir: &Path,
    ) -> Result<RelayEnrollmentBundle, EnrollmentError> {
        let node_certificate_pem = fs::read_to_string(relay_dir.join("node-cert.pem"))
            .map_err(|_| EnrollmentError::internal())?;
        let telemetry_secret = fs::read_to_string(relay_dir.join("telemetry.secret"))
            .map_err(|_| EnrollmentError::internal())?
            .trim()
            .to_owned();
        if decode_config(&telemetry_secret, URL_SAFE_NO_PAD)
            .map(|secret| secret.len() != 32)
            .unwrap_or(true)
        {
            return Err(EnrollmentError::internal());
        }
        let center_public_key = fs::read_to_string(&self.center_public_key_file)
            .map_err(|_| EnrollmentError::internal())?
            .trim()
            .to_owned();
        let relay_ca_pem = fs::read_to_string(self.identity_dir.join("relay-ca.pem"))
            .map_err(|_| EnrollmentError::internal())?;
        validate_issued_certificate(
            &node_certificate_pem,
            &relay_ca_pem,
            record
                .key_fingerprint
                .as_deref()
                .ok_or_else(EnrollmentError::internal)?,
        )?;
        let approved = &record.approved;
        Ok(RelayEnrollmentBundle {
            node_id: approved.node_id.clone(),
            relay_server: approved.relay_server.clone(),
            public_endpoint: approved.public_endpoint.clone(),
            node_certificate_pem,
            relay_ca_pem,
            center_public_key,
            telemetry_secret,
            max_sessions: approved.max_sessions,
            capacity_bandwidth_bps: approved.capacity_bandwidth_bps,
            draining: approved.draining,
            relay_pool: approved.relay_pool.clone(),
            profile: approved.profile.clone(),
            wss_endpoint: approved.wss_endpoint.clone(),
            activate_after_health: approved.activate_after_health,
            fast_media_udp_port: approved.fast_media_udp_port,
        })
    }
}

impl EnrollmentError {
    fn invalid(detail: &'static str) -> Self {
        Self {
            status: 400,
            code: "RELAY_ENROLLMENT_INVALID",
            detail,
            retryable: false,
        }
    }

    fn conflict(code: &'static str, detail: &'static str) -> Self {
        Self {
            status: 409,
            code,
            detail,
            retryable: false,
        }
    }

    fn gone(detail: &'static str) -> Self {
        Self {
            status: 410,
            code: "RELAY_ENROLLMENT_EXPIRED",
            detail,
            retryable: false,
        }
    }

    fn capacity() -> Self {
        Self {
            status: 503,
            code: "RELAY_ENROLLMENT_CAPACITY",
            detail: "The bounded Relay enrollment registry is full.",
            retryable: false,
        }
    }

    fn internal() -> Self {
        Self {
            status: 500,
            code: "RELAY_ENROLLMENT_UNAVAILABLE",
            detail: "The Relay enrollment registry is unavailable.",
            retryable: true,
        }
    }
}

fn validate_prepare(request: &RelayEnrollmentPrepareRequest) -> Result<(), EnrollmentError> {
    if request.version != 1
        || !safe_identifier(&request.node_id)
        || !safe_identifier(&request.relay_pool)
        || request.relay_server != request.public_endpoint
        || !valid_relay_endpoint(&request.relay_server)
        || request.max_sessions == 0
        || request.max_sessions > 1_000_000
        || request.capacity_bandwidth_bps == 0
        || request.expires_in_seconds == 0
        || request.expires_in_seconds > MAX_EXPIRY_SECONDS
        || !matches!(
            request.profile.as_str(),
            "native" | "native-wss" | "native-wss-fastmedia"
        )
        || (request.profile == "native" && request.wss_endpoint.is_some())
        || (request.profile != "native"
            && request
                .wss_endpoint
                .as_deref()
                .is_none_or(|value| !valid_wss_endpoint(value)))
        || (request.profile == "native-wss-fastmedia") != request.fast_media_udp_port.is_some()
    {
        return Err(EnrollmentError::invalid(
            "The approved Relay configuration is invalid.",
        ));
    }
    Ok(())
}

fn validate_complete(request: &RelayEnrollmentCompleteRequest) -> Result<(), EnrollmentError> {
    if request.version != 1
        || uuid::Uuid::parse_str(&request.enrollment_id).is_err()
        || !valid_digest(&request.configuration_digest)
        || !valid_digest(&request.request_digest)
        || !valid_digest(&request.key_fingerprint)
        || request.csr_pem.is_empty()
        || request.csr_pem.len() > MAX_CSR_BYTES
    {
        return Err(EnrollmentError::invalid(
            "The Relay enrollment claim is invalid.",
        ));
    }
    Ok(())
}

fn validate_activate(request: &RelayEnrollmentActivateRequest) -> Result<(), EnrollmentError> {
    if request.version != 1
        || uuid::Uuid::parse_str(&request.enrollment_id).is_err()
        || !valid_digest(&request.configuration_digest)
        || uuid::Uuid::parse_str(&request.operation_id).is_err()
        || request.config_generation == 0
        || request.health_snapshot_id.len() < 8
        || request.health_snapshot_id.len() > 128
        || !request.health_snapshot_id.starts_with("health-")
        || !request
            .health_snapshot_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(EnrollmentError::invalid(
            "The Relay activation evidence request is invalid.",
        ));
    }
    Ok(())
}

fn validate_activation_evidence(
    record: &EnrollmentRecord,
    request: &RelayEnrollmentActivateRequest,
    activation_ack: &Value,
    relay_snapshot: &Value,
) -> Result<(), EnrollmentError> {
    let ack_generation = activation_ack.get("generation").and_then(Value::as_u64);
    let subsystem_acks = activation_ack
        .get("subsystem_acks")
        .and_then(Value::as_array)
        .filter(|acks| {
            !acks.is_empty()
                && acks
                    .iter()
                    .all(|ack| ack.get("accepted").and_then(Value::as_bool) == Some(true))
        });
    if ack_generation != Some(request.config_generation)
        || subsystem_acks.is_none()
        || activation_ack
            .get("source_digest")
            .and_then(Value::as_str)
            .is_none_or(|digest| !valid_digest(digest))
        || activation_ack
            .get("effective_digest")
            .and_then(Value::as_str)
            .is_none_or(|digest| !valid_digest(digest))
    {
        return Err(EnrollmentError::conflict(
            "RELAY_ENROLLMENT_ACTIVATION_ACK_MISMATCH",
            "The configuration activation acknowledgement is incomplete or does not match.",
        ));
    }
    if relay_snapshot
        .get("config_generation")
        .and_then(Value::as_u64)
        != Some(request.config_generation)
        || relay_snapshot
            .get("health_snapshot_id")
            .and_then(Value::as_str)
            != Some(request.health_snapshot_id.as_str())
    {
        return Err(EnrollmentError::conflict(
            "RELAY_ENROLLMENT_HEALTH_MISMATCH",
            "The Relay health snapshot does not match the activated configuration generation.",
        ));
    }
    let relays = relay_snapshot
        .get("relays")
        .and_then(Value::as_array)
        .filter(|relays| relays.len() <= 4_096)
        .ok_or_else(|| {
            EnrollmentError::conflict(
                "RELAY_ENROLLMENT_HEALTH_MISMATCH",
                "The Relay health snapshot is missing or unbounded.",
            )
        })?;
    let relay = relays
        .iter()
        .find(|relay| {
            relay.get("id").and_then(Value::as_str) == Some(record.approved.relay_server.as_str())
        })
        .ok_or_else(|| {
            EnrollmentError::conflict(
                "RELAY_ENROLLMENT_ENDPOINT_DRIFT",
                "The activated Relay endpoint does not match the approved endpoint.",
            )
        })?;
    if relay.pointer("/native/state").and_then(Value::as_str) != Some("online")
        || relay
            .get("version")
            .and_then(Value::as_str)
            .is_none_or(|version| version.is_empty() || version.len() > 128)
    {
        return Err(EnrollmentError::conflict(
            "RELAY_ENROLLMENT_HEALTH_MISMATCH",
            "The approved Relay has not passed the Native and version health gates.",
        ));
    }
    if record.approved.profile == "native" {
        return Ok(());
    }
    let expected_public_probe = record
        .approved
        .wss_endpoint
        .as_deref()
        .map(public_probe_url)
        .ok_or_else(|| EnrollmentError::internal())?;
    let websocket_ready = relay
        .pointer("/websocket/configured")
        .and_then(Value::as_bool)
        == Some(true)
        && relay.pointer("/websocket/state").and_then(Value::as_str) == Some("healthy")
        && relay.pointer("/websocket/stale").and_then(Value::as_bool) == Some(false)
        && relay.pointer("/websocket/url").and_then(Value::as_str)
            == Some(expected_public_probe.as_str())
        && relay
            .pointer("/websocket/telemetry_schema")
            .and_then(Value::as_u64)
            .is_some_and(|schema| schema >= 2)
        && relay
            .pointer("/websocket/telemetry_sequence")
            .and_then(Value::as_u64)
            .is_some_and(|sequence| sequence > 0)
        && relay
            .pointer("/websocket/process_instance_id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty() && id.len() <= 128)
        && relay
            .pointer("/capabilities/relay_probe_protocol")
            .and_then(Value::as_u64)
            .is_some_and(|version| version >= 1)
        && relay
            .pointer("/capabilities/relay_load_protocol")
            .and_then(Value::as_u64)
            .is_some_and(|version| version >= 1)
        && relay
            .pointer("/websocket/capacity_sessions")
            .and_then(Value::as_u64)
            == Some(u64::from(record.approved.max_sessions))
        && relay
            .pointer("/websocket/capacity_bandwidth_bps")
            .and_then(Value::as_u64)
            == Some(record.approved.capacity_bandwidth_bps)
        && relay
            .pointer("/websocket/draining")
            .and_then(Value::as_bool)
            == Some(record.approved.draining);
    if !websocket_ready {
        return Err(EnrollmentError::conflict(
            "RELAY_ENROLLMENT_HEALTH_MISMATCH",
            "The approved Relay has not passed authenticated WSS and telemetry health gates.",
        ));
    }
    if record.approved.profile != "native-wss-fastmedia" {
        return Ok(());
    }
    let expected_port = record.approved.fast_media_udp_port.map(u64::from);
    let fast_media_ready = relay
        .pointer("/capabilities/fast_media_relay_udp")
        .and_then(Value::as_u64)
        == Some(1)
        && relay
            .pointer("/fast_media_udp/configured_port")
            .and_then(Value::as_u64)
            == expected_port
        && relay
            .pointer("/fast_media_udp/reported_port")
            .and_then(Value::as_u64)
            == expected_port
        && relay
            .pointer("/fast_media_udp/enabled")
            .and_then(Value::as_bool)
            == Some(true)
        && relay
            .pointer("/fast_media_udp/healthy")
            .and_then(Value::as_bool)
            == Some(true);
    if !fast_media_ready {
        return Err(EnrollmentError::conflict(
            "RELAY_ENROLLMENT_HEALTH_MISMATCH",
            "The approved Relay has not passed the FastMedia UDP capability and health gates.",
        ));
    }
    Ok(())
}

fn public_probe_url(value: &str) -> String {
    value
        .strip_suffix("/ws/telemetry")
        .map(|prefix| format!("{prefix}/ws/relay"))
        .unwrap_or_else(|| value.to_owned())
}

fn approved_digest(request: &RelayEnrollmentPrepareRequest) -> String {
    relay_configuration_digest(
        &request.node_id,
        &request.relay_server,
        &request.public_endpoint,
        &request.relay_pool,
        &request.profile,
        request.wss_endpoint.as_deref(),
        request.activate_after_health,
        request.max_sessions,
        request.capacity_bandwidth_bps,
        request.draining,
        request.fast_media_udp_port,
    )
}

fn csr_fingerprint(csr_pem: &str) -> Result<String, EnrollmentError> {
    let (_, pem) = parse_x509_pem(csr_pem.as_bytes())
        .map_err(|_| EnrollmentError::invalid("The Relay CSR is invalid."))?;
    let (_, request) = X509CertificationRequest::from_der(&pem.contents)
        .map_err(|_| EnrollmentError::invalid("The Relay CSR is invalid."))?;
    request
        .verify_signature()
        .map_err(|_| EnrollmentError::invalid("The Relay CSR signature is invalid."))?;
    Ok(digest(request.certification_request_info.subject_pki.raw))
}

fn validate_issued_certificate(
    leaf_pem: &str,
    ca_pem: &str,
    expected_key_fingerprint: &str,
) -> Result<(), EnrollmentError> {
    let (_, leaf_pem) =
        parse_x509_pem(leaf_pem.as_bytes()).map_err(|_| EnrollmentError::internal())?;
    let (_, leaf) =
        parse_x509_certificate(&leaf_pem.contents).map_err(|_| EnrollmentError::internal())?;
    let (_, ca_pem) = parse_x509_pem(ca_pem.as_bytes()).map_err(|_| EnrollmentError::internal())?;
    let (_, ca) =
        parse_x509_certificate(&ca_pem.contents).map_err(|_| EnrollmentError::internal())?;
    let now = unix_seconds() as i64;
    if digest(leaf.tbs_certificate.subject_pki.raw) != expected_key_fingerprint
        || leaf.validity().not_before.timestamp() > now
        || leaf.validity().not_after.timestamp() <= now
        || leaf.issuer() != ca.subject()
        || ca.verify_signature(None).is_err()
        || leaf.verify_signature(Some(ca.public_key())).is_err()
    {
        return Err(EnrollmentError::internal());
    }
    Ok(())
}

fn validate_or_write_claim(path: &Path, expected: &ClaimMarker) -> Result<(), EnrollmentError> {
    if path.exists() {
        let existing: ClaimMarker =
            serde_json::from_slice(&fs::read(path).map_err(|_| EnrollmentError::internal())?)
                .map_err(|_| EnrollmentError::internal())?;
        let left = serde_json::to_value(existing).map_err(|_| EnrollmentError::internal())?;
        let right = serde_json::to_value(expected).map_err(|_| EnrollmentError::internal())?;
        if left != right {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_REPLAYED",
                "The enrollment was already claimed by another key or CSR.",
            ));
        }
        return Ok(());
    }
    atomic_json(path, expected, false).map_err(|_| EnrollmentError::internal())
}

fn remove_relay_credentials(
    directory: &Path,
    expected: Option<&ClaimMarker>,
) -> Result<(), EnrollmentError> {
    let claim_path = directory.join("claim.json");
    if let Some(expected) = expected {
        let actual: ClaimMarker = serde_json::from_slice(
            &fs::read(&claim_path).map_err(|_| EnrollmentError::internal())?,
        )
        .map_err(|_| EnrollmentError::internal())?;
        if &actual != expected {
            return Err(EnrollmentError::conflict(
                "RELAY_ENROLLMENT_REPLAYED",
                "The Relay identity changed while credentials were being retired.",
            ));
        }
    }
    // Leave the old claim marker until last. If the process stops midway, a
    // retry still sees the old binding and finishes this bounded cleanup;
    // once the new marker is installed, missing credentials are regenerated.
    for path in [
        directory.join("node-cert.pem"),
        directory.join("telemetry.secret"),
        claim_path,
    ] {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(EnrollmentError::internal());
            }
            Ok(_) => fs::remove_file(&path).map_err(|_| EnrollmentError::internal())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(EnrollmentError::internal()),
        }
    }
    File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| EnrollmentError::internal())
}

fn read_record(path: &Path) -> Result<EnrollmentRecord, EnrollmentError> {
    let bytes = fs::read(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            EnrollmentError::gone("The Relay enrollment does not exist.")
        } else {
            EnrollmentError::internal()
        }
    })?;
    serde_json::from_slice(&bytes).map_err(|_| EnrollmentError::internal())
}

fn prepare_response(record: &EnrollmentRecord, reused: bool) -> RelayEnrollmentPrepareResponse {
    RelayEnrollmentPrepareResponse {
        version: 1,
        enrollment_id: record.enrollment_id.clone(),
        configuration_digest: record.configuration_digest.clone(),
        expires_at_unix: record.expires_at_unix,
        state: record.state.clone(),
        reused,
    }
}

fn summary(record: &EnrollmentRecord) -> RelayEnrollmentSummary {
    RelayEnrollmentSummary {
        version: 1,
        enrollment_id: record.enrollment_id.clone(),
        node_id: record.approved.node_id.clone(),
        relay_server: record.approved.relay_server.clone(),
        relay_pool: record.approved.relay_pool.clone(),
        profile: record.approved.profile.clone(),
        configuration_digest: record.configuration_digest.clone(),
        expires_at_unix: record.expires_at_unix,
        state: record.state.clone(),
        activate_after_health: record.approved.activate_after_health,
        key_fingerprint: record.key_fingerprint.clone(),
        activation_operation_id: record.activation_operation_id.clone(),
        activation_config_generation: record.activation_config_generation,
        activation_health_snapshot_id: record.activation_health_snapshot_id.clone(),
        activated_at_unix: record.activated_at_unix,
    }
}

fn paired_persist_root(state_dir: &Path) -> Option<&Path> {
    (state_dir.file_name()?.to_str()? == "state"
        && state_dir.parent()?.file_name()?.to_str()? == "control")
        .then(|| state_dir.parent()?.parent())
        .flatten()
}

fn valid_relay_endpoint(value: &str) -> bool {
    if value.len() > 256 || value.chars().any(char::is_whitespace) {
        return false;
    }
    let Ok(parsed) = url::Url::parse(&format!("tcp://{value}")) else {
        return false;
    };
    parsed.host_str().is_some()
        && parsed.port().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.path().is_empty()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
}

fn valid_wss_endpoint(value: &str) -> bool {
    if value.len() > 512 || value.chars().any(char::is_whitespace) {
        return false;
    }
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    parsed.scheme() == "wss"
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.path() == "/ws/telemetry"
        && parsed.query().is_none()
        && parsed.fragment().is_none()
}

fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_idempotency_key(value: &str) -> Result<(), EnrollmentError> {
    if !(8..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(EnrollmentError::invalid("Idempotency-Key is invalid."));
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_center_public_key(value: &str) -> Result<(), String> {
    if base64::decode(value.trim())
        .map(|key| key.len() != 32)
        .unwrap_or(true)
    {
        return Err("Relay enrollment center public key is invalid".to_owned());
    }
    Ok(())
}

fn validate_regular_file(path: &Path, maximum: u64) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| format!("required state file is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > maximum {
        return Err(format!("state file is unsafe: {}", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 || metadata.mode() & 0o022 != 0 {
            return Err(format!(
                "state file permissions are unsafe: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn validate_private_file(path: &Path, maximum: u64) -> Result<(), String> {
    validate_regular_file(path, maximum)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path)
            .map_err(|_| format!("cannot inspect private state file: {}", path.display()))?;
        if metadata.mode() & 0o077 != 0 || metadata.nlink() != 1 {
            return Err(format!(
                "private state file permissions are unsafe: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|_| format!("cannot create state directory: {}", path.display()))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| format!("cannot inspect state directory: {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("state directory is unsafe: {}", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| format!("cannot secure state directory: {}", path.display()))?;
    }
    Ok(())
}

fn atomic_json<T: serde::Serialize>(path: &Path, value: &T, replace: bool) -> Result<(), String> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|_| "cannot serialize state".to_owned())?;
    bytes.push(b'\n');
    atomic_write(path, &bytes, 0o600, replace)
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32, replace: bool) -> Result<(), String> {
    if path.exists() && !replace {
        return Err("state file already exists".to_owned());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "state path is invalid".to_owned())?;
    create_private_directory(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state"),
        uuid::Uuid::now_v7()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| "cannot create state".to_owned())?;
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        let _ = fs::remove_file(&temporary);
        return Err("cannot persist state".to_owned());
    }
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))
            .map_err(|_| "cannot secure state".to_owned())?;
    }
    if replace {
        fs::rename(&temporary, path).map_err(|_| "cannot atomically replace state".to_owned())?;
    } else {
        match fs::hard_link(&temporary, path) {
            Ok(()) => fs::remove_file(&temporary)
                .map_err(|_| "cannot clean temporary state".to_owned())?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&temporary);
                return Err("state file already exists".to_owned());
            }
            Err(_) => {
                let _ = fs::remove_file(&temporary);
                return Err("cannot atomically install state".to_owned());
            }
        }
    }
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "cannot sync state directory".to_owned())
}

fn digest(value: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(value))
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn default_expiry_seconds() -> u64 {
    DEFAULT_EXPIRY_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> RelayEnrollmentPrepareRequest {
        RelayEnrollmentPrepareRequest {
            version: 1,
            node_id: "relay-sg".to_owned(),
            relay_server: "relay.example:21117".to_owned(),
            public_endpoint: "relay.example:21117".to_owned(),
            relay_pool: "primary".to_owned(),
            profile: "native-wss-fastmedia".to_owned(),
            wss_endpoint: Some("wss://relay.example:21119/ws/telemetry".to_owned()),
            activate_after_health: true,
            max_sessions: 1_000,
            capacity_bandwidth_bps: 1_000_000_000,
            draining: false,
            fast_media_udp_port: Some(22119),
            expires_in_seconds: 600,
        }
    }

    #[test]
    fn approved_profile_and_endpoint_are_strictly_bound() {
        let valid = request();
        assert!(validate_prepare(&valid).is_ok());
        let digest = approved_digest(&valid);
        let mut changed = valid;
        changed.fast_media_udp_port = Some(22120);
        assert_ne!(digest, approved_digest(&changed));
        changed.profile = "native".to_owned();
        assert!(validate_prepare(&changed).is_err());
    }

    #[test]
    fn identifiers_cannot_escape_the_persistent_root() {
        assert!(safe_identifier("relay-sg_1.example"));
        assert!(!safe_identifier("../relay"));
        assert!(!safe_identifier(".hidden"));
        assert!(!safe_identifier("relay/sg"));
    }

    #[test]
    fn credential_retirement_requires_the_exact_claim_and_recovers_partial_cleanup() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path();
        let marker = ClaimMarker {
            version: 1,
            enrollment_id: uuid::Uuid::now_v7().to_string(),
            configuration_digest: format!("sha256:{}", "1".repeat(64)),
            request_digest: format!("sha256:{}", "2".repeat(64)),
            key_fingerprint: format!("sha256:{}", "3".repeat(64)),
            csr_digest: format!("sha256:{}", "4".repeat(64)),
        };
        atomic_json(&directory.join("claim.json"), &marker, false).unwrap();
        atomic_write(
            &directory.join("node-cert.pem"),
            b"staging-certificate\n",
            0o640,
            false,
        )
        .unwrap();
        atomic_write(
            &directory.join("telemetry.secret"),
            b"staging-secret\n",
            0o600,
            false,
        )
        .unwrap();

        let mut conflicting = marker.clone();
        conflicting.request_digest = format!("sha256:{}", "5".repeat(64));
        let rejected = remove_relay_credentials(directory, Some(&conflicting)).unwrap_err();
        assert_eq!(rejected.code, "RELAY_ENROLLMENT_REPLAYED");
        assert!(directory.join("claim.json").exists());
        assert!(directory.join("node-cert.pem").exists());
        assert!(directory.join("telemetry.secret").exists());

        // A stop after the first removal leaves the claim as the durable
        // binding. The exact retry completes cleanup without broad deletion.
        fs::remove_file(directory.join("node-cert.pem")).unwrap();
        remove_relay_credentials(directory, Some(&marker)).unwrap();
        assert!(!directory.join("claim.json").exists());
        assert!(!directory.join("telemetry.secret").exists());
    }

    #[test]
    fn prepare_claim_and_lost_response_retry_are_idempotent() {
        sodiumoxide::init().unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let state_dir = root.join("control/state");
        let identity_dir = root.join("control/identity");
        let shared_dir = root.join("control/shared");
        create_private_directory(&state_dir).unwrap();
        create_private_directory(&identity_dir).unwrap();
        create_private_directory(&shared_dir).unwrap();

        let mut ca_params = CertificateParams::new(Vec::<String>::new());
        ca_params.alg = &rcgen::PKCS_ED25519;
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        let mut ca_name = DistinguishedName::new();
        ca_name.push(DnType::CommonName, "Starry Relay Enrollment CA v1");
        ca_params.distinguished_name = ca_name;
        let ca = Certificate::from_params(ca_params).unwrap();
        atomic_write(
            &identity_dir.join("relay-ca-key.pem"),
            ca.serialize_private_key_pem().as_bytes(),
            0o600,
            false,
        )
        .unwrap();
        atomic_write(
            &identity_dir.join("relay-ca.pem"),
            ca.serialize_pem().unwrap().as_bytes(),
            0o640,
            false,
        )
        .unwrap();
        atomic_write(
            &shared_dir.join("center-public-key"),
            format!("{}\n", base64::encode([9_u8; 32])).as_bytes(),
            0o640,
            false,
        )
        .unwrap();
        let instance = uuid::Uuid::now_v7().to_string();
        let store = RelayEnrollmentStore::open(
            instance,
            &state_dir.join("instance-id"),
            &identity_dir.join("server-key.pem"),
            &shared_dir.join("local-control.token"),
        )
        .unwrap()
        .unwrap();
        let prepared = store.prepare(request(), "test-enrollment-0001").unwrap();
        let prepared_retry = store.prepare(request(), "test-enrollment-0001").unwrap();
        assert_eq!(prepared.enrollment_id, prepared_retry.enrollment_id);
        assert!(prepared_retry.reused);

        let key = KeyPair::generate(&rcgen::PKCS_ED25519).unwrap();
        let key_fingerprint = digest(&key.public_key_der());
        let mut csr_params = CertificateParams::new(Vec::<String>::new());
        csr_params.alg = &rcgen::PKCS_ED25519;
        csr_params.key_pair = Some(key);
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, "relay-sg");
        csr_params.distinguished_name = name;
        let csr = Certificate::from_params(csr_params)
            .unwrap()
            .serialize_request_pem()
            .unwrap();
        let request_digest = relay_claim_request_digest(
            &prepared.enrollment_id,
            &prepared.configuration_digest,
            &key_fingerprint,
            &csr,
        );
        let claim = RelayEnrollmentCompleteRequest {
            version: 1,
            enrollment_id: prepared.enrollment_id,
            configuration_digest: prepared.configuration_digest,
            request_digest,
            key_fingerprint,
            csr_pem: csr,
        };
        let completed = store.complete(claim.clone()).unwrap();
        assert_eq!(completed.bundle.profile, "native-wss-fastmedia");
        assert_eq!(completed.bundle.fast_media_udp_port, Some(22119));
        assert!(!completed.bundle.telemetry_secret.is_empty());
        assert!(!completed.reused);
        let retry = store.complete(claim.clone()).unwrap();
        assert!(retry.reused);
        assert_eq!(
            completed.bundle.telemetry_secret,
            retry.bundle.telemetry_secret
        );
        assert_eq!(
            completed.bundle.node_certificate_pem,
            retry.bundle.node_certificate_pem
        );

        let operation_id = uuid::Uuid::now_v7().to_string();
        let activation = RelayEnrollmentActivateRequest {
            version: 1,
            enrollment_id: completed.enrollment_id.clone(),
            configuration_digest: completed.configuration_digest.clone(),
            operation_id: operation_id.clone(),
            config_generation: 42,
            health_snapshot_id: "health-17".to_owned(),
        };
        let activation_ack = serde_json::json!({
            "source_digest": digest(b"source"),
            "effective_digest": digest(b"effective"),
            "schema_version": 5,
            "generation": 42,
            "subsystem_acks": [{"subsystem": "relay_health", "accepted": true}]
        });
        let relay_snapshot = serde_json::json!({
            "config_generation": 42,
            "health_snapshot_id": "health-17",
            "relays": [{
                "id": "relay.example:21117",
                "version": "1.1.16-patch-v1.3.1",
                "capabilities": {
                    "relay_probe_protocol": 1,
                    "relay_load_protocol": 1,
                    "fast_media_relay_udp": 1
                },
                "native": {"state": "online"},
                "websocket": {
                    "configured": true,
                    "url": "wss://relay.example:21119/ws/relay",
                    "state": "healthy",
                    "stale": false,
                    "telemetry_schema": 2,
                    "telemetry_sequence": 9,
                    "process_instance_id": "relay-process-1",
                    "capacity_sessions": 1000,
                    "capacity_bandwidth_bps": 1000000000,
                    "draining": false
                },
                "fast_media_udp": {
                    "configured_port": 22119,
                    "reported_port": 22119,
                    "enabled": true,
                    "healthy": true
                }
            }]
        });
        let mut stale_snapshot = relay_snapshot.clone();
        stale_snapshot["relays"][0]["websocket"]["stale"] = serde_json::json!(true);
        let rejected = store
            .activate(activation.clone(), &activation_ack, &stale_snapshot)
            .unwrap_err();
        assert_eq!(rejected.code, "RELAY_ENROLLMENT_HEALTH_MISMATCH");

        let active = store
            .activate(activation.clone(), &activation_ack, &relay_snapshot)
            .unwrap();
        assert_eq!(active.state, "active");
        assert_eq!(
            active.activation_operation_id.as_deref(),
            Some(operation_id.as_str())
        );
        assert_eq!(active.activation_config_generation, Some(42));
        assert_eq!(
            active.activation_health_snapshot_id.as_deref(),
            Some("health-17")
        );
        assert!(active.activated_at_unix.is_some());
        let active_retry = store
            .activate(activation, &activation_ack, &relay_snapshot)
            .unwrap();
        assert_eq!(active_retry.state, "active");

        // A lost claim response may still be retried after activation without
        // regressing the durable enrollment state back to pending health.
        let post_activation_claim_retry = store.complete(claim.clone()).unwrap();
        assert!(post_activation_claim_retry.reused);
        assert_eq!(post_activation_claim_retry.state, "active");

        let record_path = store.record_path(&completed.enrollment_id);
        let mut outside_recovery = read_record(&record_path).unwrap();
        outside_recovery.completed_at_unix = Some(
            unix_seconds()
                .saturating_sub(CLAIM_RECOVERY_SECONDS)
                .saturating_sub(1),
        );
        atomic_json(&record_path, &outside_recovery, true).unwrap();
        let expired_recovery = store.complete(claim).unwrap_err();
        assert_eq!(expired_recovery.code, "RELAY_ENROLLMENT_RECOVERY_EXPIRED");

        let relay_dir = root.join("relay-secrets/relay-sg");
        let stale_claim = fs::read(relay_dir.join("claim.json")).unwrap();
        let stale_certificate = fs::read(relay_dir.join("node-cert.pem")).unwrap();
        let stale_telemetry = fs::read(relay_dir.join("telemetry.secret")).unwrap();
        let stale_telemetry_value = String::from_utf8(stale_telemetry.clone())
            .unwrap()
            .trim()
            .to_owned();
        let revoked = store
            .revoke(RelayEnrollmentRevokeRequest {
                version: 1,
                enrollment_id: completed.enrollment_id,
                configuration_digest: completed.configuration_digest,
            })
            .unwrap();
        assert_eq!(revoked.state, "revoked");
        assert!(!relay_dir.join("claim.json").exists());
        assert!(!relay_dir.join("node-cert.pem").exists());
        assert!(!relay_dir.join("telemetry.secret").exists());

        // Simulate a pre-fix revoked record whose per-node credentials were
        // left behind. A new enrollment may retire exactly that revoked
        // claim, while active and unknown claims remain fail-closed.
        atomic_write(&relay_dir.join("claim.json"), &stale_claim, 0o600, false).unwrap();
        atomic_write(
            &relay_dir.join("node-cert.pem"),
            &stale_certificate,
            0o640,
            false,
        )
        .unwrap();
        atomic_write(
            &relay_dir.join("telemetry.secret"),
            &stale_telemetry,
            0o600,
            false,
        )
        .unwrap();

        let replacement = store.prepare(request(), "test-enrollment-0002").unwrap();
        let replacement_key = KeyPair::generate(&rcgen::PKCS_ED25519).unwrap();
        let replacement_fingerprint = digest(&replacement_key.public_key_der());
        let mut replacement_params = CertificateParams::new(Vec::<String>::new());
        replacement_params.alg = &rcgen::PKCS_ED25519;
        replacement_params.key_pair = Some(replacement_key);
        let replacement_csr = Certificate::from_params(replacement_params)
            .unwrap()
            .serialize_request_pem()
            .unwrap()
            .trim()
            .to_owned();
        let replacement_digest = relay_claim_request_digest(
            &replacement.enrollment_id,
            &replacement.configuration_digest,
            &replacement_fingerprint,
            &replacement_csr,
        );
        let replacement = store
            .complete(RelayEnrollmentCompleteRequest {
                version: 1,
                enrollment_id: replacement.enrollment_id,
                configuration_digest: replacement.configuration_digest,
                request_digest: replacement_digest,
                key_fingerprint: replacement_fingerprint,
                csr_pem: replacement_csr,
            })
            .unwrap();
        assert_eq!(replacement.state, "claimed_pending_health");
        assert_ne!(replacement.bundle.telemetry_secret, stale_telemetry_value);
    }
}

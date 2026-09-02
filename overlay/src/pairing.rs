use base64::{decode_config, encode_config, URL_SAFE_NO_PAD};
use hbb_common::tokio::time::{timeout, Duration};
use rcgen::{Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair};
use serde_derive::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use x509_parser::{parse_x509_certificate, pem::parse_x509_pem};

pub const PROTOCOL_VERSION: u32 = 1;
const MAX_PAIRING_CODE_BYTES: usize = 4_096;
const MAX_CLAIM_RESPONSE_BYTES: usize = 1024 * 1024;
const CLAIM_TIMEOUT_SECONDS: u64 = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Purpose {
    ControlAgent,
    Relay,
}

impl Purpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::ControlAgent => "control-agent",
            Self::Relay => "relay",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "control-agent" => Some(Self::ControlAgent),
            "relay" => Some(Self::Relay),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PairingCodePayload {
    version: u32,
    purpose: String,
    broker_origin: String,
    broker_spki_sha256: String,
    enrollment_id: String,
    configuration_digest: String,
    expires_at_unix: u64,
    secret: String,
}

#[derive(Clone, Debug)]
pub struct PairingCode {
    purpose: Purpose,
    broker_origin: String,
    broker_spki_sha256: String,
    enrollment_id: String,
    configuration_digest: String,
    expires_at_unix: u64,
    secret: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlPairMode {
    Pair,
    Adopt,
    Rotate,
}

impl ControlPairMode {
    fn action(self) -> &'static str {
        match self {
            Self::Pair => "pair",
            Self::Adopt => "adopt",
            Self::Rotate => "rotate",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ControlPairOptions {
    pub mode: ControlPairMode,
    pub tls_server_name: Option<String>,
    pub state_dir: PathBuf,
    pub identity_dir: PathBuf,
    pub output: PathBuf,
    pub shared_dir: PathBuf,
    pub managed_config_path: PathBuf,
    pub backup_dir: PathBuf,
    pub listen: SocketAddr,
    pub local_control_address: SocketAddr,
    pub broker_ca_file: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct RelayEnrollOptions {
    pub data_dir: PathBuf,
    pub broker_ca_file: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PairingSummary {
    pub purpose: &'static str,
    pub enrollment_state: &'static str,
    pub identity_fingerprint: String,
    pub output: String,
    pub restart_required: bool,
}

#[derive(Serialize)]
struct ClaimRequest<'a> {
    version: u32,
    purpose: &'a str,
    action: &'a str,
    enrollment_id: &'a str,
    configuration_digest: &'a str,
    secret: &'a str,
    request_digest: &'a str,
    key_fingerprint: &'a str,
    csr_pem: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimResponse {
    version: u32,
    purpose: String,
    enrollment_id: String,
    configuration_digest: String,
    request_digest: String,
    key_fingerprint: String,
    bundle: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ControlAgentBundle {
    instance_id: String,
    agent_origin: String,
    server_certificate_pem: String,
    client_ca_pem: String,
    allowed_client_uri_sans: Vec<String>,
    service_jwks: Value,
    service_jwt_issuer: String,
    service_jwt_audience_prefix: String,
    center_public_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RelayBundle {
    node_id: String,
    relay_server: String,
    public_endpoint: String,
    node_certificate_pem: String,
    relay_ca_pem: String,
    center_public_key: String,
    telemetry_secret: String,
    max_sessions: u32,
    capacity_bandwidth_bps: u64,
    draining: bool,
    relay_pool: String,
    profile: String,
    #[serde(default)]
    wss_endpoint: Option<String>,
    activate_after_health: bool,
    #[serde(default)]
    fast_media_udp_port: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PendingIdentity {
    version: u32,
    purpose: String,
    action: String,
    enrollment_id: String,
    configuration_digest: String,
    request_digest: String,
    key_fingerprint: String,
    csr_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance_id: Option<String>,
}

#[derive(Serialize)]
struct ControlIdentityMarker<'a> {
    version: u32,
    purpose: &'a str,
    enrollment_id: &'a str,
    configuration_digest: &'a str,
    instance_id: &'a str,
    key_fingerprint: &'a str,
    generation: u64,
}

#[derive(Serialize)]
struct RelayIdentityMarker<'a> {
    version: u32,
    purpose: &'a str,
    enrollment_id: &'a str,
    configuration_digest: &'a str,
    node_id: &'a str,
    relay_server: &'a str,
    key_fingerprint: &'a str,
    host_binding: &'a str,
    generation: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledControlIdentity {
    version: u32,
    purpose: String,
    enrollment_id: String,
    configuration_digest: String,
    instance_id: String,
    key_fingerprint: String,
    generation: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledRelayIdentity {
    version: u32,
    purpose: String,
    enrollment_id: String,
    configuration_digest: String,
    node_id: String,
    relay_server: String,
    key_fingerprint: String,
    host_binding: String,
    generation: u64,
}

#[derive(Serialize)]
struct RelayRuntimeConfig<'a> {
    version: u32,
    node_id: &'a str,
    relay_server: &'a str,
    public_endpoint: &'a str,
    telemetry_secret_file: String,
    max_sessions: u32,
    capacity_bandwidth_bps: u64,
    draining: bool,
    relay_pool: &'a str,
    profile: &'a str,
    wss_endpoint: Option<&'a str>,
    activate_after_health: bool,
    fast_media_udp_port: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GeneratedAgentConfig {
    version: u32,
    instance_id_file: String,
    listen: SocketAddr,
    tls: GeneratedTlsConfig,
    service_jwt: GeneratedServiceJwtConfig,
    local_control: GeneratedLocalControlConfig,
    config: GeneratedManagedConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GeneratedTlsConfig {
    ca_file: String,
    cert_file: String,
    key_file: String,
    allowed_client_uri_sans: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GeneratedServiceJwtConfig {
    issuer: String,
    jwks_file: String,
    audience_prefix: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GeneratedLocalControlConfig {
    address: SocketAddr,
    token_file: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GeneratedManagedConfig {
    path: String,
    backup_dir: String,
    max_bytes: usize,
    write_enabled: bool,
}

impl PairingCode {
    pub fn parse(input: &str, expected: Purpose) -> Result<Self, String> {
        let input = input.trim();
        if input.len() > MAX_PAIRING_CODE_BYTES || !input.starts_with("SP1.") {
            return Err("pairing_code_invalid".to_owned());
        }
        let encoded = &input[4..];
        if encoded.is_empty() || encoded.contains('.') {
            return Err("pairing_code_invalid".to_owned());
        }
        let decoded = decode_config(encoded, URL_SAFE_NO_PAD)
            .map_err(|_| "pairing_code_invalid".to_owned())?;
        let payload: PairingCodePayload =
            serde_json::from_slice(&decoded).map_err(|_| "pairing_code_invalid".to_owned())?;
        let purpose = Purpose::parse(&payload.purpose)
            .ok_or_else(|| "pairing_code_purpose_invalid".to_owned())?;
        if payload.version != PROTOCOL_VERSION || purpose != expected {
            return Err("pairing_code_purpose_mismatch".to_owned());
        }
        validate_origin(&payload.broker_origin)?;
        validate_digest(&payload.broker_spki_sha256, "pairing_code_pin_invalid")?;
        validate_digest(
            &payload.configuration_digest,
            "pairing_code_config_digest_invalid",
        )?;
        uuid::Uuid::parse_str(&payload.enrollment_id)
            .map_err(|_| "pairing_code_enrollment_invalid".to_owned())?;
        let secret = decode_config(&payload.secret, URL_SAFE_NO_PAD)
            .map_err(|_| "pairing_code_secret_invalid".to_owned())?;
        if secret.len() != 32 {
            return Err("pairing_code_secret_invalid".to_owned());
        }
        let now = unix_seconds();
        if payload.expires_at_unix <= now || payload.expires_at_unix > now.saturating_add(60 * 60) {
            return Err("pairing_code_expired_or_unbounded".to_owned());
        }
        Ok(Self {
            purpose,
            broker_origin: payload.broker_origin,
            broker_spki_sha256: payload.broker_spki_sha256,
            enrollment_id: payload.enrollment_id,
            configuration_digest: payload.configuration_digest,
            expires_at_unix: payload.expires_at_unix,
            secret: payload.secret,
        })
    }

    #[cfg(test)]
    fn encode(payload: PairingCodePayload) -> String {
        format!(
            "SP1.{}",
            encode_config(serde_json::to_vec(&payload).unwrap(), URL_SAFE_NO_PAD)
        )
    }
}

pub fn read_pairing_code(code_file: Option<&Path>) -> Result<String, String> {
    let mut value = String::new();
    match code_file {
        Some(path) => {
            validate_private_input(path)?;
            value = fs::read_to_string(path).map_err(|_| "pairing_code_unreadable".to_owned())?;
        }
        None => {
            if atty_stdin() {
                eprint!("Paste pairing code: ");
                io::stderr().flush().ok();
            }
            io::stdin()
                .read_line(&mut value)
                .map_err(|_| "pairing_code_unreadable".to_owned())?;
        }
    }
    let trimmed = value.trim().to_owned();
    value.clear();
    if trimmed.len() > MAX_PAIRING_CODE_BYTES {
        return Err("pairing_code_invalid".to_owned());
    }
    Ok(trimmed)
}

pub async fn control_pair(
    code: &str,
    options: ControlPairOptions,
) -> Result<PairingSummary, String> {
    let code = PairingCode::parse(code, Purpose::ControlAgent)?;
    validate_control_paths(&options)?;
    create_private_directory(&options.state_dir)?;
    create_private_directory(&options.identity_dir)?;
    create_private_directory(&options.shared_dir)?;
    create_private_directory(&options.backup_dir)?;
    if let Some(parent) = options.output.parent() {
        create_private_directory(parent)?;
    }
    if let Some(parent) = options.managed_config_path.parent() {
        create_private_directory(parent)?;
    }
    for directory in [
        options.state_dir.as_path(),
        options.identity_dir.as_path(),
        options.shared_dir.as_path(),
        options.backup_dir.as_path(),
        options
            .output
            .parent()
            .ok_or_else(|| "control_pairing_path_layout_invalid".to_owned())?,
        options
            .managed_config_path
            .parent()
            .ok_or_else(|| "control_pairing_path_layout_invalid".to_owned())?,
    ] {
        validate_persistent_directory(directory)?;
    }

    let completed_marker = options.state_dir.join("identity.json");
    let pending_path = options.state_dir.join("pairing.pending.json");
    let pending_key_path = options.state_dir.join("server-key.pending.pem");
    if let Some(summary) = completed_control_retry(
        &code,
        &options,
        &completed_marker,
        &pending_path,
        &pending_key_path,
    )? {
        return Ok(summary);
    }
    if completed_marker.exists() && options.mode == ControlPairMode::Pair {
        return Err("control_identity_exists_use_adopt_or_rotate".to_owned());
    }
    if !completed_marker.exists() && options.mode == ControlPairMode::Rotate {
        return Err("control_identity_missing_for_rotate".to_owned());
    }

    let existing_instance = read_optional_trimmed(&options.state_dir.join("instance-id"), 64)?;
    let pending_instance = if options.mode == ControlPairMode::Pair && existing_instance.is_none() {
        recover_pending_control_instance(&pending_path, &code)?
    } else {
        None
    };
    let instance_id = match (options.mode, existing_instance) {
        (ControlPairMode::Pair, None) => {
            pending_instance.unwrap_or_else(|| uuid::Uuid::now_v7().to_string())
        }
        (ControlPairMode::Pair, Some(value)) if pending_path.exists() => {
            // A first pairing may have reached the durable install phase before
            // the process or host stopped. The pending record below still has
            // to match every SP1/CSR/configuration binding before this value is
            // accepted, so this resumes the same transaction without adopting
            // or replacing an unrelated identity.
            uuid::Uuid::parse_str(&value).map_err(|_| "control_instance_id_invalid".to_owned())?;
            value
        }
        (ControlPairMode::Pair, Some(_)) => {
            return Err("control_instance_exists_use_adopt".to_owned())
        }
        (_, Some(value)) => {
            uuid::Uuid::parse_str(&value).map_err(|_| "control_instance_id_invalid".to_owned())?;
            value
        }
        (_, None) => return Err("control_instance_id_missing".to_owned()),
    };

    let (private_key_pem, csr_pem, key_fingerprint) = load_or_create_pending_key(
        &pending_key_path,
        options.mode == ControlPairMode::Adopt,
        options.mode != ControlPairMode::Pair,
        &options.identity_dir.join("server-key.pem"),
        &instance_id,
        options.tls_server_name.as_deref(),
    )?;
    let request_digest = claim_request_digest(
        code.purpose,
        options.mode.action(),
        &code.enrollment_id,
        &code.configuration_digest,
        &key_fingerprint,
        &csr_pem,
        Some(&instance_id),
    );
    let pending = PendingIdentity {
        version: 1,
        purpose: code.purpose.as_str().to_owned(),
        action: options.mode.action().to_owned(),
        enrollment_id: code.enrollment_id.clone(),
        configuration_digest: code.configuration_digest.clone(),
        request_digest: request_digest.clone(),
        key_fingerprint: key_fingerprint.clone(),
        csr_digest: digest(csr_pem.as_bytes()),
        instance_id: Some(instance_id.clone()),
    };
    validate_or_write_pending(&pending_path, &pending)?;

    let claim = ClaimRequest {
        version: 1,
        purpose: code.purpose.as_str(),
        action: options.mode.action(),
        enrollment_id: &code.enrollment_id,
        configuration_digest: &code.configuration_digest,
        secret: &code.secret,
        request_digest: &request_digest,
        key_fingerprint: &key_fingerprint,
        csr_pem: &csr_pem,
        instance_id: Some(&instance_id),
    };
    let response = claim_broker(&code, &claim, options.broker_ca_file.as_deref()).await?;
    let bundle: ControlAgentBundle = validate_claim_response(&code, &claim, response)?;
    validate_control_bundle(
        &bundle,
        &instance_id,
        &key_fingerprint,
        &code.configuration_digest,
    )?;

    install_control_identity(
        &options,
        &code,
        &bundle,
        &instance_id,
        &key_fingerprint,
        &private_key_pem,
    )?;
    remove_pending(&pending_path, &pending_key_path)?;
    Ok(PairingSummary {
        purpose: Purpose::ControlAgent.as_str(),
        enrollment_state: "paired",
        identity_fingerprint: key_fingerprint,
        output: options.output.display().to_string(),
        restart_required: options.mode != ControlPairMode::Pair,
    })
}

pub async fn relay_enroll(
    code: &str,
    options: RelayEnrollOptions,
) -> Result<PairingSummary, String> {
    let code = PairingCode::parse(code, Purpose::Relay)?;
    if !options.data_dir.is_absolute() {
        return Err("relay_data_dir_not_absolute".to_owned());
    }
    create_private_directory(&options.data_dir)?;
    validate_persistent_directory(&options.data_dir)?;
    let enrollment_dir = options.data_dir.join("starry/enrollment");
    create_private_directory(&enrollment_dir)?;
    let completed_marker = enrollment_dir.join("enrollment.json");
    let pending_path = enrollment_dir.join("pairing.pending.json");
    let pending_key_path = enrollment_dir.join("node-key.pending.pem");
    if let Some(summary) = completed_relay_retry(
        &code,
        &enrollment_dir,
        &completed_marker,
        &pending_path,
        &pending_key_path,
    )? {
        return Ok(summary);
    }
    if completed_marker.exists() {
        return Err("relay_identity_exists".to_owned());
    }
    let (private_key_pem, csr_pem, key_fingerprint) = load_or_create_pending_key(
        &pending_key_path,
        false,
        false,
        &enrollment_dir.join("node-key.pem"),
        &code.enrollment_id,
        None,
    )?;
    let request_digest = claim_request_digest(
        code.purpose,
        "enroll",
        &code.enrollment_id,
        &code.configuration_digest,
        &key_fingerprint,
        &csr_pem,
        None,
    );
    let pending = PendingIdentity {
        version: 1,
        purpose: code.purpose.as_str().to_owned(),
        action: "enroll".to_owned(),
        enrollment_id: code.enrollment_id.clone(),
        configuration_digest: code.configuration_digest.clone(),
        request_digest: request_digest.clone(),
        key_fingerprint: key_fingerprint.clone(),
        csr_digest: digest(csr_pem.as_bytes()),
        instance_id: None,
    };
    validate_or_write_pending(&pending_path, &pending)?;
    let claim = ClaimRequest {
        version: 1,
        purpose: code.purpose.as_str(),
        action: "enroll",
        enrollment_id: &code.enrollment_id,
        configuration_digest: &code.configuration_digest,
        secret: &code.secret,
        request_digest: &request_digest,
        key_fingerprint: &key_fingerprint,
        csr_pem: &csr_pem,
        instance_id: None,
    };
    let response = claim_broker(&code, &claim, options.broker_ca_file.as_deref()).await?;
    let bundle: RelayBundle = validate_claim_response(&code, &claim, response)?;
    validate_relay_bundle(&bundle, &key_fingerprint, &code.configuration_digest)?;
    install_relay_identity(
        &enrollment_dir,
        &code,
        &bundle,
        &key_fingerprint,
        &private_key_pem,
    )?;
    remove_pending(&pending_path, &pending_key_path)?;
    Ok(PairingSummary {
        purpose: Purpose::Relay.as_str(),
        enrollment_state: "enrolled",
        identity_fingerprint: key_fingerprint,
        output: enrollment_dir.display().to_string(),
        restart_required: false,
    })
}

fn completed_control_retry(
    code: &PairingCode,
    options: &ControlPairOptions,
    marker_path: &Path,
    pending_path: &Path,
    pending_key_path: &Path,
) -> Result<Option<PairingSummary>, String> {
    if !marker_path.exists() {
        return Ok(None);
    }
    let marker: InstalledControlIdentity = serde_json::from_slice(
        &fs::read(marker_path).map_err(|_| "control_identity_marker_unreadable".to_owned())?,
    )
    .map_err(|_| "control_identity_marker_invalid".to_owned())?;
    if marker.version != PROTOCOL_VERSION
        || marker.purpose != Purpose::ControlAgent.as_str()
        || marker.generation == 0
        || uuid::Uuid::parse_str(&marker.instance_id).is_err()
        || validate_digest(
            &marker.configuration_digest,
            "control_identity_marker_invalid",
        )
        .is_err()
        || validate_digest(&marker.key_fingerprint, "control_identity_marker_invalid").is_err()
    {
        return Err("control_identity_marker_invalid".to_owned());
    }
    validate_installed_key_and_certificate(
        &options.identity_dir.join("server-key.pem"),
        &options.identity_dir.join("server-cert.pem"),
        &marker.key_fingerprint,
    )?;
    if marker.enrollment_id != code.enrollment_id
        || marker.configuration_digest != code.configuration_digest
    {
        return Ok(None);
    }
    cleanup_completed_pending(
        pending_path,
        pending_key_path,
        Purpose::ControlAgent,
        &marker.enrollment_id,
        &marker.configuration_digest,
        &marker.key_fingerprint,
    )?;
    Ok(Some(PairingSummary {
        purpose: Purpose::ControlAgent.as_str(),
        enrollment_state: "paired",
        identity_fingerprint: marker.key_fingerprint,
        output: options.output.display().to_string(),
        restart_required: options.mode != ControlPairMode::Pair,
    }))
}

fn completed_relay_retry(
    code: &PairingCode,
    enrollment_dir: &Path,
    marker_path: &Path,
    pending_path: &Path,
    pending_key_path: &Path,
) -> Result<Option<PairingSummary>, String> {
    if !marker_path.exists() {
        return Ok(None);
    }
    let marker: InstalledRelayIdentity = serde_json::from_slice(
        &fs::read(marker_path).map_err(|_| "relay_identity_marker_unreadable".to_owned())?,
    )
    .map_err(|_| "relay_identity_marker_invalid".to_owned())?;
    if marker.version != PROTOCOL_VERSION
        || marker.purpose != Purpose::Relay.as_str()
        || marker.generation == 0
        || marker.node_id.is_empty()
        || marker.node_id.len() > 128
        || marker.relay_server.is_empty()
        || marker.relay_server.len() > 256
        || marker.host_binding.len() != 71
        || validate_digest(&marker.host_binding, "relay_identity_marker_invalid").is_err()
        || validate_digest(
            &marker.configuration_digest,
            "relay_identity_marker_invalid",
        )
        .is_err()
        || validate_digest(&marker.key_fingerprint, "relay_identity_marker_invalid").is_err()
    {
        return Err("relay_identity_marker_invalid".to_owned());
    }
    validate_installed_key_and_certificate(
        &enrollment_dir.join("node-key.pem"),
        &enrollment_dir.join("node-cert.pem"),
        &marker.key_fingerprint,
    )?;
    if marker.enrollment_id != code.enrollment_id
        || marker.configuration_digest != code.configuration_digest
    {
        return Ok(None);
    }
    cleanup_completed_pending(
        pending_path,
        pending_key_path,
        Purpose::Relay,
        &marker.enrollment_id,
        &marker.configuration_digest,
        &marker.key_fingerprint,
    )?;
    Ok(Some(PairingSummary {
        purpose: Purpose::Relay.as_str(),
        enrollment_state: "enrolled",
        identity_fingerprint: marker.key_fingerprint,
        output: enrollment_dir.display().to_string(),
        restart_required: false,
    }))
}

fn validate_installed_key_and_certificate(
    key_path: &Path,
    certificate_path: &Path,
    expected_fingerprint: &str,
) -> Result<(), String> {
    validate_private_input(key_path)?;
    let private_key =
        fs::read_to_string(key_path).map_err(|_| "installed_identity_key_unreadable".to_owned())?;
    let key =
        KeyPair::from_pem(&private_key).map_err(|_| "installed_identity_key_invalid".to_owned())?;
    if digest(&key.public_key_der()) != expected_fingerprint {
        return Err("installed_identity_key_mismatch".to_owned());
    }
    let certificate = fs::read_to_string(certificate_path)
        .map_err(|_| "installed_identity_certificate_unreadable".to_owned())?;
    validate_certificate_for_key(&certificate, expected_fingerprint)
}

fn cleanup_completed_pending(
    metadata_path: &Path,
    key_path: &Path,
    purpose: Purpose,
    enrollment_id: &str,
    configuration_digest: &str,
    key_fingerprint: &str,
) -> Result<(), String> {
    if metadata_path.exists() {
        let pending: PendingIdentity = serde_json::from_slice(
            &fs::read(metadata_path).map_err(|_| "pairing_pending_unreadable".to_owned())?,
        )
        .map_err(|_| "pairing_pending_invalid".to_owned())?;
        if pending.version != PROTOCOL_VERSION
            || pending.purpose != purpose.as_str()
            || pending.enrollment_id != enrollment_id
            || pending.configuration_digest != configuration_digest
            || pending.key_fingerprint != key_fingerprint
        {
            return Err("pairing_completed_pending_mismatch".to_owned());
        }
    }
    if key_path.exists() {
        validate_private_input(key_path)?;
        let private_key = fs::read_to_string(key_path)
            .map_err(|_| "pairing_pending_key_unreadable".to_owned())?;
        let key = KeyPair::from_pem(&private_key).map_err(|_| "pairing_pending_key_invalid")?;
        if digest(&key.public_key_der()) != key_fingerprint {
            return Err("pairing_completed_pending_key_mismatch".to_owned());
        }
    }
    remove_pending(metadata_path, key_path)
}

fn load_or_create_pending_key(
    pending_key_path: &Path,
    reuse_existing: bool,
    allow_existing_identity: bool,
    existing_key_path: &Path,
    common_name: &str,
    tls_server_name: Option<&str>,
) -> Result<(String, String, String), String> {
    let private_key_pem = if pending_key_path.exists() {
        validate_private_input(pending_key_path)?;
        fs::read_to_string(pending_key_path)
            .map_err(|_| "pairing_pending_key_unreadable".to_owned())?
    } else if reuse_existing && existing_key_path.exists() {
        validate_private_input(existing_key_path)?;
        let value = fs::read_to_string(existing_key_path)
            .map_err(|_| "existing_identity_key_unreadable".to_owned())?;
        atomic_write(pending_key_path, value.as_bytes(), 0o600, false)?;
        value
    } else {
        if existing_key_path.exists() && !allow_existing_identity {
            return Err("identity_key_exists".to_owned());
        }
        let key = KeyPair::generate(&rcgen::PKCS_ED25519)
            .map_err(|_| "identity_key_generation_failed".to_owned())?;
        let value = key.serialize_pem();
        atomic_write(pending_key_path, value.as_bytes(), 0o600, false)?;
        value
    };
    let key_pair =
        KeyPair::from_pem(&private_key_pem).map_err(|_| "identity_key_invalid".to_owned())?;
    let key_fingerprint = digest(&key_pair.public_key_der());
    let subject_alt_names: Vec<String> = tls_server_name
        .map(validate_tls_server_name)
        .transpose()?
        .into_iter()
        .collect();
    let mut params = CertificateParams::new(subject_alt_names);
    params.alg = &rcgen::PKCS_ED25519;
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, bounded_name(common_name)?);
    params.distinguished_name = name;
    params.key_pair = Some(key_pair);
    let certificate = Certificate::from_params(params)
        .map_err(|_| "identity_csr_generation_failed".to_owned())?;
    let csr_pem = certificate
        .serialize_request_pem()
        .map_err(|_| "identity_csr_generation_failed".to_owned())?
        .trim()
        .to_owned();
    Ok((private_key_pem, csr_pem, key_fingerprint))
}

async fn claim_broker(
    code: &PairingCode,
    request: &ClaimRequest<'_>,
    broker_ca_file: Option<&Path>,
) -> Result<ClaimResponse, String> {
    if code.expires_at_unix <= unix_seconds() {
        return Err("pairing_code_expired".to_owned());
    }
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .tls_info(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(CLAIM_TIMEOUT_SECONDS));
    if let Some(path) = broker_ca_file {
        validate_private_input(path)?;
        let bytes = fs::read(path).map_err(|_| "broker_ca_unreadable".to_owned())?;
        let certificate =
            reqwest::Certificate::from_pem(&bytes).map_err(|_| "broker_ca_invalid".to_owned())?;
        builder = builder.add_root_certificate(certificate);
    }
    let client = builder
        .build()
        .map_err(|_| "pairing_broker_client_failed".to_owned())?;
    let preflight_url = format!(
        "{}/.well-known/starry-pairing-v1",
        code.broker_origin.trim_end_matches('/')
    );
    let preflight = timeout(
        Duration::from_secs(CLAIM_TIMEOUT_SECONDS),
        client.get(preflight_url).send(),
    )
    .await
    .map_err(|_| "pairing_broker_timeout".to_owned())?
    .map_err(|_| "pairing_broker_unreachable".to_owned())?;
    verify_response_pin(&preflight, &code.broker_spki_sha256)?;
    if !preflight.status().is_success() {
        return Err("pairing_broker_preflight_rejected".to_owned());
    }
    let claim_url = format!(
        "{}/api/internal/v1/starry/pairing/claim",
        code.broker_origin.trim_end_matches('/')
    );
    let mut response = timeout(
        Duration::from_secs(CLAIM_TIMEOUT_SECONDS),
        client.post(claim_url).json(request).send(),
    )
    .await
    .map_err(|_| "pairing_broker_timeout".to_owned())?
    .map_err(|_| "pairing_broker_unreachable".to_owned())?;
    verify_response_pin(&response, &code.broker_spki_sha256)?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            404 | 410 => "pairing_enrollment_expired_or_unknown",
            409 => "pairing_enrollment_replayed_or_key_changed",
            429 => "pairing_broker_rate_limited",
            _ => "pairing_broker_claim_rejected",
        }
        .to_owned());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_CLAIM_RESPONSE_BYTES as u64)
    {
        return Err("pairing_response_too_large".to_owned());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "pairing_response_unreadable".to_owned())?
    {
        if body.len().saturating_add(chunk.len()) > MAX_CLAIM_RESPONSE_BYTES {
            body.fill(0);
            return Err("pairing_response_too_large".to_owned());
        }
        body.extend_from_slice(&chunk);
    }
    let decoded = serde_json::from_slice(&body).map_err(|_| "pairing_response_invalid".to_owned());
    body.fill(0);
    decoded
}

fn verify_response_pin(response: &reqwest::Response, expected: &str) -> Result<(), String> {
    let certificate = response
        .extensions()
        .get::<reqwest::tls::TlsInfo>()
        .and_then(reqwest::tls::TlsInfo::peer_certificate)
        .ok_or_else(|| "pairing_broker_certificate_missing".to_owned())?;
    let (_, certificate) = parse_x509_certificate(certificate)
        .map_err(|_| "pairing_broker_certificate_invalid".to_owned())?;
    let actual = digest(certificate.tbs_certificate.subject_pki.raw);
    if !constant_time_equal(actual.as_bytes(), expected.as_bytes()) {
        return Err("pairing_broker_pin_mismatch".to_owned());
    }
    Ok(())
}

fn validate_claim_response<T: serde::de::DeserializeOwned>(
    code: &PairingCode,
    request: &ClaimRequest<'_>,
    response: ClaimResponse,
) -> Result<T, String> {
    if response.version != 1
        || response.purpose != request.purpose
        || response.enrollment_id != request.enrollment_id
        || response.configuration_digest != request.configuration_digest
        || response.request_digest != request.request_digest
        || response.key_fingerprint != request.key_fingerprint
        || response.configuration_digest != code.configuration_digest
    {
        return Err("pairing_response_binding_mismatch".to_owned());
    }
    serde_json::from_value(response.bundle).map_err(|_| "pairing_bundle_invalid".to_owned())
}

fn validate_control_bundle(
    bundle: &ControlAgentBundle,
    instance_id: &str,
    key_fingerprint: &str,
    configuration_digest: &str,
) -> Result<(), String> {
    if bundle.instance_id != instance_id
        || bundle.allowed_client_uri_sans.is_empty()
        || bundle.allowed_client_uri_sans.len() > 32
        || bundle
            .allowed_client_uri_sans
            .iter()
            .any(|value| value.len() > 256 || !value.starts_with("spiffe://"))
        || bundle.service_jwt_audience_prefix.is_empty()
        || bundle.service_jwt_audience_prefix.len() > 128
    {
        return Err("control_pairing_bundle_invalid".to_owned());
    }
    validate_origin(&bundle.agent_origin)?;
    validate_origin(&bundle.service_jwt_issuer)?;
    validate_certificate_for_key(&bundle.server_certificate_pem, key_fingerprint)?;
    validate_certificate_pem(&bundle.client_ca_pem)?;
    validate_public_jwks(&bundle.service_jwks)?;
    let approved = serde_json::json!({
        "agent_origin": bundle.agent_origin,
        "allowed_client_uri_sans": bundle.allowed_client_uri_sans,
        "center_public_key": bundle.center_public_key,
        "service_jwt_audience_prefix": bundle.service_jwt_audience_prefix,
        "service_jwt_issuer": bundle.service_jwt_issuer,
    });
    if base64::decode(&bundle.center_public_key)
        .map(|key| key.len() != 32)
        .unwrap_or(true)
    {
        return Err("control_pairing_center_key_invalid".to_owned());
    }
    if digest(&canonical_json(&approved)) != configuration_digest {
        return Err("pairing_configuration_drift".to_owned());
    }
    Ok(())
}

fn validate_relay_bundle(
    bundle: &RelayBundle,
    key_fingerprint: &str,
    configuration_digest: &str,
) -> Result<(), String> {
    if bundle.node_id.is_empty()
        || bundle.node_id.len() > 128
        || bundle.relay_server.is_empty()
        || bundle.relay_server.len() > 256
        || bundle.relay_server != bundle.public_endpoint
        || bundle.max_sessions == 0
        || bundle.max_sessions > 1_000_000
        || bundle.capacity_bandwidth_bps == 0
        || bundle.relay_pool.is_empty()
        || bundle.relay_pool.len() > 128
        || !matches!(
            bundle.profile.as_str(),
            "native" | "native-wss" | "native-wss-fastmedia"
        )
        || (bundle.profile == "native" && bundle.wss_endpoint.is_some())
        || (bundle.profile != "native" && bundle.wss_endpoint.is_none())
        || bundle
            .wss_endpoint
            .as_deref()
            .is_some_and(|endpoint| !valid_telemetry_endpoint(endpoint))
        || (bundle.profile == "native-wss-fastmedia") != bundle.fast_media_udp_port.is_some()
        || bundle.fast_media_udp_port == Some(0)
        || decode_config(&bundle.telemetry_secret, URL_SAFE_NO_PAD)
            .map(|secret| secret.len() != 32)
            .unwrap_or(true)
        || base64::decode(&bundle.center_public_key)
            .map(|key| key.len() != 32)
            .unwrap_or(true)
        || !safe_env_value(&bundle.relay_server)
    {
        return Err("relay_pairing_bundle_invalid".to_owned());
    }
    validate_certificate_for_key(&bundle.node_certificate_pem, key_fingerprint)?;
    validate_certificate_pem(&bundle.relay_ca_pem)?;
    validate_certificate_chain(&bundle.node_certificate_pem, &bundle.relay_ca_pem)?;
    if relay_configuration_digest(
        &bundle.node_id,
        &bundle.relay_server,
        &bundle.public_endpoint,
        &bundle.relay_pool,
        &bundle.profile,
        bundle.wss_endpoint.as_deref(),
        bundle.activate_after_health,
        bundle.max_sessions,
        bundle.capacity_bandwidth_bps,
        bundle.draining,
        bundle.fast_media_udp_port,
    ) != configuration_digest
    {
        return Err("pairing_configuration_drift".to_owned());
    }
    Ok(())
}

fn install_control_identity(
    options: &ControlPairOptions,
    code: &PairingCode,
    bundle: &ControlAgentBundle,
    instance_id: &str,
    key_fingerprint: &str,
    private_key_pem: &str,
) -> Result<(), String> {
    let marker_path = options.state_dir.join("identity.json");
    let completed = marker_path.exists();
    if completed && options.mode == ControlPairMode::Pair {
        return Err("control_identity_exists".to_owned());
    }
    let previous = if completed {
        Some(load_existing_generated_config(options)?)
    } else {
        None
    };
    atomic_write(
        &options.state_dir.join("instance-id"),
        format!("{instance_id}\n").as_bytes(),
        0o600,
        completed,
    )?;
    let server_key = options.identity_dir.join("server-key.pem");
    let server_cert = options.identity_dir.join("server-cert.pem");
    let client_ca = options.identity_dir.join("client-ca.pem");
    let jwks = options.identity_dir.join("service-jwks.json");
    atomic_write(&server_key, private_key_pem.as_bytes(), 0o600, completed)?;
    atomic_write(
        &server_cert,
        bundle.server_certificate_pem.as_bytes(),
        0o640,
        completed,
    )?;
    atomic_write(
        &client_ca,
        bundle.client_ca_pem.as_bytes(),
        0o640,
        completed,
    )?;
    atomic_json(&jwks, &bundle.service_jwks, 0o640, completed)?;
    atomic_write(
        &options.shared_dir.join("center-public-key"),
        format!("{}\n", bundle.center_public_key).as_bytes(),
        0o640,
        completed,
    )?;
    ensure_relay_ca(&options.identity_dir)?;

    let token_file = options.shared_dir.join("local-control.token");
    if !token_file.exists() {
        let token = encode_config(sodiumoxide::randombytes::randombytes(32), URL_SAFE_NO_PAD);
        atomic_write(&token_file, format!("{token}\n").as_bytes(), 0o600, false)?;
    }
    let generated = GeneratedAgentConfig {
        version: 1,
        instance_id_file: options.state_dir.join("instance-id").display().to_string(),
        listen: previous
            .as_ref()
            .map(|config| config.listen)
            .unwrap_or(options.listen),
        tls: GeneratedTlsConfig {
            ca_file: client_ca.display().to_string(),
            cert_file: server_cert.display().to_string(),
            key_file: server_key.display().to_string(),
            allowed_client_uri_sans: bundle.allowed_client_uri_sans.clone(),
        },
        service_jwt: GeneratedServiceJwtConfig {
            issuer: bundle.service_jwt_issuer.clone(),
            jwks_file: jwks.display().to_string(),
            audience_prefix: bundle.service_jwt_audience_prefix.clone(),
        },
        local_control: GeneratedLocalControlConfig {
            address: previous
                .as_ref()
                .map(|config| config.local_control.address)
                .unwrap_or(options.local_control_address),
            token_file: token_file.display().to_string(),
        },
        config: GeneratedManagedConfig {
            path: options.managed_config_path.display().to_string(),
            backup_dir: options.backup_dir.display().to_string(),
            max_bytes: previous
                .as_ref()
                .map(|config| config.config.max_bytes)
                .unwrap_or(1024 * 1024),
            write_enabled: previous
                .as_ref()
                .map(|config| config.config.write_enabled)
                .unwrap_or(false),
        },
    };
    let yaml =
        serde_yml::to_string(&generated).map_err(|_| "generated_agent_config_failed".to_owned())?;
    atomic_write(&options.output, yaml.as_bytes(), 0o640, completed)?;
    let generation = existing_generation(&marker_path).saturating_add(1).max(1);
    atomic_json(
        &marker_path,
        &ControlIdentityMarker {
            version: 1,
            purpose: "control-agent",
            enrollment_id: &code.enrollment_id,
            configuration_digest: &code.configuration_digest,
            instance_id,
            key_fingerprint,
            generation,
        },
        0o600,
        completed,
    )
}

fn load_existing_generated_config(
    options: &ControlPairOptions,
) -> Result<GeneratedAgentConfig, String> {
    let metadata = fs::symlink_metadata(&options.output)
        .map_err(|_| "control_generated_config_unreadable".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err("control_generated_config_invalid".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 || metadata.mode() & 0o027 != 0 {
            return Err("control_generated_config_invalid".to_owned());
        }
    }
    let raw = fs::read_to_string(&options.output)
        .map_err(|_| "control_generated_config_unreadable".to_owned())?;
    let config: GeneratedAgentConfig =
        serde_yml::from_str(&raw).map_err(|_| "control_generated_config_invalid".to_owned())?;
    let expected_instance = options.state_dir.join("instance-id").display().to_string();
    let expected_ca = options
        .identity_dir
        .join("client-ca.pem")
        .display()
        .to_string();
    let expected_cert = options
        .identity_dir
        .join("server-cert.pem")
        .display()
        .to_string();
    let expected_key = options
        .identity_dir
        .join("server-key.pem")
        .display()
        .to_string();
    let expected_jwks = options
        .identity_dir
        .join("service-jwks.json")
        .display()
        .to_string();
    let expected_token = options
        .shared_dir
        .join("local-control.token")
        .display()
        .to_string();
    if config.version != 1
        || config.instance_id_file != expected_instance
        || config.tls.ca_file != expected_ca
        || config.tls.cert_file != expected_cert
        || config.tls.key_file != expected_key
        || config.service_jwt.jwks_file != expected_jwks
        || config.local_control.token_file != expected_token
        || config.config.path != options.managed_config_path.display().to_string()
        || config.config.backup_dir != options.backup_dir.display().to_string()
        || config.config.max_bytes == 0
        || config.config.max_bytes > 16 * 1024 * 1024
    {
        return Err("control_generated_config_binding_mismatch".to_owned());
    }
    Ok(config)
}

fn install_relay_identity(
    directory: &Path,
    code: &PairingCode,
    bundle: &RelayBundle,
    key_fingerprint: &str,
    private_key_pem: &str,
) -> Result<(), String> {
    let marker_path = directory.join("enrollment.json");
    if marker_path.exists() {
        return Err("relay_identity_exists".to_owned());
    }
    let host_binding = host_binding()?;
    atomic_write(
        &directory.join("node-id"),
        format!("{}\n", bundle.node_id).as_bytes(),
        0o640,
        false,
    )?;
    atomic_write(
        &directory.join("node-key.pem"),
        private_key_pem.as_bytes(),
        0o600,
        false,
    )?;
    atomic_write(
        &directory.join("node-cert.pem"),
        bundle.node_certificate_pem.as_bytes(),
        0o640,
        false,
    )?;
    atomic_write(
        &directory.join("relay-ca.pem"),
        bundle.relay_ca_pem.as_bytes(),
        0o640,
        false,
    )?;
    atomic_write(
        &directory.join("center-public-key"),
        format!("{}\n", bundle.center_public_key).as_bytes(),
        0o640,
        false,
    )?;
    atomic_write(
        &directory.join("telemetry.secret"),
        format!("{}\n", bundle.telemetry_secret).as_bytes(),
        0o600,
        false,
    )?;
    atomic_write(
        &directory.join("host-id"),
        format!("{host_binding}\n").as_bytes(),
        0o600,
        false,
    )?;
    let runtime = RelayRuntimeConfig {
        version: 1,
        node_id: &bundle.node_id,
        relay_server: &bundle.relay_server,
        public_endpoint: &bundle.public_endpoint,
        telemetry_secret_file: directory.join("telemetry.secret").display().to_string(),
        max_sessions: bundle.max_sessions,
        capacity_bandwidth_bps: bundle.capacity_bandwidth_bps,
        draining: bundle.draining,
        relay_pool: &bundle.relay_pool,
        profile: &bundle.profile,
        wss_endpoint: bundle.wss_endpoint.as_deref(),
        activate_after_health: bundle.activate_after_health,
        fast_media_udp_port: bundle.fast_media_udp_port,
    };
    atomic_json(&directory.join("relay-config.json"), &runtime, 0o640, false)?;
    let mut compatibility = format!(
        "KEY={}\nSTARRY_RELAY_TELEMETRY_SECRET_FILE={}\nSTARRY_RELAY_PUBLIC_ENDPOINT={}\nSTARRY_RELAY_MAX_SESSIONS={}\nSTARRY_RELAY_CAPACITY_BANDWIDTH_BPS={}\nSTARRY_RELAY_DRAINING={}\nSTARRY_RELAY_ENROLLMENT_DIR={}\n",
        bundle.center_public_key,
        directory.join("telemetry.secret").display(),
        bundle.public_endpoint,
        bundle.max_sessions,
        bundle.capacity_bandwidth_bps,
        if bundle.draining { 1 } else { 0 },
        directory.display(),
    );
    if let Some(port) = bundle.fast_media_udp_port {
        compatibility.push_str(&format!("STARRY_RELAY_FAST_MEDIA_UDP_PORT={port}\n"));
    }
    atomic_write(
        &directory.join("relay-compat.env"),
        compatibility.as_bytes(),
        0o640,
        false,
    )?;
    atomic_json(
        &marker_path,
        &RelayIdentityMarker {
            version: 1,
            purpose: "relay",
            enrollment_id: &code.enrollment_id,
            configuration_digest: &code.configuration_digest,
            node_id: &bundle.node_id,
            relay_server: &bundle.relay_server,
            key_fingerprint,
            host_binding: &host_binding,
            generation: 1,
        },
        0o600,
        false,
    )?;
    // The original SP1 secret was only ever held by the PairingCode value and
    // is intentionally absent from every installed file. Only the public
    // enrollment/configuration binding is retained for idempotent recovery.
    Ok(())
}

fn ensure_relay_ca(identity_dir: &Path) -> Result<(), String> {
    let cert_path = identity_dir.join("relay-ca.pem");
    let key_path = identity_dir.join("relay-ca-key.pem");
    match (cert_path.exists(), key_path.exists()) {
        (true, true) => {
            validate_private_input(&key_path)?;
            validate_certificate_pem(
                &fs::read_to_string(&cert_path).map_err(|_| "relay_ca_unreadable".to_owned())?,
            )
        }
        (false, false) => {
            let mut params = CertificateParams::new(Vec::<String>::new());
            params.alg = &rcgen::PKCS_ED25519;
            params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
            let mut name = DistinguishedName::new();
            name.push(DnType::CommonName, "Starry Relay Enrollment CA v1");
            params.distinguished_name = name;
            let certificate = Certificate::from_params(params)
                .map_err(|_| "relay_ca_generation_failed".to_owned())?;
            atomic_write(
                &key_path,
                certificate.serialize_private_key_pem().as_bytes(),
                0o600,
                false,
            )?;
            atomic_write(
                &cert_path,
                certificate
                    .serialize_pem()
                    .map_err(|_| "relay_ca_generation_failed".to_owned())?
                    .as_bytes(),
                0o640,
                false,
            )
        }
        (false, true) => {
            // Recover the only valid interrupted creation shape: the private
            // key was durably installed, but the public certificate was not.
            // Reusing that exact key keeps retries idempotent.
            validate_private_input(&key_path)?;
            let private_key =
                fs::read_to_string(&key_path).map_err(|_| "relay_ca_unreadable".to_owned())?;
            let key_pair =
                KeyPair::from_pem(&private_key).map_err(|_| "relay_ca_key_invalid".to_owned())?;
            let mut params = CertificateParams::new(Vec::<String>::new());
            params.alg = &rcgen::PKCS_ED25519;
            params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
            let mut name = DistinguishedName::new();
            name.push(DnType::CommonName, "Starry Relay Enrollment CA v1");
            params.distinguished_name = name;
            params.key_pair = Some(key_pair);
            let certificate = Certificate::from_params(params)
                .map_err(|_| "relay_ca_generation_failed".to_owned())?;
            atomic_write(
                &cert_path,
                certificate
                    .serialize_pem()
                    .map_err(|_| "relay_ca_generation_failed".to_owned())?
                    .as_bytes(),
                0o640,
                false,
            )
        }
        _ => Err("relay_ca_partial_identity".to_owned()),
    }
}

fn validate_or_write_pending(path: &Path, expected: &PendingIdentity) -> Result<(), String> {
    if path.exists() {
        let existing: PendingIdentity = serde_json::from_slice(
            &fs::read(path).map_err(|_| "pairing_pending_unreadable".to_owned())?,
        )
        .map_err(|_| "pairing_pending_invalid".to_owned())?;
        if serde_json::to_value(&existing).ok() != serde_json::to_value(expected).ok() {
            return Err("pairing_pending_identity_changed".to_owned());
        }
        return Ok(());
    }
    atomic_json(path, expected, 0o600, false)
}

fn recover_pending_control_instance(
    path: &Path,
    code: &PairingCode,
) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let pending: PendingIdentity = serde_json::from_slice(
        &fs::read(path).map_err(|_| "pairing_pending_unreadable".to_owned())?,
    )
    .map_err(|_| "pairing_pending_invalid".to_owned())?;
    if pending.version != PROTOCOL_VERSION
        || pending.purpose != Purpose::ControlAgent.as_str()
        || pending.action != ControlPairMode::Pair.action()
        || pending.enrollment_id != code.enrollment_id
        || pending.configuration_digest != code.configuration_digest
    {
        return Err("pairing_pending_identity_changed".to_owned());
    }
    let instance_id = pending
        .instance_id
        .ok_or_else(|| "pairing_pending_invalid".to_owned())?;
    uuid::Uuid::parse_str(&instance_id).map_err(|_| "pairing_pending_invalid".to_owned())?;
    Ok(Some(instance_id))
}

fn remove_pending(metadata: &Path, key: &Path) -> Result<(), String> {
    for path in [metadata, key] {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err("pairing_pending_cleanup_failed".to_owned()),
        }
    }
    Ok(())
}

fn claim_request_digest(
    purpose: Purpose,
    action: &str,
    enrollment_id: &str,
    configuration_digest: &str,
    key_fingerprint: &str,
    csr_pem: &str,
    instance_id: Option<&str>,
) -> String {
    digest(
        format!(
            "starry-pairing-claim-v1\n{}\n{action}\n{enrollment_id}\n{configuration_digest}\n{key_fingerprint}\n{}\n{}",
            purpose.as_str(),
            digest(csr_pem.as_bytes()),
            instance_id.unwrap_or_default(),
        )
        .as_bytes(),
    )
}

pub(crate) fn relay_claim_request_digest(
    enrollment_id: &str,
    configuration_digest: &str,
    key_fingerprint: &str,
    csr_pem: &str,
) -> String {
    claim_request_digest(
        Purpose::Relay,
        "enroll",
        enrollment_id,
        configuration_digest,
        key_fingerprint,
        csr_pem,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn relay_configuration_digest(
    node_id: &str,
    relay_server: &str,
    public_endpoint: &str,
    relay_pool: &str,
    profile: &str,
    wss_endpoint: Option<&str>,
    activate_after_health: bool,
    max_sessions: u32,
    capacity_bandwidth_bps: u64,
    draining: bool,
    fast_media_udp_port: Option<u16>,
) -> String {
    let approved = serde_json::json!({
        "activate_after_health": activate_after_health,
        "capacity_bandwidth_bps": capacity_bandwidth_bps,
        "draining": draining,
        "fast_media_udp_port": fast_media_udp_port,
        "max_sessions": max_sessions,
        "node_id": node_id,
        "profile": profile,
        "public_endpoint": public_endpoint,
        "relay_pool": relay_pool,
        "relay_server": relay_server,
        "wss_endpoint": wss_endpoint,
    });
    digest(&canonical_json(&approved))
}

fn canonical_json(value: &Value) -> Vec<u8> {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                let mut output = Map::new();
                for key in keys {
                    output.insert(key.clone(), sorted(&object[key]));
                }
                Value::Object(output)
            }
            Value::Array(values) => Value::Array(values.iter().map(sorted).collect()),
            _ => value.clone(),
        }
    }
    serde_json::to_vec(&sorted(value)).unwrap_or_default()
}

fn validate_control_paths(options: &ControlPairOptions) -> Result<(), String> {
    for path in [
        &options.state_dir,
        &options.identity_dir,
        &options.output,
        &options.shared_dir,
        &options.managed_config_path,
        &options.backup_dir,
    ] {
        if !path.is_absolute() {
            return Err("control_pairing_path_not_absolute".to_owned());
        }
    }
    if options.state_dir == options.identity_dir
        || options.output.starts_with(&options.state_dir)
        || options.output.starts_with(&options.shared_dir)
    {
        return Err("control_pairing_path_layout_invalid".to_owned());
    }
    if let Some(name) = options.tls_server_name.as_deref() {
        validate_tls_server_name(name)?;
    }
    Ok(())
}

fn validate_tls_server_name(value: &str) -> Result<String, String> {
    let valid = !value.is_empty()
        && value.len() <= 253
        && value.is_ascii()
        && value.parse::<std::net::IpAddr>().is_err()
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    if !valid {
        return Err("control_tls_server_name_invalid".to_owned());
    }
    Ok(value.to_owned())
}

/// Rejects container writable layers and tmpfs for durable identity/config
/// state. Native filesystems and explicit bind/volume mounts remain valid.
#[cfg(target_os = "linux")]
pub fn validate_persistent_directory(path: &Path) -> Result<(), String> {
    let canonical =
        fs::canonicalize(path).map_err(|_| "persistent_state_directory_missing".to_owned())?;
    let mountinfo = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|_| "persistent_state_mountinfo_unavailable".to_owned())?;
    match persistent_filesystem(&canonical, &mountinfo).as_deref() {
        Some("overlay" | "tmpfs") => Err("persistent_state_not_durable".to_owned()),
        Some(_) => Ok(()),
        None => Err("persistent_state_mount_unknown".to_owned()),
    }
}

#[cfg(target_os = "linux")]
fn persistent_filesystem(path: &Path, mountinfo: &str) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for line in mountinfo.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let mountpoint = left
            .split_whitespace()
            .nth(4)
            .unwrap_or_default()
            .replace("\\040", " ");
        let filesystem = right.split_whitespace().next().unwrap_or_default();
        if path.starts_with(Path::new(&mountpoint))
            && best
                .as_ref()
                .is_none_or(|(length, _)| mountpoint.len() > *length)
        {
            best = Some((mountpoint.len(), filesystem.to_owned()));
        }
    }
    best.map(|(_, filesystem)| filesystem)
}

#[cfg(not(target_os = "linux"))]
pub fn validate_persistent_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn validate_origin(value: &str) -> Result<(), String> {
    let parsed = url::Url::parse(value).map_err(|_| "pairing_origin_invalid".to_owned())?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.path(), "" | "/")
    {
        return Err("pairing_origin_invalid".to_owned());
    }
    Ok(())
}

fn valid_telemetry_endpoint(value: &str) -> bool {
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

fn validate_digest(value: &str, code: &str) -> Result<(), String> {
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(code.to_owned());
    }
    Ok(())
}

fn validate_certificate_for_key(pem: &str, expected: &str) -> Result<(), String> {
    let (_, pem) =
        parse_x509_pem(pem.as_bytes()).map_err(|_| "pairing_certificate_invalid".to_owned())?;
    let (_, certificate) = parse_x509_certificate(&pem.contents)
        .map_err(|_| "pairing_certificate_invalid".to_owned())?;
    if digest(certificate.tbs_certificate.subject_pki.raw) != expected {
        return Err("pairing_certificate_key_mismatch".to_owned());
    }
    let now = unix_seconds() as i64;
    if certificate.validity().not_before.timestamp() > now
        || certificate.validity().not_after.timestamp() <= now
    {
        return Err("pairing_certificate_expired".to_owned());
    }
    Ok(())
}

fn validate_certificate_pem(pem: &str) -> Result<(), String> {
    let (_, pem) =
        parse_x509_pem(pem.as_bytes()).map_err(|_| "pairing_certificate_invalid".to_owned())?;
    let (_, certificate) = parse_x509_certificate(&pem.contents)
        .map_err(|_| "pairing_certificate_invalid".to_owned())?;
    let now = unix_seconds() as i64;
    if certificate.validity().not_before.timestamp() > now
        || certificate.validity().not_after.timestamp() <= now
    {
        return Err("pairing_certificate_expired".to_owned());
    }
    Ok(())
}

fn validate_certificate_chain(leaf_pem: &str, ca_pem: &str) -> Result<(), String> {
    let (_, leaf_pem) = parse_x509_pem(leaf_pem.as_bytes())
        .map_err(|_| "pairing_certificate_invalid".to_owned())?;
    let (_, leaf) = parse_x509_certificate(&leaf_pem.contents)
        .map_err(|_| "pairing_certificate_invalid".to_owned())?;
    let (_, ca_pem) =
        parse_x509_pem(ca_pem.as_bytes()).map_err(|_| "pairing_certificate_invalid".to_owned())?;
    let (_, ca) = parse_x509_certificate(&ca_pem.contents)
        .map_err(|_| "pairing_certificate_invalid".to_owned())?;
    if leaf.issuer() != ca.subject()
        || ca.verify_signature(None).is_err()
        || leaf.verify_signature(Some(ca.public_key())).is_err()
    {
        return Err("pairing_certificate_chain_invalid".to_owned());
    }
    Ok(())
}

fn validate_public_jwks(value: &Value) -> Result<(), String> {
    let keys = value
        .get("keys")
        .and_then(Value::as_array)
        .ok_or_else(|| "pairing_jwks_invalid".to_owned())?;
    if keys.is_empty() || keys.len() > 32 {
        return Err("pairing_jwks_invalid".to_owned());
    }
    for key in keys {
        let object = key
            .as_object()
            .ok_or_else(|| "pairing_jwks_invalid".to_owned())?;
        if object.contains_key("d")
            || object.get("kty").and_then(Value::as_str) != Some("OKP")
            || object.get("crv").and_then(Value::as_str) != Some("Ed25519")
            || object
                .get("x")
                .and_then(Value::as_str)
                .and_then(|x| decode_config(x, URL_SAFE_NO_PAD).ok())
                .is_none_or(|x| x.len() != 32)
        {
            return Err("pairing_jwks_invalid".to_owned());
        }
    }
    Ok(())
}

fn validate_private_input(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "private_input_missing".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err("private_input_unsafe".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
            return Err("private_input_permissions".to_owned());
        }
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|_| "state_directory_create_failed".to_owned())?;
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "state_directory_unavailable".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("state_directory_unsafe".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "state_directory_permissions_failed".to_owned())?;
    }
    Ok(())
}

fn atomic_json<T: serde::Serialize>(
    path: &Path,
    value: &T,
    mode: u32,
    replace: bool,
) -> Result<(), String> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|_| "state_serialization_failed".to_owned())?;
    bytes.push(b'\n');
    atomic_write(path, &bytes, mode, replace)
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32, replace: bool) -> Result<(), String> {
    if !replace && existing_file_matches(path, bytes, mode)? {
        return Ok(());
    }
    if path.exists() && !replace {
        return Err("state_file_exists".to_owned());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "state_path_invalid".to_owned())?;
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
        .map_err(|_| "state_temporary_create_failed".to_owned())?;
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        let _ = fs::remove_file(&temporary);
        return Err("state_write_failed".to_owned());
    }
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))
            .map_err(|_| "state_permissions_failed".to_owned())?;
    }
    if replace {
        fs::rename(&temporary, path).map_err(|_| "state_atomic_replace_failed".to_owned())?;
    } else {
        match fs::hard_link(&temporary, path) {
            Ok(()) => {
                fs::remove_file(&temporary)
                    .map_err(|_| "state_temporary_cleanup_failed".to_owned())?;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&temporary);
                if !existing_file_matches(path, bytes, mode)? {
                    return Err("state_file_exists".to_owned());
                }
            }
            Err(_) => {
                let _ = fs::remove_file(&temporary);
                return Err("state_atomic_install_failed".to_owned());
            }
        }
    }
    let directory = OpenOptions::new()
        .read(true)
        .open(parent)
        .map_err(|_| "state_directory_sync_failed".to_owned())?;
    directory
        .sync_all()
        .map_err(|_| "state_directory_sync_failed".to_owned())
}

fn existing_file_matches(path: &Path, bytes: &[u8], mode: u32) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("state_file_unreadable".to_owned()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("state_file_unsafe".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 || metadata.mode() & 0o777 != mode {
            return Err("state_file_permissions".to_owned());
        }
    }
    if metadata.len() != bytes.len() as u64 {
        return Ok(false);
    }
    let existing = fs::read(path).map_err(|_| "state_file_unreadable".to_owned())?;
    Ok(constant_time_equal(&existing, bytes))
}

fn existing_generation(path: &Path) -> u64 {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.get("generation").and_then(Value::as_u64))
        .unwrap_or_default()
}

fn read_optional_trimmed(path: &Path, maximum: usize) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(value) => {
            let value = value.trim().to_owned();
            if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
                return Err("state_value_invalid".to_owned());
            }
            Ok(Some(value))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("state_value_unreadable".to_owned()),
    }
}

fn host_binding() -> Result<String, String> {
    let source = std::env::var("STARRY_RELAY_HOST_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| machine_uid::get().ok())
        .ok_or_else(|| "relay_host_identity_unavailable".to_owned())?;
    if source.len() > 512 || source.chars().any(char::is_control) {
        return Err("relay_host_identity_invalid".to_owned());
    }
    Ok(digest(source.as_bytes()))
}

fn bounded_name(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err("identity_name_invalid".to_owned());
    }
    Ok(value.to_owned())
}

fn safe_env_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':' | b'[' | b']')
        })
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
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

#[cfg(unix)]
fn atty_stdin() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

#[cfg(not(unix))]
fn atty_stdin() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn code(purpose: &str, expires_at_unix: u64) -> String {
        PairingCode::encode(PairingCodePayload {
            version: 1,
            purpose: purpose.to_owned(),
            broker_origin: "https://broker.example:8443".to_owned(),
            broker_spki_sha256: format!("sha256:{}", "1".repeat(64)),
            enrollment_id: uuid::Uuid::now_v7().to_string(),
            configuration_digest: format!("sha256:{}", "2".repeat(64)),
            expires_at_unix,
            secret: encode_config([7_u8; 32], URL_SAFE_NO_PAD),
        })
    }

    fn test_identity() -> (String, String, String) {
        let mut params = CertificateParams::new(Vec::<String>::new());
        params.alg = &rcgen::PKCS_ED25519;
        let certificate = Certificate::from_params(params).unwrap();
        let private_key = certificate.serialize_private_key_pem();
        let key = KeyPair::from_pem(&private_key).unwrap();
        let fingerprint = digest(&key.public_key_der());
        let certificate_pem = certificate.serialize_pem().unwrap();
        (private_key, certificate_pem, fingerprint)
    }

    #[test]
    fn sp1_is_bounded_expiring_and_purpose_bound() {
        let relay = code("relay", unix_seconds() + 600);
        assert!(PairingCode::parse(&relay, Purpose::Relay).is_ok());
        assert_eq!(
            PairingCode::parse(&relay, Purpose::ControlAgent).unwrap_err(),
            "pairing_code_purpose_mismatch"
        );
        let expired = code("relay", unix_seconds());
        assert_eq!(
            PairingCode::parse(&expired, Purpose::Relay).unwrap_err(),
            "pairing_code_expired_or_unbounded"
        );
    }

    #[test]
    fn request_digest_binds_csr_purpose_and_configuration() {
        let first = claim_request_digest(
            Purpose::Relay,
            "enroll",
            "id",
            "sha256:a",
            "sha256:b",
            "csr-a",
            None,
        );
        let changed = claim_request_digest(
            Purpose::ControlAgent,
            "enroll",
            "id",
            "sha256:a",
            "sha256:b",
            "csr-a",
            None,
        );
        let changed_csr = claim_request_digest(
            Purpose::Relay,
            "enroll",
            "id",
            "sha256:a",
            "sha256:b",
            "csr-b",
            None,
        );
        assert_ne!(first, changed);
        assert_ne!(first, changed_csr);
    }

    #[test]
    fn canonical_bundle_digest_is_key_order_independent() {
        let left = json!({"z": 1, "a": {"y": 2, "x": 3}});
        let right = json!({"a": {"x": 3, "y": 2}, "z": 1});
        assert_eq!(
            digest(&canonical_json(&left)),
            digest(&canonical_json(&right))
        );
    }

    #[test]
    fn relay_approved_configuration_excludes_generated_secret_material() {
        let public = serde_json::json!({
            "activate_after_health": true,
            "capacity_bandwidth_bps": 1_000_000_u64,
            "draining": false,
            "fast_media_udp_port": 22119,
            "max_sessions": 100_u32,
            "node_id": "relay-sg",
            "profile": "native-wss-fastmedia",
            "public_endpoint": "relay.example:21117",
            "relay_pool": "primary",
            "relay_server": "relay.example:21117",
            "wss_endpoint": "wss://relay.example:21119/ws/telemetry",
        });
        let expected = digest(&canonical_json(&public));
        let bundle = RelayBundle {
            node_id: "relay-sg".to_owned(),
            relay_server: "relay.example:21117".to_owned(),
            public_endpoint: "relay.example:21117".to_owned(),
            node_certificate_pem: String::new(),
            relay_ca_pem: String::new(),
            center_public_key: base64::encode([1_u8; 32]),
            telemetry_secret: encode_config([2_u8; 32], URL_SAFE_NO_PAD),
            max_sessions: 100,
            capacity_bandwidth_bps: 1_000_000,
            draining: false,
            relay_pool: "primary".to_owned(),
            profile: "native-wss-fastmedia".to_owned(),
            wss_endpoint: Some("wss://relay.example:21119/ws/telemetry".to_owned()),
            activate_after_health: true,
            fast_media_udp_port: Some(22119),
        };
        assert_eq!(
            relay_configuration_digest(
                &bundle.node_id,
                &bundle.relay_server,
                &bundle.public_endpoint,
                &bundle.relay_pool,
                &bundle.profile,
                bundle.wss_endpoint.as_deref(),
                bundle.activate_after_health,
                bundle.max_sessions,
                bundle.capacity_bandwidth_bps,
                bundle.draining,
                bundle.fast_media_udp_port,
            ),
            expected
        );
    }

    #[test]
    fn public_jwks_rejects_private_material() {
        let public = json!({"keys": [{
            "kty": "OKP",
            "crv": "Ed25519",
            "x": encode_config([9_u8; 32], URL_SAFE_NO_PAD)
        }]});
        assert!(validate_public_jwks(&public).is_ok());
        let mut private = public;
        private["keys"][0]["d"] = Value::String("secret".to_owned());
        assert!(validate_public_jwks(&private).is_err());
    }

    #[test]
    fn interrupted_install_accepts_only_an_identical_durable_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.pem");
        atomic_write(&path, b"first\n", 0o600, false).unwrap();
        atomic_write(&path, b"first\n", 0o600, false).unwrap();
        assert_eq!(
            atomic_write(&path, b"different\n", 0o600, false).unwrap_err(),
            "state_file_exists"
        );
        assert_eq!(fs::read(&path).unwrap(), b"first\n");
    }

    #[test]
    fn interrupted_control_pair_recovers_the_pending_instance_id() {
        let temporary = tempfile::tempdir().unwrap();
        let pending_path = temporary.path().join("pairing.pending.json");
        let raw_code = code("control-agent", unix_seconds() + 600);
        let parsed_code = PairingCode::parse(&raw_code, Purpose::ControlAgent).unwrap();
        let instance_id = uuid::Uuid::now_v7().to_string();
        atomic_json(
            &pending_path,
            &PendingIdentity {
                version: PROTOCOL_VERSION,
                purpose: Purpose::ControlAgent.as_str().to_owned(),
                action: ControlPairMode::Pair.action().to_owned(),
                enrollment_id: parsed_code.enrollment_id.clone(),
                configuration_digest: parsed_code.configuration_digest.clone(),
                request_digest: format!("sha256:{}", "3".repeat(64)),
                key_fingerprint: format!("sha256:{}", "4".repeat(64)),
                csr_digest: format!("sha256:{}", "5".repeat(64)),
                instance_id: Some(instance_id.clone()),
            },
            0o600,
            false,
        )
        .unwrap();

        assert_eq!(
            recover_pending_control_instance(&pending_path, &parsed_code).unwrap(),
            Some(instance_id)
        );

        let different = PairingCode::parse(
            &code("control-agent", unix_seconds() + 600),
            Purpose::ControlAgent,
        )
        .unwrap();
        assert_eq!(
            recover_pending_control_instance(&pending_path, &different).unwrap_err(),
            "pairing_pending_identity_changed"
        );
    }

    #[test]
    fn generated_control_csr_is_canonical_and_contains_the_explicit_tls_name() {
        let temporary = tempfile::tempdir().unwrap();
        let (_, csr, _) = load_or_create_pending_key(
            &temporary.path().join("server-key.pending.pem"),
            false,
            false,
            &temporary.path().join("server-key.pem"),
            &uuid::Uuid::now_v7().to_string(),
            Some("starry.internal"),
        )
        .unwrap();
        assert_eq!(csr.trim(), csr);
        let request = rcgen::CertificateSigningRequest::from_pem(&csr).unwrap();
        assert!(request.params.subject_alt_names.iter().any(|name| {
            matches!(name, rcgen::SanType::DnsName(value) if value == "starry.internal")
        }));
        assert_eq!(
            validate_tls_server_name("not/a/host").unwrap_err(),
            "control_tls_server_name_invalid"
        );
        for invalid in [
            "host:21120",
            "127.0.0.1",
            "::1",
            ".starry.internal",
            "starry.internal.",
            "starry..internal",
            "-starry.internal",
            "starry-.internal",
        ] {
            assert_eq!(
                validate_tls_server_name(invalid).unwrap_err(),
                "control_tls_server_name_invalid"
            );
        }
        let oversized_label = format!("{}.internal", "a".repeat(64));
        assert_eq!(
            validate_tls_server_name(&oversized_label).unwrap_err(),
            "control_tls_server_name_invalid"
        );
        assert_eq!(
            validate_tls_server_name("Starry-01.internal").unwrap(),
            "Starry-01.internal"
        );
    }

    #[test]
    fn rotation_preserves_existing_generated_runtime_settings() {
        let temporary = tempfile::tempdir().unwrap();
        let options = ControlPairOptions {
            mode: ControlPairMode::Rotate,
            tls_server_name: Some("starry.internal".to_owned()),
            state_dir: temporary.path().join("control/state"),
            identity_dir: temporary.path().join("control/identity"),
            output: temporary
                .path()
                .join("control/generated/control-agent.yaml"),
            shared_dir: temporary.path().join("control/shared"),
            managed_config_path: temporary.path().join("config/config.yaml"),
            backup_dir: temporary.path().join("config/history"),
            listen: "0.0.0.0:21120".parse().unwrap(),
            local_control_address: "127.0.0.1:21119".parse().unwrap(),
            broker_ca_file: None,
        };
        let existing = GeneratedAgentConfig {
            version: 1,
            instance_id_file: options.state_dir.join("instance-id").display().to_string(),
            listen: "0.0.0.0:24443".parse().unwrap(),
            tls: GeneratedTlsConfig {
                ca_file: options
                    .identity_dir
                    .join("client-ca.pem")
                    .display()
                    .to_string(),
                cert_file: options
                    .identity_dir
                    .join("server-cert.pem")
                    .display()
                    .to_string(),
                key_file: options
                    .identity_dir
                    .join("server-key.pem")
                    .display()
                    .to_string(),
                allowed_client_uri_sans: vec!["spiffe://old-client".to_owned()],
            },
            service_jwt: GeneratedServiceJwtConfig {
                issuer: "https://old-issuer.example".to_owned(),
                jwks_file: options
                    .identity_dir
                    .join("service-jwks.json")
                    .display()
                    .to_string(),
                audience_prefix: "urn:old:".to_owned(),
            },
            local_control: GeneratedLocalControlConfig {
                address: "127.0.0.1:21115".parse().unwrap(),
                token_file: options
                    .shared_dir
                    .join("local-control.token")
                    .display()
                    .to_string(),
            },
            config: GeneratedManagedConfig {
                path: options.managed_config_path.display().to_string(),
                backup_dir: options.backup_dir.display().to_string(),
                max_bytes: 2 * 1024 * 1024,
                write_enabled: true,
            },
        };
        atomic_write(
            &options.output,
            serde_yml::to_string(&existing).unwrap().as_bytes(),
            0o640,
            false,
        )
        .unwrap();

        let loaded = load_existing_generated_config(&options).unwrap();
        assert_eq!(loaded.listen, existing.listen);
        assert_eq!(loaded.local_control.address, existing.local_control.address);
        assert_eq!(loaded.config.max_bytes, existing.config.max_bytes);
        assert!(loaded.config.write_enabled);
    }

    #[test]
    fn completed_relay_enrollment_retry_is_idempotent_after_lost_response() {
        let temporary = tempfile::tempdir().unwrap();
        let enrollment = temporary.path().join("starry/enrollment");
        create_private_directory(&enrollment).unwrap();
        let raw_code = code("relay", unix_seconds() + 600);
        let code = PairingCode::parse(&raw_code, Purpose::Relay).unwrap();
        let (private_key, certificate, fingerprint) = test_identity();
        atomic_write(
            &enrollment.join("node-key.pem"),
            private_key.as_bytes(),
            0o600,
            false,
        )
        .unwrap();
        atomic_write(
            &enrollment.join("node-cert.pem"),
            certificate.as_bytes(),
            0o640,
            false,
        )
        .unwrap();
        atomic_write(
            &enrollment.join("node-key.pending.pem"),
            private_key.as_bytes(),
            0o600,
            false,
        )
        .unwrap();
        atomic_json(
            &enrollment.join("pairing.pending.json"),
            &PendingIdentity {
                version: 1,
                purpose: "relay".to_owned(),
                action: "enroll".to_owned(),
                enrollment_id: code.enrollment_id.clone(),
                configuration_digest: code.configuration_digest.clone(),
                request_digest: format!("sha256:{}", "3".repeat(64)),
                key_fingerprint: fingerprint.clone(),
                csr_digest: format!("sha256:{}", "4".repeat(64)),
                instance_id: None,
            },
            0o600,
            false,
        )
        .unwrap();
        atomic_json(
            &enrollment.join("enrollment.json"),
            &RelayIdentityMarker {
                version: 1,
                purpose: "relay",
                enrollment_id: &code.enrollment_id,
                configuration_digest: &code.configuration_digest,
                node_id: "relay-test",
                relay_server: "relay.example:21117",
                key_fingerprint: &fingerprint,
                host_binding: &format!("sha256:{}", "5".repeat(64)),
                generation: 1,
            },
            0o600,
            false,
        )
        .unwrap();

        let summary = completed_relay_retry(
            &code,
            &enrollment,
            &enrollment.join("enrollment.json"),
            &enrollment.join("pairing.pending.json"),
            &enrollment.join("node-key.pending.pem"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(summary.enrollment_state, "enrolled");
        assert_eq!(summary.identity_fingerprint, fingerprint);
        assert!(!enrollment.join("pairing.pending.json").exists());
        assert!(!enrollment.join("node-key.pending.pem").exists());

        assert!(completed_relay_retry(
            &code,
            &enrollment,
            &enrollment.join("enrollment.json"),
            &enrollment.join("pairing.pending.json"),
            &enrollment.join("node-key.pending.pem"),
        )
        .unwrap()
        .is_some());
    }

    #[test]
    fn completed_control_pair_retry_is_idempotent_after_lost_response() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        let identity = temporary.path().join("identity");
        let shared = temporary.path().join("shared");
        let backup = temporary.path().join("backup");
        for directory in [&state, &identity, &shared, &backup] {
            create_private_directory(directory).unwrap();
        }
        let raw_code = code("control-agent", unix_seconds() + 600);
        let code = PairingCode::parse(&raw_code, Purpose::ControlAgent).unwrap();
        let (private_key, certificate, fingerprint) = test_identity();
        atomic_write(
            &identity.join("server-key.pem"),
            private_key.as_bytes(),
            0o600,
            false,
        )
        .unwrap();
        atomic_write(
            &identity.join("server-cert.pem"),
            certificate.as_bytes(),
            0o640,
            false,
        )
        .unwrap();
        atomic_write(
            &state.join("server-key.pending.pem"),
            private_key.as_bytes(),
            0o600,
            false,
        )
        .unwrap();
        let instance_id = uuid::Uuid::now_v7().to_string();
        atomic_json(
            &state.join("pairing.pending.json"),
            &PendingIdentity {
                version: 1,
                purpose: "control-agent".to_owned(),
                action: "pair".to_owned(),
                enrollment_id: code.enrollment_id.clone(),
                configuration_digest: code.configuration_digest.clone(),
                request_digest: format!("sha256:{}", "3".repeat(64)),
                key_fingerprint: fingerprint.clone(),
                csr_digest: format!("sha256:{}", "4".repeat(64)),
                instance_id: Some(instance_id.clone()),
            },
            0o600,
            false,
        )
        .unwrap();
        atomic_json(
            &state.join("identity.json"),
            &ControlIdentityMarker {
                version: 1,
                purpose: "control-agent",
                enrollment_id: &code.enrollment_id,
                configuration_digest: &code.configuration_digest,
                instance_id: &instance_id,
                key_fingerprint: &fingerprint,
                generation: 1,
            },
            0o600,
            false,
        )
        .unwrap();
        let options = ControlPairOptions {
            mode: ControlPairMode::Pair,
            tls_server_name: Some("starry.internal".to_owned()),
            state_dir: state.clone(),
            identity_dir: identity,
            output: temporary.path().join("control-agent.yaml"),
            shared_dir: shared,
            managed_config_path: temporary.path().join("config.yaml"),
            backup_dir: backup,
            listen: "127.0.0.1:21121".parse().unwrap(),
            local_control_address: "127.0.0.1:21115".parse().unwrap(),
            broker_ca_file: None,
        };
        let summary = completed_control_retry(
            &code,
            &options,
            &state.join("identity.json"),
            &state.join("pairing.pending.json"),
            &state.join("server-key.pending.pem"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(summary.enrollment_state, "paired");
        assert_eq!(summary.identity_fingerprint, fingerprint);
        assert!(!state.join("pairing.pending.json").exists());
        assert!(!state.join("server-key.pending.pem").exists());
    }

    #[test]
    fn interrupted_relay_ca_creation_recovers_from_the_same_private_key() {
        let directory = tempfile::tempdir().unwrap();
        ensure_relay_ca(directory.path()).unwrap();
        let key = fs::read_to_string(directory.path().join("relay-ca-key.pem")).unwrap();
        let key_fingerprint = digest(&KeyPair::from_pem(&key).unwrap().public_key_der());
        fs::remove_file(directory.path().join("relay-ca.pem")).unwrap();
        ensure_relay_ca(directory.path()).unwrap();
        let certificate = fs::read_to_string(directory.path().join("relay-ca.pem")).unwrap();
        validate_certificate_for_key(&certificate, &key_fingerprint).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn persistence_preflight_uses_the_most_specific_mount() {
        let mountinfo = concat!(
            "1 0 0:1 / / rw - overlay overlay rw\n",
            "2 1 8:1 /data /starry-persist rw - ext4 /dev/sda1 rw\n",
            "3 1 0:2 / /starry-persist/control/tmp rw - tmpfs tmpfs rw\n",
        );
        assert_eq!(
            persistent_filesystem(Path::new("/starry-persist/control/state"), mountinfo,)
                .as_deref(),
            Some("ext4")
        );
        assert_eq!(
            persistent_filesystem(Path::new("/starry-persist/control/tmp/session"), mountinfo,)
                .as_deref(),
            Some("tmpfs")
        );
        assert_eq!(
            persistent_filesystem(Path::new("/unmounted/state"), mountinfo).as_deref(),
            Some("overlay")
        );
    }
}

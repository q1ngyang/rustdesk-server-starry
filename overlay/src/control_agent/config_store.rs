use super::{local_client, AgentError};
use crate::starry_config::{self, ValidatedConfig};
use chrono::{Duration as ChronoDuration, Utc};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Serialize as SerializeTrait};
use serde_derive::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const PLAN_LIFETIME_SECONDS: i64 = 600;
const MAX_COMMENT_BYTES: usize = 500;
const MINIMUM_FREE_STATE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_REVISIONS: usize = 100;
const MAX_REVISION_BYTES: u64 = 128 * 1024 * 1024;
const MAX_RESIDENT_PLANS: usize = 128;
const MAX_RESIDENT_PLAN_BYTES: usize = 16 * 1024 * 1024;
const OPERATION_RETENTION_SECONDS: i64 = 86_400;
const MAX_DURABLE_OPERATIONS: usize = 256;
const MAX_DURABLE_IDEMPOTENCY: usize = 512;
const MAX_DURABLE_AUDITS: usize = 512;
const MAX_DURABLE_STATE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DURABLE_RECORDS_PER_DIRECTORY: usize = 4_096;
const MAX_DURABLE_JSON_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct TransactionCaller {
    pub service: String,
    pub actor: String,
    pub certificate_uri_san: String,
    pub traceparent: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct RuntimeSnapshot {
    pub generation: u64,
    pub source_digest: Option<String>,
    pub effective_digest: Option<String>,
}

impl RuntimeSnapshot {
    pub fn from_value(value: &Value) -> Result<Self, TransactionError> {
        let generation = value
            .get("generation")
            .and_then(Value::as_u64)
            .ok_or_else(|| TransactionError::internal("HBBS omitted the runtime generation."))?;
        Ok(Self {
            generation,
            source_digest: optional_digest(value.get("source_digest"))?,
            effective_digest: optional_digest(value.get("effective_digest"))?,
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct TransactionError {
    pub status: u16,
    pub code: String,
    pub detail: String,
    pub retryable: bool,
    pub errors: Vec<Value>,
}

impl TransactionError {
    pub fn new(
        status: u16,
        code: impl Into<String>,
        detail: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            status,
            code: code.into(),
            detail: detail.into(),
            retryable,
            errors: Vec::new(),
        }
    }

    pub fn with_errors(mut self, errors: Vec<Value>) -> Self {
        self.errors = errors;
        self
    }

    fn internal(detail: impl Into<String>) -> Self {
        Self::new(503, "STARRY_NOT_READY", detail, true)
    }

    fn operation_in_progress() -> Self {
        Self::new(
            409,
            "OPERATION_IN_PROGRESS",
            "Another configuration transaction is in progress.",
            true,
        )
    }
}

impl From<AgentError> for TransactionError {
    fn from(error: AgentError) -> Self {
        let status = match error.code.as_str() {
            "CONFIG_TOO_LARGE" => 413,
            "REQUEST_INVALID" | "CONFIG_INVALID" => 400,
            "LOCAL_CONTROL_UNAVAILABLE" | "LOCAL_CONTROL_TIMEOUT" | "STARRY_NOT_READY" => 503,
            _ => 502,
        };
        Self::new(status, error.code, error.detail, error.retryable)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Operation {
    pub id: String,
    #[serde(default)]
    pub audit_id: Option<String>,
    pub kind: String,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub activation_ack: Option<Value>,
    pub error: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Revision {
    pub id: String,
    pub generation: u64,
    pub before_etag: String,
    pub after_etag: String,
    pub candidate_digest: String,
    pub actor: String,
    pub comment: String,
    pub created_at: String,
    pub result: String,
}

#[derive(Clone)]
struct PlanRecord {
    id: String,
    instance_id: String,
    caller: TransactionCaller,
    base_etag: String,
    base_generation: u64,
    candidate: Vec<u8>,
    candidate_digest: String,
    candidate_effective_digest: String,
    schema_version: u8,
    changes: Vec<Value>,
    risk: String,
    restart_required: bool,
    expires_epoch: i64,
    expires_at: String,
}

impl PlanRecord {
    fn response(&self) -> Value {
        json!({
            "plan_id": self.id,
            "instance_id": self.instance_id,
            "base_etag": self.base_etag,
            "base_generation": self.base_generation,
            "candidate_digest": self.candidate_digest,
            "changes": self.changes,
            "impact": {
                "risk": self.risk,
                "restart_required": self.restart_required
            },
            "expires_at": self.expires_at
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct IdempotencyRecord {
    key_hash: String,
    #[serde(default)]
    instance_id: String,
    #[serde(default)]
    caller_digest: String,
    request_digest: String,
    kind: String,
    created_at: String,
    operation_id: Option<String>,
    response: Option<Value>,
    error: Option<StoredFailure>,
}

trait DurableRecord {
    fn durable_key(&self) -> &str;
}

impl DurableRecord for Operation {
    fn durable_key(&self) -> &str {
        &self.id
    }
}

impl DurableRecord for IdempotencyRecord {
    fn durable_key(&self) -> &str {
        &self.key_hash
    }
}

impl DurableRecord for Revision {
    fn durable_key(&self) -> &str {
        &self.id
    }
}

impl DurableRecord for AuditRecord {
    fn durable_key(&self) -> &str {
        &self.audit_id
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredFailure {
    status: u16,
    code: String,
    detail: String,
    retryable: bool,
}

impl From<TransactionError> for StoredFailure {
    fn from(error: TransactionError) -> Self {
        Self {
            status: error.status,
            code: error.code,
            detail: error.detail,
            retryable: error.retryable,
        }
    }
}

impl From<StoredFailure> for TransactionError {
    fn from(error: StoredFailure) -> Self {
        Self::new(error.status, error.code, error.detail, error.retryable)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AuditRecord {
    audit_id: String,
    operation_id: String,
    request_id: String,
    instance_id: String,
    action: String,
    actor: String,
    service: String,
    certificate_uri_san: String,
    #[serde(default)]
    traceparent: Option<String>,
    before_etag: String,
    after_etag: Option<String>,
    generation: Option<u64>,
    candidate_digest: String,
    result: String,
    error_code: Option<String>,
    recovery_result: Option<String>,
    idempotency_key_hash: String,
    comment: String,
    created_at: String,
    updated_at: String,
}

struct StoreState {
    operations: HashMap<String, Operation>,
    idempotency: HashMap<String, IdempotencyRecord>,
    revisions: HashMap<String, Revision>,
    manual_intervention_required: bool,
}

pub(super) struct ConfigTransactions {
    instance_id: String,
    config_path: PathBuf,
    backup_dir: PathBuf,
    max_config_bytes: usize,
    plans: Mutex<HashMap<String, PlanRecord>>,
    state: Mutex<StoreState>,
    admission: Mutex<()>,
    active_operation: Mutex<Option<String>>,
}

struct TransactionWork {
    operation_id: String,
    request_id: String,
    caller: TransactionCaller,
    candidate: Vec<u8>,
    candidate_digest: String,
    candidate_effective_digest: String,
    schema_version: u8,
    base_etag: String,
    base_runtime: RuntimeSnapshot,
    comment: String,
    audit: AuditRecord,
    lock: File,
}

struct OriginalConfig {
    bytes: Option<Vec<u8>>,
    #[cfg(unix)]
    mode: u32,
    #[cfg(unix)]
    uid: u32,
    #[cfg(unix)]
    gid: u32,
    #[cfg(unix)]
    parent_dev: u64,
    #[cfg(unix)]
    parent_ino: u64,
}

#[derive(Debug)]
struct WriteFailure {
    error: TransactionError,
    may_have_changed: bool,
}

impl ConfigTransactions {
    pub fn open(
        instance_id: String,
        config_path: PathBuf,
        backup_dir: PathBuf,
        max_config_bytes: usize,
    ) -> Result<Arc<Self>, String> {
        for directory in [
            backup_dir.clone(),
            backup_dir.join("operations"),
            backup_dir.join("idempotency"),
            backup_dir.join("revisions"),
            backup_dir.join("audit"),
            backup_dir.join("recovery"),
        ] {
            create_private_directory(&directory)?;
        }
        let mut operations = load_records::<Operation>(&backup_dir.join("operations"))?;
        let mut idempotency: HashMap<String, IdempotencyRecord> =
            load_records::<IdempotencyRecord>(&backup_dir.join("idempotency"))?
                .into_values()
                .map(|record| (record.key_hash.clone(), record))
                .collect();
        let revisions = load_records::<Revision>(&backup_dir.join("revisions"))?;
        let mut audits = load_records::<AuditRecord>(&backup_dir.join("audit"))?;
        for operation in operations
            .values_mut()
            .filter(|operation| operation.audit_id.is_none())
        {
            if let Some(audit) = audits
                .values()
                .find(|audit| audit.operation_id == operation.id)
            {
                operation.audit_id = Some(audit.audit_id.clone());
                atomic_json(
                    &backup_dir
                        .join("operations")
                        .join(format!("{}.json", operation.id)),
                    operation,
                )?;
            }
        }
        let mut manual = operations
            .values()
            .any(|operation| operation.state == "manual_intervention_required");
        let interrupted: Vec<String> = operations
            .values()
            .filter(|operation| matches!(operation.state.as_str(), "pending" | "running"))
            .map(|operation| operation.id.clone())
            .collect();
        for id in interrupted {
            let operation = operations
                .get_mut(&id)
                .expect("interrupted operation came from the operation map");
            operation.state = "manual_intervention_required".to_owned();
            operation.updated_at = now();
            operation.error = Some(problem_value(
                "ROLLBACK_FAILED",
                500,
                "The Agent restarted during a configuration transaction; operator reconciliation is required.",
                false,
                &new_id(),
            ));
            atomic_json(
                &backup_dir.join("operations").join(format!("{id}.json")),
                operation,
            )?;
            for audit in audits.values_mut().filter(|audit| audit.operation_id == id) {
                audit.result = "manual_intervention_required".to_owned();
                audit.error_code = Some("ROLLBACK_FAILED".to_owned());
                audit.recovery_result = Some("agent_restarted".to_owned());
                audit.updated_at = now();
                atomic_json(
                    &backup_dir
                        .join("audit")
                        .join(format!("{}.json", audit.audit_id)),
                    audit,
                )?;
            }
            manual = true;
        }
        for record in idempotency.values_mut().filter(|record| {
            record.kind == "runtime_reload" && record.response.is_none() && record.error.is_none()
        }) {
            record.error = Some(StoredFailure {
                status: 500,
                code: "ROLLBACK_FAILED".to_owned(),
                detail: "The Agent restarted while a runtime reload result was unresolved."
                    .to_owned(),
                retryable: false,
            });
            atomic_json(
                &backup_dir
                    .join("idempotency")
                    .join(format!("{}.json", record.key_hash)),
                record,
            )?;
            manual = true;
        }
        let manager = Arc::new(Self {
            instance_id,
            config_path,
            backup_dir,
            max_config_bytes,
            plans: Mutex::new(HashMap::new()),
            state: Mutex::new(StoreState {
                operations,
                idempotency,
                revisions,
                manual_intervention_required: manual,
            }),
            admission: Mutex::new(()),
            active_operation: Mutex::new(None),
        });
        manager
            .prune_durable_records()
            .map_err(|error| error.detail)?;
        let current_etag = manager
            .read_config()
            .map(|bytes| strong_etag(&bytes))
            .unwrap_or_default();
        manager.prune_revisions(&current_etag)?;
        Ok(manager)
    }

    pub fn read_config(&self) -> Result<Vec<u8>, TransactionError> {
        read_bounded(&self.config_path, self.max_config_bytes)
    }

    pub fn validate_write_authority(&self) -> Result<(), String> {
        validate_managed_parent(&self.config_path)
            .map_err(|error| format!("unsafe managed configuration parent: {}", error.detail))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            let metadata = match fs::symlink_metadata(&self.config_path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
                Err(error) => {
                    return Err(format!(
                        "cannot inspect write-enabled managed configuration {}: {error}",
                        self.config_path.display()
                    ))
                }
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.nlink() != 1 {
                return Err(format!(
                    "write-enabled managed configuration {} must be a regular file with one hard link",
                    self.config_path.display()
                ));
            }
            if metadata.mode() & 0o022 != 0 {
                return Err(format!(
                    "write-enabled managed configuration {} must not be writable by group or other users",
                    self.config_path.display()
                ));
            }
            let effective_uid = unsafe { libc::geteuid() };
            let effective_gid = unsafe { libc::getegid() };
            if !same_owner(metadata.uid(), metadata.gid(), effective_uid, effective_gid) {
                return Err(format!(
                    "write-enabled managed configuration {} is owned by uid {} gid {}, but the Control Agent runs as uid {} gid {}; exact ownership is required for atomic replacement",
                    self.config_path.display(),
                    metadata.uid(),
                    metadata.gid(),
                    effective_uid,
                    effective_gid
                ));
            }
        }
        Ok(())
    }

    pub fn create_plan(
        &self,
        candidate: Vec<u8>,
        validated: &ValidatedConfig,
        supplied_etag: &str,
        runtime: RuntimeSnapshot,
        caller: TransactionCaller,
    ) -> Result<Value, TransactionError> {
        let current = self.read_config()?;
        let current_etag = strong_etag(&current);
        if supplied_etag != current_etag {
            return Err(etag_mismatch());
        }
        let before = normalized_config(&current);
        let after = serde_json::to_value(&validated.config)
            .map_err(|_| TransactionError::internal("Cannot normalize the configuration plan."))?;
        let mut changes = Vec::new();
        collect_changes("", before.as_ref(), Some(&after), &mut changes);
        let risk = classify_risk(&changes, before.as_ref(), &after);
        let id = new_id();
        let expires = Utc::now() + ChronoDuration::seconds(PLAN_LIFETIME_SECONDS);
        let record = PlanRecord {
            id: id.clone(),
            instance_id: self.instance_id.clone(),
            caller,
            base_etag: current_etag,
            base_generation: runtime.generation,
            candidate,
            candidate_digest: validated.source_digest.clone(),
            candidate_effective_digest: validated.effective_digest.clone(),
            schema_version: validated.config.version,
            changes,
            risk,
            restart_required: false,
            expires_epoch: expires.timestamp(),
            expires_at: expires.to_rfc3339(),
        };
        let response = record.response();
        let mut plans = self
            .plans
            .lock()
            .map_err(|_| TransactionError::internal("Configuration plan state is unavailable."))?;
        plans.retain(|_, plan| plan.expires_epoch >= Utc::now().timestamp());
        let resident_bytes = plans
            .values()
            .map(|plan| plan.candidate.len())
            .sum::<usize>();
        if plans.len() >= MAX_RESIDENT_PLANS
            || resident_bytes.saturating_add(record.candidate.len()) > MAX_RESIDENT_PLAN_BYTES
        {
            return Err(TransactionError::new(
                429,
                "PLAN_CAPACITY_EXCEEDED",
                "The bounded configuration plan cache is full; retry after existing plans expire.",
                true,
            ));
        }
        plans.insert(id, record);
        Ok(response)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn accept_apply(
        self: &Arc<Self>,
        plan_id: &str,
        candidate_digest: &str,
        supplied_etag: &str,
        idempotency_key: &str,
        comment: Option<&str>,
        caller: &TransactionCaller,
        runtime: RuntimeSnapshot,
        request_id: &str,
        local_address: SocketAddr,
    ) -> Result<Operation, TransactionError> {
        validate_comment(comment)?;
        let request_digest = request_digest(&json!({
            "kind": "config_apply",
            "plan_id": plan_id,
            "candidate_digest": candidate_digest,
            "if_match": supplied_etag,
            "comment": comment.unwrap_or("")
        }))?;
        let key_hash = idempotency_hash(idempotency_key);
        let _admission = self
            .admission
            .lock()
            .map_err(|_| TransactionError::internal("Transaction admission is unavailable."))?;
        if let Some(operation) = self.idempotent_operation(&key_hash, &request_digest, caller)? {
            return Ok(operation);
        }
        self.ensure_writable()?;
        let plan = {
            let plans = self.plans.lock().map_err(|_| {
                TransactionError::internal("Configuration plan state is unavailable.")
            })?;
            plans.get(plan_id).cloned().ok_or_else(|| {
                TransactionError::new(
                    409,
                    "PLAN_STALE",
                    "The configuration plan is unknown or no longer available.",
                    false,
                )
            })?
        };
        self.validate_plan(&plan, candidate_digest, supplied_etag, caller, &runtime)?;
        let current = self.read_config()?;
        if strong_etag(&current) != supplied_etag {
            return Err(etag_mismatch());
        }
        let operation_id = new_id();
        let lock = self.reserve_operation(&operation_id)?;
        let operation = Operation {
            id: operation_id.clone(),
            audit_id: Some(new_id()),
            kind: "config_apply".to_owned(),
            state: "pending".to_owned(),
            created_at: now(),
            updated_at: now(),
            activation_ack: None,
            error: None,
        };
        let audit = self.audit_intent(
            &operation,
            request_id,
            caller,
            supplied_etag,
            candidate_digest,
            &key_hash,
            comment,
        );
        let idempotency = IdempotencyRecord {
            key_hash: key_hash.clone(),
            instance_id: self.instance_id.clone(),
            caller_digest: caller_digest(caller),
            request_digest,
            kind: operation.kind.clone(),
            created_at: now(),
            operation_id: Some(operation_id.clone()),
            response: None,
            error: None,
        };
        if let Err(error) = self.persist_intent(&operation, &audit, &idempotency) {
            self.release_operation(&operation_id);
            drop(lock);
            return Err(error);
        }
        let work = TransactionWork {
            operation_id,
            request_id: request_id.to_owned(),
            caller: caller.clone(),
            candidate: plan.candidate,
            candidate_digest: plan.candidate_digest,
            candidate_effective_digest: plan.candidate_effective_digest,
            schema_version: plan.schema_version,
            base_etag: plan.base_etag,
            base_runtime: runtime,
            comment: comment.unwrap_or("").to_owned(),
            audit,
            lock,
        };
        let manager = Arc::clone(self);
        hbb_common::tokio::spawn(async move {
            manager.execute_transaction(work, local_address).await;
        });
        Ok(operation)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn accept_rollback(
        self: &Arc<Self>,
        revision_id: &str,
        supplied_etag: &str,
        idempotency_key: &str,
        comment: Option<&str>,
        caller: &TransactionCaller,
        runtime: RuntimeSnapshot,
        request_id: &str,
        local_address: SocketAddr,
    ) -> Result<Operation, TransactionError> {
        validate_comment(comment)?;
        uuid::Uuid::parse_str(revision_id).map_err(|_| {
            TransactionError::new(400, "REQUEST_INVALID", "revision_id must be a UUID.", false)
        })?;
        let request_digest = request_digest(&json!({
            "kind": "config_rollback",
            "revision_id": revision_id,
            "if_match": supplied_etag,
            "comment": comment.unwrap_or("")
        }))?;
        let key_hash = idempotency_hash(idempotency_key);
        let _admission = self
            .admission
            .lock()
            .map_err(|_| TransactionError::internal("Transaction admission is unavailable."))?;
        if let Some(operation) = self.idempotent_operation(&key_hash, &request_digest, caller)? {
            return Ok(operation);
        }
        self.ensure_writable()?;
        let revision = {
            let state = self
                .state
                .lock()
                .map_err(|_| TransactionError::internal("Revision state is unavailable."))?;
            state.revisions.get(revision_id).cloned().ok_or_else(|| {
                TransactionError::new(
                    404,
                    "REQUEST_INVALID",
                    "The requested revision does not exist.",
                    false,
                )
            })?
        };
        let candidate = read_bounded(
            &self.revisions_dir().join(format!("{}.yaml", revision.id)),
            self.max_config_bytes,
        )?;
        let validated = validate_candidate(&candidate)?;
        let current = self.read_config()?;
        if strong_etag(&current) != supplied_etag {
            return Err(etag_mismatch());
        }
        let operation_id = new_id();
        let lock = self.reserve_operation(&operation_id)?;
        let operation = Operation {
            id: operation_id.clone(),
            audit_id: Some(new_id()),
            kind: "config_rollback".to_owned(),
            state: "pending".to_owned(),
            created_at: now(),
            updated_at: now(),
            activation_ack: None,
            error: None,
        };
        let audit = self.audit_intent(
            &operation,
            request_id,
            caller,
            supplied_etag,
            &validated.source_digest,
            &key_hash,
            comment,
        );
        let idempotency = IdempotencyRecord {
            key_hash: key_hash.clone(),
            instance_id: self.instance_id.clone(),
            caller_digest: caller_digest(caller),
            request_digest,
            kind: operation.kind.clone(),
            created_at: now(),
            operation_id: Some(operation_id.clone()),
            response: None,
            error: None,
        };
        if let Err(error) = self.persist_intent(&operation, &audit, &idempotency) {
            self.release_operation(&operation_id);
            drop(lock);
            return Err(error);
        }
        let work = TransactionWork {
            operation_id,
            request_id: request_id.to_owned(),
            caller: caller.clone(),
            candidate,
            candidate_digest: validated.source_digest,
            candidate_effective_digest: validated.effective_digest,
            schema_version: validated.config.version,
            base_etag: supplied_etag.to_owned(),
            base_runtime: runtime,
            comment: comment.unwrap_or("").to_owned(),
            audit,
            lock,
        };
        let manager = Arc::clone(self);
        hbb_common::tokio::spawn(async move {
            manager.execute_transaction(work, local_address).await;
        });
        Ok(operation)
    }

    pub fn operation(&self, id: &str) -> Result<Operation, TransactionError> {
        uuid::Uuid::parse_str(id).map_err(|_| {
            TransactionError::new(
                400,
                "REQUEST_INVALID",
                "operation id must be a UUID.",
                false,
            )
        })?;
        self.state
            .lock()
            .map_err(|_| TransactionError::internal("Operation state is unavailable."))?
            .operations
            .get(id)
            .cloned()
            .ok_or_else(|| {
                TransactionError::new(
                    404,
                    "REQUEST_INVALID",
                    "The requested operation does not exist.",
                    false,
                )
            })
    }

    pub fn history(&self) -> Result<Vec<Revision>, TransactionError> {
        let mut revisions: Vec<Revision> = self
            .state
            .lock()
            .map_err(|_| TransactionError::internal("Revision state is unavailable."))?
            .revisions
            .values()
            .cloned()
            .collect();
        revisions.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(revisions)
    }

    pub async fn runtime_reload(
        &self,
        expected_source_digest: &str,
        idempotency_key: &str,
        request_id: &str,
        caller: &TransactionCaller,
        local_address: SocketAddr,
    ) -> Result<Value, TransactionError> {
        validate_digest(expected_source_digest)?;
        let request_digest = request_digest(&json!({
            "kind": "runtime_reload",
            "expected_source_digest": expected_source_digest
        }))?;
        let key_hash = idempotency_hash(idempotency_key);
        let operation_id = format!("reload:{}", new_id());
        let (lock, record, mut audit) = {
            let _admission = self
                .admission
                .lock()
                .map_err(|_| TransactionError::internal("Transaction admission is unavailable."))?;
            if let Some(outcome) = self.idempotent_reload(&key_hash, &request_digest, caller)? {
                return outcome;
            }
            self.ensure_writable()?;
            let current = self.read_config()?;
            if digest(&current) != expected_source_digest {
                return Err(etag_mismatch());
            }
            let lock = self.reserve_operation(&operation_id)?;
            let record = IdempotencyRecord {
                key_hash: key_hash.clone(),
                instance_id: self.instance_id.clone(),
                caller_digest: caller_digest(caller),
                request_digest,
                kind: "runtime_reload".to_owned(),
                created_at: now(),
                operation_id: None,
                response: None,
                error: None,
            };
            let audit = AuditRecord {
                audit_id: new_id(),
                operation_id: operation_id.clone(),
                request_id: request_id.to_owned(),
                instance_id: self.instance_id.clone(),
                action: "runtime_reload".to_owned(),
                actor: caller.actor.clone(),
                service: caller.service.clone(),
                certificate_uri_san: caller.certificate_uri_san.clone(),
                traceparent: caller.traceparent.clone(),
                before_etag: strong_etag(&current),
                after_etag: None,
                generation: None,
                candidate_digest: expected_source_digest.to_owned(),
                result: "intent_persisted".to_owned(),
                error_code: None,
                recovery_result: None,
                idempotency_key_hash: key_hash.clone(),
                comment: String::new(),
                created_at: now(),
                updated_at: now(),
            };
            if atomic_json(&self.audit_path(&audit.audit_id), &audit).is_err() {
                self.release_operation(&operation_id);
                drop(lock);
                return Err(TransactionError::internal(
                    "Cannot persist the runtime reload audit intent.",
                ));
            }
            if let Err(error) = self.persist_idempotency(&record) {
                let mut failed_audit = audit.clone();
                failed_audit.result = "failed".to_owned();
                failed_audit.error_code = Some(error.code.clone());
                failed_audit.updated_at = now();
                let _ = atomic_json(&self.audit_path(&failed_audit.audit_id), &failed_audit);
                self.release_operation(&operation_id);
                drop(lock);
                return Err(error);
            }
            (lock, record, audit)
        };
        let mut result = local_client::call(local_address, request_id, "runtime.reload", json!({}))
            .await
            .map_err(TransactionError::from)
            .and_then(|mut ack| {
                if ack.get("source_digest").and_then(Value::as_str) != Some(expected_source_digest)
                    || ack
                        .get("subsystem_acks")
                        .and_then(Value::as_array)
                        .is_none_or(|acks| {
                            acks.is_empty()
                                || acks.iter().any(|ack| {
                                    ack.get("accepted").and_then(Value::as_bool) != Some(true)
                                })
                        })
                {
                    Err(TransactionError::internal(
                        "HBBS returned an incomplete runtime activation acknowledgement.",
                    ))
                } else {
                    ack.as_object_mut()
                        .ok_or_else(|| {
                            TransactionError::internal(
                                "HBBS returned a non-object runtime activation acknowledgement.",
                            )
                        })?
                        .insert("audit_id".to_owned(), Value::String(audit.audit_id.clone()));
                    Ok(ack)
                }
            });
        audit.updated_at = now();
        match &result {
            Ok(response) => {
                audit.result = "succeeded".to_owned();
                audit.after_etag = Some(audit.before_etag.clone());
                audit.generation = response.get("generation").and_then(Value::as_u64);
            }
            Err(error) => {
                audit.result = "failed".to_owned();
                audit.error_code = Some(error.code.clone());
            }
        }
        if atomic_json(&self.audit_path(&audit.audit_id), &audit).is_err() {
            self.mark_manual();
            result = Err(TransactionError::new(
                500,
                "ROLLBACK_FAILED",
                "The runtime reload completed without a durable audit result.",
                false,
            ));
        }
        let mut completed = record;
        match &result {
            Ok(response) => completed.response = Some(response.clone()),
            Err(error) => completed.error = Some(error.clone().into()),
        }
        if let Err(error) = self.persist_idempotency(&completed) {
            self.mark_manual();
            self.release_operation(&operation_id);
            drop(lock);
            return Err(error);
        }
        self.release_operation(&operation_id);
        drop(lock);
        result
    }

    fn validate_plan(
        &self,
        plan: &PlanRecord,
        candidate_digest: &str,
        supplied_etag: &str,
        caller: &TransactionCaller,
        runtime: &RuntimeSnapshot,
    ) -> Result<(), TransactionError> {
        if plan.expires_epoch < Utc::now().timestamp() {
            return Err(TransactionError::new(
                409,
                "PLAN_EXPIRED",
                "The configuration plan has expired.",
                false,
            ));
        }
        if plan.instance_id != self.instance_id
            || plan.caller.service != caller.service
            || plan.caller.actor != caller.actor
            || plan.caller.certificate_uri_san != caller.certificate_uri_san
            || plan.candidate_digest != candidate_digest
        {
            return Err(TransactionError::new(
                409,
                "PLAN_STALE",
                "The configuration plan does not match this request.",
                false,
            ));
        }
        if plan.base_etag != supplied_etag {
            return Err(etag_mismatch());
        }
        if plan.base_generation != runtime.generation {
            return Err(TransactionError::new(
                409,
                "PLAN_STALE",
                "The active configuration generation changed after the plan was created.",
                false,
            ));
        }
        if plan.restart_required {
            return Err(TransactionError::new(
                409,
                "RESTART_REQUIRED",
                "The planned change requires a process restart and cannot be applied by Control API v1.",
                false,
            ));
        }
        Ok(())
    }

    fn ensure_writable(&self) -> Result<(), TransactionError> {
        self.prune_durable_records()?;
        let state = self
            .state
            .lock()
            .map_err(|_| TransactionError::internal("Transaction state is unavailable."))?;
        if state.manual_intervention_required {
            return Err(TransactionError::new(
                503,
                "ROLLBACK_FAILED",
                "A previous transaction requires operator reconciliation before further writes.",
                false,
            ));
        }
        if state.operations.len() >= MAX_DURABLE_OPERATIONS
            || state.idempotency.len() >= MAX_DURABLE_IDEMPOTENCY
            || durable_state_bytes(&self.backup_dir)? >= MAX_DURABLE_STATE_BYTES
        {
            return Err(TransactionError::new(
                503,
                "STARRY_NOT_READY",
                "The bounded durable transaction store is full; reconcile protected records before retrying.",
                true,
            ));
        }
        let available = fs2::available_space(&self.backup_dir).map_err(|_| {
            TransactionError::internal("Cannot inspect free space for configuration recovery.")
        })?;
        if available < MINIMUM_FREE_STATE_BYTES {
            return Err(TransactionError::new(
                503,
                "STARRY_NOT_READY",
                "Insufficient free space is available for an atomic configuration backup.",
                true,
            ));
        }
        Ok(())
    }

    fn prune_durable_records(&self) -> Result<(), TransactionError> {
        let mut audits = load_records::<AuditRecord>(&self.backup_dir.join("audit"))
            .map_err(TransactionError::internal)?;
        let cutoff = Utc::now().timestamp() - OPERATION_RETENTION_SECONDS;
        let mut state = self
            .state
            .lock()
            .map_err(|_| TransactionError::internal("Durable transaction state is unavailable."))?;

        let mut terminal: Vec<Operation> = state
            .operations
            .values()
            .filter(|operation| operation_is_terminal(operation))
            .cloned()
            .collect();
        terminal.sort_by(|left, right| left.updated_at.cmp(&right.updated_at));
        for operation in terminal {
            let expired = timestamp_before(&operation.updated_at, cutoff)?;
            let over_budget = state.operations.len() > MAX_DURABLE_OPERATIONS
                || state.idempotency.len() > MAX_DURABLE_IDEMPOTENCY
                || audits.len() > MAX_DURABLE_AUDITS
                || durable_state_bytes(&self.backup_dir)? > MAX_DURABLE_STATE_BYTES;
            if !expired && !over_budget {
                continue;
            }
            remove_if_present(&self.operation_path(&operation.id))?;
            for suffix in ["json", "yaml"] {
                remove_if_present(
                    &self
                        .backup_dir
                        .join("recovery")
                        .join(format!("{}.{}", operation.id, suffix)),
                )?;
            }
            let idempotency_keys: Vec<String> = state
                .idempotency
                .values()
                .filter(|record| record.operation_id.as_deref() == Some(&operation.id))
                .map(|record| record.key_hash.clone())
                .collect();
            for key in idempotency_keys {
                remove_if_present(&self.idempotency_path(&key))?;
                state.idempotency.remove(&key);
            }
            let audit_ids: Vec<String> = audits
                .values()
                .filter(|audit| audit.operation_id == operation.id)
                .map(|audit| audit.audit_id.clone())
                .collect();
            for id in audit_ids {
                remove_if_present(&self.audit_path(&id))?;
                audits.remove(&id);
            }
            state.operations.remove(&operation.id);
        }

        let protected_operations: std::collections::HashSet<String> = state
            .operations
            .values()
            .filter(|operation| !operation_is_terminal(operation))
            .map(|operation| operation.id.clone())
            .collect();
        let mut standalone_idempotency: Vec<IdempotencyRecord> = state
            .idempotency
            .values()
            .filter(|record| {
                record
                    .operation_id
                    .as_deref()
                    .is_none_or(|id| !protected_operations.contains(id))
            })
            .cloned()
            .collect();
        standalone_idempotency.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        for record in standalone_idempotency {
            if !timestamp_before(&record.created_at, cutoff)?
                && state.idempotency.len() <= MAX_DURABLE_IDEMPOTENCY
            {
                continue;
            }
            remove_if_present(&self.idempotency_path(&record.key_hash))?;
            state.idempotency.remove(&record.key_hash);
        }

        let mut standalone_audits: Vec<AuditRecord> = audits
            .values()
            .filter(|audit| !protected_operations.contains(audit.operation_id.as_str()))
            .cloned()
            .collect();
        standalone_audits.sort_by(|left, right| left.updated_at.cmp(&right.updated_at));
        for audit in standalone_audits {
            if !timestamp_before(&audit.updated_at, cutoff)? && audits.len() <= MAX_DURABLE_AUDITS {
                continue;
            }
            remove_if_present(&self.audit_path(&audit.audit_id))?;
            audits.remove(&audit.audit_id);
        }

        for directory in ["operations", "idempotency", "audit", "recovery"] {
            sync_directory(&self.backup_dir.join(directory)).map_err(TransactionError::internal)?;
        }
        Ok(())
    }

    fn reserve_operation(&self, id: &str) -> Result<File, TransactionError> {
        let mut active = self
            .active_operation
            .lock()
            .map_err(|_| TransactionError::internal("Transaction lock state is unavailable."))?;
        if active.is_some() {
            return Err(TransactionError::operation_in_progress());
        }
        let lock_path = self.backup_dir.join("transaction.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|_| {
                TransactionError::internal("Cannot open the configuration transaction lock.")
            })?;
        lock.try_lock_exclusive().map_err(|error| {
            if error.kind() == ErrorKind::WouldBlock {
                TransactionError::operation_in_progress()
            } else {
                TransactionError::internal("Cannot acquire the configuration transaction lock.")
            }
        })?;
        *active = Some(id.to_owned());
        Ok(lock)
    }

    fn release_operation(&self, id: &str) {
        if let Ok(mut active) = self.active_operation.lock() {
            if active.as_deref() == Some(id) {
                *active = None;
            }
        }
    }

    fn idempotent_operation(
        &self,
        key_hash: &str,
        request_digest: &str,
        caller: &TransactionCaller,
    ) -> Result<Option<Operation>, TransactionError> {
        let state = self
            .state
            .lock()
            .map_err(|_| TransactionError::internal("Idempotency state is unavailable."))?;
        let Some(record) = state.idempotency.get(key_hash) else {
            return Ok(None);
        };
        if record.request_digest != request_digest
            || record.instance_id != self.instance_id
            || record.caller_digest != caller_digest(caller)
        {
            return Err(idempotency_reused());
        }
        let id = record.operation_id.as_deref().ok_or_else(|| {
            TransactionError::new(
                409,
                "IDEMPOTENCY_KEY_REUSED",
                "The idempotency key belongs to a different operation kind.",
                false,
            )
        })?;
        state.operations.get(id).cloned().map(Some).ok_or_else(|| {
            TransactionError::internal("The durable idempotency record has no operation.")
        })
    }

    fn idempotent_reload(
        &self,
        key_hash: &str,
        request_digest: &str,
        caller: &TransactionCaller,
    ) -> Result<Option<Result<Value, TransactionError>>, TransactionError> {
        let state = self
            .state
            .lock()
            .map_err(|_| TransactionError::internal("Idempotency state is unavailable."))?;
        let Some(record) = state.idempotency.get(key_hash) else {
            return Ok(None);
        };
        if record.request_digest != request_digest
            || record.kind != "runtime_reload"
            || record.instance_id != self.instance_id
            || record.caller_digest != caller_digest(caller)
        {
            return Err(idempotency_reused());
        }
        if let Some(response) = &record.response {
            return Ok(Some(Ok(response.clone())));
        }
        if let Some(error) = &record.error {
            return Ok(Some(Err(error.clone().into())));
        }
        Ok(Some(Err(TransactionError::operation_in_progress())))
    }

    fn persist_intent(
        &self,
        operation: &Operation,
        audit: &AuditRecord,
        idempotency: &IdempotencyRecord,
    ) -> Result<(), TransactionError> {
        atomic_json(&self.audit_path(&audit.audit_id), audit).map_err(|_| {
            TransactionError::internal("Cannot persist the configuration audit intent.")
        })?;
        atomic_json(&self.operation_path(&operation.id), operation).map_err(|_| {
            TransactionError::internal("Cannot persist the configuration operation.")
        })?;
        atomic_json(&self.idempotency_path(&idempotency.key_hash), idempotency)
            .map_err(|_| TransactionError::internal("Cannot persist the idempotency record."))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| TransactionError::internal("Transaction state is unavailable."))?;
        state
            .operations
            .insert(operation.id.clone(), operation.clone());
        state
            .idempotency
            .insert(idempotency.key_hash.clone(), idempotency.clone());
        Ok(())
    }

    fn persist_idempotency(&self, record: &IdempotencyRecord) -> Result<(), TransactionError> {
        atomic_json(&self.idempotency_path(&record.key_hash), record)
            .map_err(|_| TransactionError::internal("Cannot persist the idempotency result."))?;
        self.state
            .lock()
            .map_err(|_| TransactionError::internal("Idempotency state is unavailable."))?
            .idempotency
            .insert(record.key_hash.clone(), record.clone());
        Ok(())
    }

    fn audit_intent(
        &self,
        operation: &Operation,
        request_id: &str,
        caller: &TransactionCaller,
        before_etag: &str,
        candidate_digest: &str,
        idempotency_key_hash: &str,
        comment: Option<&str>,
    ) -> AuditRecord {
        AuditRecord {
            audit_id: operation
                .audit_id
                .clone()
                .expect("configuration operations always allocate an audit ID"),
            operation_id: operation.id.clone(),
            request_id: request_id.to_owned(),
            instance_id: self.instance_id.clone(),
            action: operation.kind.clone(),
            actor: caller.actor.clone(),
            service: caller.service.clone(),
            certificate_uri_san: caller.certificate_uri_san.clone(),
            traceparent: caller.traceparent.clone(),
            before_etag: before_etag.to_owned(),
            after_etag: None,
            generation: None,
            candidate_digest: candidate_digest.to_owned(),
            result: "intent_persisted".to_owned(),
            error_code: None,
            recovery_result: None,
            idempotency_key_hash: idempotency_key_hash.to_owned(),
            comment: comment.unwrap_or("").to_owned(),
            created_at: now(),
            updated_at: now(),
        }
    }

    async fn execute_transaction(
        self: Arc<Self>,
        mut work: TransactionWork,
        local_address: SocketAddr,
    ) {
        if self
            .update_operation(&work.operation_id, "running", None, None)
            .is_err()
        {
            self.mark_manual();
            self.release_operation(&work.operation_id);
            return;
        }
        #[cfg(debug_assertions)]
        if let Some(delay_ms) = std::env::var("STARRY_TEST_TRANSACTION_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value <= 5_000)
        {
            hbb_common::tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        work.audit.result = "running".to_owned();
        work.audit.updated_at = now();
        if atomic_json(&self.audit_path(&work.audit.audit_id), &work.audit).is_err() {
            let error = TransactionError::internal("Cannot update the durable audit record.");
            self.finish_operation(&mut work, "failed", None, error, None);
            self.release_operation(&work.operation_id);
            return;
        }

        let original = match self.read_original() {
            Ok(original) => original,
            Err(error) => {
                self.finish_operation(&mut work, "failed", None, error, None);
                self.release_operation(&work.operation_id);
                return;
            }
        };
        let original_bytes = original.bytes.as_deref().unwrap_or_default();
        if strong_etag(original_bytes) != work.base_etag {
            self.finish_operation(&mut work, "failed", None, etag_mismatch(), None);
            self.release_operation(&work.operation_id);
            return;
        }
        match local_client::call(
            local_address,
            &work.request_id,
            "config.runtime_state",
            json!({}),
        )
        .await
        .map_err(TransactionError::from)
        .and_then(|value| RuntimeSnapshot::from_value(&value))
        {
            Ok(current)
                if current.generation == work.base_runtime.generation
                    && current.source_digest == work.base_runtime.source_digest
                    && current.effective_digest == work.base_runtime.effective_digest => {}
            Ok(_) => {
                self.finish_operation(
                    &mut work,
                    "failed",
                    None,
                    TransactionError::new(
                        409,
                        "PLAN_STALE",
                        "The HBBS runtime changed before the transaction started.",
                        false,
                    ),
                    None,
                );
                self.release_operation(&work.operation_id);
                return;
            }
            Err(error) => {
                self.finish_operation(&mut work, "failed", None, error, None);
                self.release_operation(&work.operation_id);
                return;
            }
        }
        if let Err(error) = self.persist_recovery(&work, &original) {
            self.finish_operation(&mut work, "failed", None, error, None);
            self.release_operation(&work.operation_id);
            return;
        }
        if let Err(error) = self.ensure_baseline_revision(&work, &original) {
            self.finish_operation(&mut work, "failed", None, error, None);
            self.release_operation(&work.operation_id);
            return;
        }
        match self.read_config() {
            Ok(bytes) if bytes == original_bytes => {}
            Ok(_) => {
                self.finish_operation(&mut work, "failed", None, etag_mismatch(), None);
                self.release_operation(&work.operation_id);
                return;
            }
            Err(error) => {
                self.finish_operation(&mut work, "failed", None, error, None);
                self.release_operation(&work.operation_id);
                return;
            }
        }

        let write_result = atomic_replace_config(&self.config_path, &work.candidate, &original);
        if let Err(failure) = write_result {
            if failure.may_have_changed {
                self.recover_transaction(&mut work, &original, failure.error, local_address)
                    .await;
            } else {
                self.finish_operation(&mut work, "failed", None, failure.error, None);
            }
            self.release_operation(&work.operation_id);
            return;
        }
        #[cfg(debug_assertions)]
        if let Some(delay_ms) = std::env::var("STARRY_TEST_POST_WRITE_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value <= 5_000)
        {
            hbb_common::tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        let activation =
            local_client::call(local_address, &work.request_id, "runtime.reload", json!({}))
                .await
                .map_err(TransactionError::from)
                .and_then(|ack| self.validate_activation_ack(&work, ack));
        let ack = match activation {
            Ok(ack) => ack,
            Err(error) => {
                self.recover_transaction(&mut work, &original, error, local_address)
                    .await;
                self.release_operation(&work.operation_id);
                return;
            }
        };
        let after_etag = strong_etag(&work.candidate);
        let generation = ack.get("generation").and_then(Value::as_u64).unwrap_or(0);
        if let Err(error) = self.save_revision(&work, generation, &after_etag) {
            self.recover_transaction(&mut work, &original, error, local_address)
                .await;
            self.release_operation(&work.operation_id);
            return;
        }
        work.audit.after_etag = Some(after_etag);
        work.audit.generation = Some(generation);
        work.audit.result = "succeeded".to_owned();
        work.audit.updated_at = now();
        if atomic_json(&self.audit_path(&work.audit.audit_id), &work.audit).is_err()
            || self
                .update_operation(&work.operation_id, "succeeded", Some(ack), None)
                .is_err()
        {
            self.mark_manual();
            let error = TransactionError::new(
                500,
                "ROLLBACK_FAILED",
                "The configuration activated but its durable result could not be recorded.",
                false,
            );
            self.finish_operation(
                &mut work,
                "manual_intervention_required",
                None,
                error,
                Some("durable_result_failed"),
            );
        }
        self.release_operation(&work.operation_id);
        drop(work.lock);
    }

    fn validate_activation_ack(
        &self,
        work: &TransactionWork,
        ack: Value,
    ) -> Result<Value, TransactionError> {
        let accepted = ack
            .get("subsystem_acks")
            .and_then(Value::as_array)
            .is_some_and(|acks| {
                !acks.is_empty()
                    && acks
                        .iter()
                        .all(|ack| ack.get("accepted").and_then(Value::as_bool) == Some(true))
            });
        if ack.get("source_digest").and_then(Value::as_str) != Some(work.candidate_digest.as_str())
            || ack.get("effective_digest").and_then(Value::as_str)
                != Some(work.candidate_effective_digest.as_str())
            || ack.get("schema_version").and_then(Value::as_u64) != Some(work.schema_version as u64)
            || ack.get("generation").and_then(Value::as_u64) <= Some(work.base_runtime.generation)
            || !accepted
        {
            return Err(TransactionError::internal(
                "HBBS returned an activation acknowledgement that does not match the candidate.",
            ));
        }
        Ok(ack)
    }

    async fn recover_transaction(
        &self,
        work: &mut TransactionWork,
        original: &OriginalConfig,
        cause: TransactionError,
        local_address: SocketAddr,
    ) {
        let restored = restore_config(&self.config_path, original)
            .and_then(|_| self.verify_restored_bytes(original));
        let recovery = match restored {
            Ok(()) => {
                let _ = local_client::call(
                    local_address,
                    &work.request_id,
                    "runtime.reload",
                    json!({}),
                )
                .await;
                local_client::call(
                    local_address,
                    &work.request_id,
                    "config.runtime_state",
                    json!({}),
                )
                .await
                .map_err(TransactionError::from)
                .and_then(|value| RuntimeSnapshot::from_value(&value))
                .and_then(|runtime| {
                    if runtime.source_digest == work.base_runtime.source_digest
                        && runtime.effective_digest == work.base_runtime.effective_digest
                    {
                        Ok(())
                    } else {
                        Err(TransactionError::new(
                            500,
                            "ROLLBACK_FAILED",
                            "HBBS did not return to the previous runtime configuration.",
                            false,
                        ))
                    }
                })
            }
            Err(error) => Err(error),
        };
        match recovery {
            Ok(()) => self.finish_operation(
                work,
                "rolled_back",
                None,
                cause,
                Some("restored_and_acknowledged"),
            ),
            Err(recovery_error) => {
                self.mark_manual();
                let error = TransactionError::new(
                    500,
                    "ROLLBACK_FAILED",
                    format!(
                        "The configuration transaction failed and automatic recovery was not acknowledged: {}",
                        recovery_error.detail
                    ),
                    false,
                );
                self.finish_operation(
                    work,
                    "manual_intervention_required",
                    None,
                    error,
                    Some("failed"),
                );
            }
        }
    }

    fn finish_operation(
        &self,
        work: &mut TransactionWork,
        state: &str,
        ack: Option<Value>,
        error: TransactionError,
        recovery: Option<&str>,
    ) {
        work.audit.result = state.to_owned();
        work.audit.error_code = Some(error.code.clone());
        work.audit.recovery_result = recovery.map(str::to_owned);
        work.audit.updated_at = now();
        let _ = atomic_json(&self.audit_path(&work.audit.audit_id), &work.audit);
        let problem = problem_value(
            &error.code,
            error.status,
            &error.detail,
            error.retryable,
            &work.request_id,
        );
        if self
            .update_operation(&work.operation_id, state, ack, Some(problem))
            .is_err()
        {
            self.mark_manual();
        }
    }

    fn update_operation(
        &self,
        id: &str,
        state_name: &str,
        ack: Option<Value>,
        error: Option<Value>,
    ) -> Result<(), TransactionError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| TransactionError::internal("Operation state is unavailable."))?;
        let mut operation = state.operations.get(id).cloned().ok_or_else(|| {
            TransactionError::internal("The durable configuration operation is missing.")
        })?;
        operation.state = state_name.to_owned();
        operation.updated_at = now();
        operation.activation_ack = ack;
        operation.error = error;
        atomic_json(&self.operation_path(id), &operation)
            .map_err(|_| TransactionError::internal("Cannot persist the operation result."))?;
        state.operations.insert(id.to_owned(), operation);
        Ok(())
    }

    fn mark_manual(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.manual_intervention_required = true;
        }
    }

    fn read_original(&self) -> Result<OriginalConfig, TransactionError> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        #[cfg(unix)]
        let parent = validate_managed_parent(&self.config_path)?;
        match read_regular_bounded(&self.config_path, self.max_config_bytes)? {
            Some((bytes, metadata)) => {
                #[cfg(unix)]
                {
                    if !same_owner(
                        metadata.uid(),
                        metadata.gid(),
                        unsafe { libc::geteuid() },
                        unsafe { libc::getegid() },
                    ) {
                        return Err(unsafe_config_path());
                    }
                    Ok(OriginalConfig {
                        bytes: Some(bytes),
                        mode: metadata.mode(),
                        uid: metadata.uid(),
                        gid: metadata.gid(),
                        parent_dev: parent.dev(),
                        parent_ino: parent.ino(),
                    })
                }
                #[cfg(not(unix))]
                {
                    Ok(OriginalConfig { bytes: Some(bytes) })
                }
            }
            None => {
                #[cfg(unix)]
                {
                    Ok(OriginalConfig {
                        bytes: None,
                        mode: 0o600,
                        uid: unsafe { libc::geteuid() },
                        gid: unsafe { libc::getegid() },
                        parent_dev: parent.dev(),
                        parent_ino: parent.ino(),
                    })
                }
                #[cfg(not(unix))]
                {
                    Ok(OriginalConfig { bytes: None })
                }
            }
        }
    }

    fn persist_recovery(
        &self,
        work: &TransactionWork,
        original: &OriginalConfig,
    ) -> Result<(), TransactionError> {
        let directory = self.backup_dir.join("recovery");
        if let Some(bytes) = &original.bytes {
            atomic_bytes(
                &directory.join(format!("{}.yaml", work.operation_id)),
                bytes,
                0o600,
            )
            .map_err(|_| TransactionError::internal("Cannot persist the recovery backup."))?;
        }
        #[cfg(unix)]
        let metadata = json!({
            "operation_id": work.operation_id,
            "existed": original.bytes.is_some(),
            "mode": original.mode,
            "uid": original.uid,
            "gid": original.gid,
            "etag": work.base_etag,
            "runtime_source_digest": work.base_runtime.source_digest,
            "runtime_effective_digest": work.base_runtime.effective_digest,
            "created_at": now()
        });
        #[cfg(not(unix))]
        let metadata = json!({
            "operation_id": work.operation_id,
            "existed": original.bytes.is_some(),
            "etag": work.base_etag,
            "runtime_source_digest": work.base_runtime.source_digest,
            "runtime_effective_digest": work.base_runtime.effective_digest,
            "created_at": now()
        });
        atomic_json(
            &directory.join(format!("{}.json", work.operation_id)),
            &metadata,
        )
        .map_err(|_| TransactionError::internal("Cannot persist recovery metadata."))
    }

    fn ensure_baseline_revision(
        &self,
        work: &TransactionWork,
        original: &OriginalConfig,
    ) -> Result<(), TransactionError> {
        let Some(bytes) = original.bytes.as_ref() else {
            return Ok(());
        };
        let validated = match validate_candidate(bytes) {
            Ok(validated) => validated,
            Err(_) => return Ok(()),
        };
        {
            let state = self
                .state
                .lock()
                .map_err(|_| TransactionError::internal("Revision state is unavailable."))?;
            if state
                .revisions
                .values()
                .any(|revision| revision.after_etag == work.base_etag)
            {
                return Ok(());
            }
        }
        let id = new_id();
        let revision = Revision {
            id: id.clone(),
            generation: work.base_runtime.generation.max(1),
            before_etag: work.base_etag.clone(),
            after_etag: work.base_etag.clone(),
            candidate_digest: validated.source_digest,
            actor: "system:baseline".to_owned(),
            comment: "Baseline captured before the first managed transaction.".to_owned(),
            created_at: now(),
            result: "baseline".to_owned(),
        };
        self.persist_revision(&revision, bytes)
    }

    fn save_revision(
        &self,
        work: &TransactionWork,
        generation: u64,
        after_etag: &str,
    ) -> Result<(), TransactionError> {
        let revision = Revision {
            id: new_id(),
            generation,
            before_etag: work.base_etag.clone(),
            after_etag: after_etag.to_owned(),
            candidate_digest: work.candidate_digest.clone(),
            actor: work.caller.actor.clone(),
            comment: work.comment.clone(),
            created_at: now(),
            result: "succeeded".to_owned(),
        };
        self.persist_revision(&revision, &work.candidate)?;
        if let Err(error) = self.prune_revisions(after_etag) {
            hbb_common::log::warn!("Configuration revision retention failed: {error}");
        }
        Ok(())
    }

    fn persist_revision(
        &self,
        revision: &Revision,
        document: &[u8],
    ) -> Result<(), TransactionError> {
        atomic_bytes(
            &self.revisions_dir().join(format!("{}.yaml", revision.id)),
            document,
            0o600,
        )
        .map_err(|_| TransactionError::internal("Cannot persist the revision document."))?;
        atomic_json(
            &self.revisions_dir().join(format!("{}.json", revision.id)),
            revision,
        )
        .map_err(|_| TransactionError::internal("Cannot persist the revision manifest."))?;
        self.state
            .lock()
            .map_err(|_| TransactionError::internal("Revision state is unavailable."))?
            .revisions
            .insert(revision.id.clone(), revision.clone());
        Ok(())
    }

    fn prune_revisions(&self, current_etag: &str) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "revision state is unavailable".to_owned())?;
        let mut ordered: Vec<Revision> = state.revisions.values().cloned().collect();
        ordered.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        let protected_recent: Vec<String> = ordered
            .iter()
            .rev()
            .take(2)
            .map(|revision| revision.id.clone())
            .collect();
        let size = |revision: &Revision| -> u64 {
            [
                self.revisions_dir().join(format!("{}.json", revision.id)),
                self.revisions_dir().join(format!("{}.yaml", revision.id)),
            ]
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum()
        };
        let mut total_bytes: u64 = ordered.iter().map(&size).sum();
        let mut total_count = ordered.len();
        for revision in ordered {
            if total_count <= MAX_REVISIONS && total_bytes <= MAX_REVISION_BYTES {
                break;
            }
            if protected_recent.iter().any(|id| id == &revision.id)
                || revision.after_etag == current_etag
            {
                continue;
            }
            let revision_bytes = size(&revision);
            for path in [
                self.revisions_dir().join(format!("{}.json", revision.id)),
                self.revisions_dir().join(format!("{}.yaml", revision.id)),
            ] {
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(format!("cannot prune revision state: {error}"));
                    }
                }
            }
            state.revisions.remove(&revision.id);
            total_count = total_count.saturating_sub(1);
            total_bytes = total_bytes.saturating_sub(revision_bytes);
        }
        sync_directory(&self.revisions_dir())
    }

    fn verify_restored_bytes(&self, original: &OriginalConfig) -> Result<(), TransactionError> {
        match &original.bytes {
            Some(expected) => {
                let actual = fs::read(&self.config_path).map_err(|_| {
                    TransactionError::new(
                        500,
                        "ROLLBACK_FAILED",
                        "The restored configuration cannot be read.",
                        false,
                    )
                })?;
                if &actual != expected {
                    return Err(TransactionError::new(
                        500,
                        "ROLLBACK_FAILED",
                        "The restored configuration bytes do not match the recovery backup.",
                        false,
                    ));
                }
            }
            None if self.config_path.exists() => {
                return Err(TransactionError::new(
                    500,
                    "ROLLBACK_FAILED",
                    "The transaction created a configuration file that could not be removed.",
                    false,
                ))
            }
            None => {}
        }
        Ok(())
    }

    fn operation_path(&self, id: &str) -> PathBuf {
        self.backup_dir
            .join("operations")
            .join(format!("{id}.json"))
    }

    fn idempotency_path(&self, key_hash: &str) -> PathBuf {
        self.backup_dir
            .join("idempotency")
            .join(format!("{key_hash}.json"))
    }

    fn audit_path(&self, id: &str) -> PathBuf {
        self.backup_dir.join("audit").join(format!("{id}.json"))
    }

    fn revisions_dir(&self) -> PathBuf {
        self.backup_dir.join("revisions")
    }
}

fn validate_candidate(raw: &[u8]) -> Result<ValidatedConfig, TransactionError> {
    starry_config::parse_document(raw)
        .and_then(starry_config::validate_config)
        .map_err(|diagnostics| {
            let errors = serde_json::to_value(&diagnostics.errors)
                .ok()
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default();
            TransactionError::new(
                400,
                "CONFIG_INVALID",
                "The configuration candidate is invalid.",
                false,
            )
            .with_errors(errors)
        })
}

fn normalized_config(raw: &[u8]) -> Option<Value> {
    starry_config::parse_document(raw)
        .and_then(starry_config::validate_config)
        .ok()
        .and_then(|validated| serde_json::to_value(validated.config).ok())
}

fn collect_changes(
    pointer: &str,
    before: Option<&Value>,
    after: Option<&Value>,
    changes: &mut Vec<Value>,
) {
    if before == after {
        return;
    }
    match (before, after) {
        (Some(Value::Object(left)), Some(Value::Object(right))) => {
            let mut keys: Vec<&str> = left
                .keys()
                .chain(right.keys())
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            keys.dedup();
            for key in keys {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                let child = format!("{pointer}/{escaped}");
                collect_changes(&child, left.get(key), right.get(key), changes);
            }
        }
        (None, Some(_)) => changes.push(json!({"pointer": pointer, "kind": "add"})),
        (Some(_), None) => changes.push(json!({"pointer": pointer, "kind": "remove"})),
        _ => changes.push(json!({"pointer": pointer, "kind": "replace"})),
    }
}

fn classify_risk(changes: &[Value], before: Option<&Value>, after: &Value) -> String {
    let pointers: Vec<&str> = changes
        .iter()
        .filter_map(|change| change.get("pointer").and_then(Value::as_str))
        .collect();
    if pointers
        .iter()
        .any(|pointer| pointer.starts_with("/connection_auth/mode"))
    {
        return "critical".to_owned();
    }
    if pointers.iter().any(|pointer| {
        pointer.starts_with("/connection_auth")
            || pointer.starts_with("/websocket_signal/trusted_proxies")
    }) {
        return "high".to_owned();
    }
    if pointers
        .iter()
        .any(|pointer| pointer.starts_with("/relay_servers"))
    {
        let old_count = before
            .and_then(|value| value.get("relay_servers"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let new_count = after
            .get("relay_servers")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        return if old_count > 0 && new_count == 0 {
            "high"
        } else {
            "medium"
        }
        .to_owned();
    }
    if pointers.iter().any(|pointer| {
        is_relay_quality_pointer(pointer)
            || *pointer == "/fast_mode"
            || pointer.starts_with("/fast_mode/")
            || pointer.starts_with("/websocket_signal")
            || pointer.starts_with("/secure_tcp")
    }) {
        "medium".to_owned()
    } else {
        "low".to_owned()
    }
}

fn is_relay_quality_pointer(pointer: &str) -> bool {
    pointer == "/relay_quality" || pointer.starts_with("/relay_quality/")
}

fn validate_comment(comment: Option<&str>) -> Result<(), TransactionError> {
    if comment.is_some_and(|value| {
        value.len() > MAX_COMMENT_BYTES || value.chars().any(|character| character.is_control())
    }) {
        return Err(TransactionError::new(
            400,
            "REQUEST_INVALID",
            "comment must contain at most 500 bytes and no control characters.",
            false,
        ));
    }
    Ok(())
}

pub(super) fn validate_idempotency_key(value: &str) -> Result<(), TransactionError> {
    if !(16..=128).contains(&value.len())
        || !value.is_ascii()
        || value.chars().any(|character| character.is_ascii_control())
    {
        return Err(TransactionError::new(
            400,
            "REQUEST_INVALID",
            "Idempotency-Key must contain 16 to 128 printable ASCII bytes.",
            false,
        ));
    }
    Ok(())
}

pub(super) fn validate_digest(value: &str) -> Result<(), TransactionError> {
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(TransactionError::new(
            400,
            "REQUEST_INVALID",
            "A lowercase sha256 digest is required.",
            false,
        ));
    }
    Ok(())
}

pub(super) fn strong_etag(raw: &[u8]) -> String {
    format!("\"{}\"", digest(raw))
}

pub(super) fn digest(raw: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(raw))
}

fn optional_digest(value: Option<&Value>) -> Result<Option<String>, TransactionError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            validate_digest(value)?;
            Ok(Some(value.clone()))
        }
        _ => Err(TransactionError::internal(
            "HBBS returned an invalid runtime digest.",
        )),
    }
}

fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>, TransactionError> {
    Ok(read_regular_bounded(path, max_bytes)?
        .map(|(bytes, _)| bytes)
        .unwrap_or_default())
}

fn read_regular_bounded(
    path: &Path,
    max_bytes: usize,
) -> Result<Option<(Vec<u8>, fs::Metadata)>, TransactionError> {
    validate_managed_parent(path)?;
    let inspected = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(TransactionError::internal(
                "The managed configuration cannot be inspected.",
            ))
        }
    };
    if inspected.file_type().is_symlink() || !inspected.is_file() {
        return Err(unsafe_config_path());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if inspected.nlink() != 1 || inspected.mode() & 0o022 != 0 {
            return Err(unsafe_config_path());
        }
    }
    if inspected.len() > max_bytes as u64 {
        return Err(config_too_large());
    }

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            etag_mismatch()
        } else {
            TransactionError::internal("The managed configuration cannot be opened safely.")
        }
    })?;
    let opened = file.metadata().map_err(|_| {
        TransactionError::internal("The managed configuration cannot be inspected after open.")
    })?;
    if !opened.is_file() {
        return Err(unsafe_config_path());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.nlink() != 1
            || opened.mode() & 0o022 != 0
            || opened.dev() != inspected.dev()
            || opened.ino() != inspected.ino()
        {
            return Err(unsafe_config_path());
        }
    }
    if opened.len() > max_bytes as u64 {
        return Err(config_too_large());
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    Read::by_ref(&mut file)
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| TransactionError::internal("The managed configuration cannot be read."))?;
    if bytes.len() > max_bytes {
        return Err(config_too_large());
    }
    Ok(Some((bytes, opened)))
}

#[cfg(unix)]
fn validate_managed_parent(path: &Path) -> Result<fs::Metadata, TransactionError> {
    use std::os::unix::fs::MetadataExt;

    let parent = path.parent().ok_or_else(unsafe_config_path)?;
    let absolute = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| unsafe_config_path())?
            .join(parent)
    };
    let effective_uid = unsafe { libc::geteuid() };
    let effective_gid = unsafe { libc::getegid() };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|_| unsafe_config_path())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(unsafe_config_path());
        }
        if metadata.uid() != 0 && metadata.uid() != effective_uid {
            return Err(unsafe_config_path());
        }
        let mode = metadata.mode();
        if mode & 0o020 != 0
            && metadata.gid() != effective_gid
            && !(metadata.uid() == 0 && mode & 0o1000 != 0)
        {
            return Err(unsafe_config_path());
        }
        if mode & 0o002 != 0 && !(metadata.uid() == 0 && mode & 0o1000 != 0) {
            return Err(unsafe_config_path());
        }
    }
    fs::symlink_metadata(&absolute).map_err(|_| unsafe_config_path())
}

#[cfg(not(unix))]
fn validate_managed_parent(path: &Path) -> Result<fs::Metadata, TransactionError> {
    let parent = path.parent().ok_or_else(unsafe_config_path)?;
    let metadata = fs::symlink_metadata(parent).map_err(|_| unsafe_config_path())?;
    if !metadata.is_dir() {
        return Err(unsafe_config_path());
    }
    Ok(metadata)
}

fn unsafe_config_path() -> TransactionError {
    TransactionError::new(
        400,
        "CONFIG_INVALID",
        "The managed configuration path and parent chain must be confined, and the file must be regular, single-link, and not writable by group or other users.",
        false,
    )
}

fn config_too_large() -> TransactionError {
    TransactionError::new(
        413,
        "CONFIG_TOO_LARGE",
        "The managed configuration exceeds the configured byte limit.",
        false,
    )
}

fn request_digest(value: &Value) -> Result<String, TransactionError> {
    serde_json::to_vec(value)
        .map(|bytes| digest(&bytes))
        .map_err(|_| TransactionError::internal("Cannot calculate the request digest."))
}

fn idempotency_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn caller_digest(caller: &TransactionCaller) -> String {
    let mut hasher = Sha256::new();
    for value in [
        caller.service.as_bytes(),
        caller.actor.as_bytes(),
        caller.certificate_uri_san.as_bytes(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    format!("{:x}", hasher.finalize())
}

fn operation_is_terminal(operation: &Operation) -> bool {
    !matches!(
        operation.state.as_str(),
        "pending" | "running" | "manual_intervention_required"
    )
}

fn timestamp_before(value: &str, cutoff: i64) -> Result<bool, TransactionError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp() < cutoff)
        .map_err(|_| TransactionError::internal("Durable state contains an invalid timestamp."))
}

fn durable_state_bytes(root: &Path) -> Result<u64, TransactionError> {
    let mut total = 0_u64;
    for name in ["operations", "idempotency", "audit", "recovery"] {
        let directory = root.join(name);
        for entry in fs::read_dir(&directory)
            .map_err(|_| TransactionError::internal("Cannot inspect durable transaction state."))?
        {
            let entry = entry.map_err(|_| {
                TransactionError::internal("Cannot inspect durable transaction state.")
            })?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|_| {
                TransactionError::internal("Cannot inspect durable transaction state.")
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(TransactionError::internal(
                    "Durable transaction state contains a non-regular entry.",
                ));
            }
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

fn remove_if_present(path: &Path) -> Result<(), TransactionError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(TransactionError::internal(
            "Cannot prune expired durable transaction state.",
        )),
    }
}

fn etag_mismatch() -> TransactionError {
    TransactionError::new(
        412,
        "CONFIG_ETAG_MISMATCH",
        "The configuration changed after the plan was created.",
        false,
    )
}

fn idempotency_reused() -> TransactionError {
    TransactionError::new(
        409,
        "IDEMPOTENCY_KEY_REUSED",
        "The idempotency key was already used with a different request.",
        false,
    )
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn problem_value(
    code: &str,
    status: u16,
    detail: &str,
    retryable: bool,
    request_id: &str,
) -> Value {
    json!({
        "type": format!("https://starry.invalid/problems/{}", code.to_ascii_lowercase().replace('_', "-")),
        "title": problem_title(code),
        "status": status,
        "code": code,
        "detail": detail,
        "request_id": request_id,
        "retryable": retryable,
        "errors": []
    })
}

fn problem_title(code: &str) -> &'static str {
    match code {
        "CONFIG_ETAG_MISMATCH" => "Configuration changed",
        "PLAN_EXPIRED" => "Plan expired",
        "PLAN_STALE" => "Plan stale",
        "OPERATION_IN_PROGRESS" => "Operation in progress",
        "IDEMPOTENCY_KEY_REUSED" => "Idempotency key reused",
        "ROLLBACK_FAILED" => "Rollback failed",
        _ => "Configuration transaction failed",
    }
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    let mut missing = Vec::new();
    let mut cursor = path;
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(_) => break,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                missing.push(cursor.to_path_buf());
                cursor = cursor.parent().ok_or_else(|| {
                    "Control Agent state path has no existing ancestor".to_owned()
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "cannot inspect Control Agent state directory: {error}"
                ))
            }
        }
    }
    validate_state_ancestor_chain(cursor)?;
    for directory in missing.iter().rev() {
        fs::create_dir(directory)
            .map_err(|error| format!("cannot create Control Agent state directory: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("cannot secure Control Agent state directory: {error}"))?;
        }
    }
    validate_private_state_directory(path)
}

#[cfg(unix)]
fn validate_state_ancestor_chain(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("cannot resolve Control Agent state path: {error}"))?
            .join(path)
    };
    let effective_uid = unsafe { libc::geteuid() };
    let effective_gid = unsafe { libc::getegid() };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("cannot inspect Control Agent state ancestor: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Control Agent state ancestors must be real directories".to_owned());
        }
        if metadata.uid() != 0 && metadata.uid() != effective_uid {
            return Err(
                "Control Agent state ancestors must be owned by root or the Agent".to_owned(),
            );
        }
        let mode = metadata.mode();
        if mode & 0o020 != 0
            && metadata.gid() != effective_gid
            && !(metadata.uid() == 0 && mode & 0o1000 != 0)
        {
            return Err(
                "Control Agent state ancestors must not be writable by another group".to_owned(),
            );
        }
        if mode & 0o002 != 0 && !(metadata.uid() == 0 && mode & 0o1000 != 0) {
            return Err("Control Agent state ancestors must not be world-writable".to_owned());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_state_ancestor_chain(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect Control Agent state ancestor: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Control Agent state ancestors must be real directories".to_owned());
    }
    Ok(())
}

#[cfg(unix)]
fn validate_private_state_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect Control Agent state directory: {error}"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || !same_owner(
            metadata.uid(),
            metadata.gid(),
            unsafe { libc::geteuid() },
            unsafe { libc::getegid() },
        )
        || metadata.mode() & 0o077 != 0
    {
        return Err(
            "Control Agent state directories must be Agent-owned real directories with mode 0700"
                .to_owned(),
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_state_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect Control Agent state directory: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Control Agent state directories must be real directories".to_owned());
    }
    Ok(())
}

fn load_records<T>(directory: &Path) -> Result<HashMap<String, T>, String>
where
    T: DeserializeOwned + DurableRecord,
{
    let mut records = HashMap::new();
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("cannot read Control Agent state directory: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("cannot inspect Control Agent state: {error}"))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if records.len() >= MAX_DURABLE_RECORDS_PER_DIRECTORY {
            return Err("Control Agent durable state exceeds the record-count limit".to_owned());
        }
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "Control Agent state contains an invalid file name".to_owned())?;
        if !valid_durable_key(stem) {
            return Err("Control Agent state contains an unsafe record name".to_owned());
        }
        let raw = read_private_state_file(&path, MAX_DURABLE_JSON_BYTES)?;
        let value: T = serde_json::from_slice(&raw)
            .map_err(|error| format!("invalid durable Control Agent state: {error}"))?;
        if value.durable_key() != stem {
            return Err(
                "Control Agent durable record identity does not match its file name".to_owned(),
            );
        }
        records.insert(stem.to_owned(), value);
    }
    Ok(records)
}

fn valid_durable_key(value: &str) -> bool {
    (16..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn read_private_state_file(path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
    let inspected = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect Control Agent state: {error}"))?;
    if inspected.file_type().is_symlink()
        || !inspected.is_file()
        || inspected.len() > max_bytes as u64
    {
        return Err("Control Agent durable state file is unsafe or too large".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        if inspected.nlink() != 1
            || inspected.mode() & 0o077 != 0
            || !same_owner(
                inspected.uid(),
                inspected.gid(),
                unsafe { libc::geteuid() },
                unsafe { libc::getegid() },
            )
        {
            return Err(
                "Control Agent durable state file must be Agent-owned, single-link, and private"
                    .to_owned(),
            );
        }
        let mut options = OpenOptions::new();
        options.read(true).custom_flags(libc::O_NOFOLLOW);
        let mut file = options
            .open(path)
            .map_err(|error| format!("cannot open Control Agent state safely: {error}"))?;
        let opened = file
            .metadata()
            .map_err(|error| format!("cannot inspect opened Control Agent state: {error}"))?;
        if opened.dev() != inspected.dev() || opened.ino() != inspected.ino() {
            return Err("Control Agent durable state changed while being opened".to_owned());
        }
        let mut raw = Vec::with_capacity(opened.len() as usize);
        Read::by_ref(&mut file)
            .take(max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut raw)
            .map_err(|error| format!("cannot read Control Agent state: {error}"))?;
        if raw.len() > max_bytes {
            return Err("Control Agent durable state file exceeds the byte limit".to_owned());
        }
        return Ok(raw);
    }
    #[cfg(not(unix))]
    {
        fs::read(path).map_err(|error| format!("cannot read Control Agent state: {error}"))
    }
}

fn atomic_json(path: &Path, value: &impl SerializeTrait) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|error| format!("cannot serialize durable state: {error}"))?;
    bytes.push(b'\n');
    atomic_bytes(path, &bytes, 0o600)
}

fn atomic_bytes(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "durable state path has no parent".to_owned())?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|v| v.to_str()).unwrap_or("state"),
        new_id()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let result = (|| -> Result<(), String> {
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("cannot create durable temporary file: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("cannot write durable temporary file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("cannot fsync durable temporary file: {error}"))?;
        drop(file);
        fs::rename(&temporary, path)
            .map_err(|error| format!("cannot atomically publish durable state: {error}"))?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn atomic_replace_config(
    path: &Path,
    bytes: &[u8],
    original: &OriginalConfig,
) -> Result<(), WriteFailure> {
    atomic_replace_config_with_fault(path, bytes, original, None)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
enum ConfigWriteFault {
    TemporaryCreate,
    Write,
    FileSync,
    Rename,
    DirectorySync,
}

fn atomic_replace_config_with_fault(
    path: &Path,
    bytes: &[u8],
    original: &OriginalConfig,
    fail_at: Option<ConfigWriteFault>,
) -> Result<(), WriteFailure> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let parent_metadata = match validate_managed_parent(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Err(WriteFailure {
                error,
                may_have_changed: false,
            })
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if parent_metadata.dev() != original.parent_dev
            || parent_metadata.ino() != original.parent_ino
        {
            return Err(WriteFailure {
                error: unsafe_config_path(),
                may_have_changed: false,
            });
        }
    }
    if !parent_metadata.is_dir() {
        return Err(WriteFailure {
            error: unsafe_config_path(),
            may_have_changed: false,
        });
    }
    let temporary = parent.join(format!(".starry-config.{}.tmp", new_id()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(original.mode & 0o7777);
    }
    let mut renamed = false;
    let result = (|| -> Result<(), TransactionError> {
        if fail_at == Some(ConfigWriteFault::TemporaryCreate) {
            return Err(TransactionError::internal(
                "Injected configuration temporary-file creation failure.",
            ));
        }
        let mut file = options.open(&temporary).map_err(|_| {
            TransactionError::internal("Cannot create the configuration temporary file.")
        })?;
        if fail_at == Some(ConfigWriteFault::Write) {
            return Err(TransactionError::internal(
                "Injected configuration temporary-file write failure.",
            ));
        }
        file.write_all(bytes).map_err(|_| {
            TransactionError::internal("Cannot write the configuration temporary file.")
        })?;
        #[cfg(unix)]
        if original.bytes.is_some() {
            preserve_owner(&file, original)?;
        }
        if fail_at == Some(ConfigWriteFault::FileSync) {
            return Err(TransactionError::internal(
                "Injected configuration file fsync failure.",
            ));
        }
        file.sync_all().map_err(|_| {
            TransactionError::internal("Cannot fsync the configuration temporary file.")
        })?;
        drop(file);
        if fail_at == Some(ConfigWriteFault::Rename) {
            return Err(TransactionError::internal(
                "Injected configuration rename failure.",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let current_parent = validate_managed_parent(path)?;
            if current_parent.dev() != original.parent_dev
                || current_parent.ino() != original.parent_ino
            {
                return Err(unsafe_config_path());
            }
        }
        fs::rename(&temporary, path).map_err(|_| {
            TransactionError::internal("Cannot atomically replace the configuration.")
        })?;
        renamed = true;
        if fail_at == Some(ConfigWriteFault::DirectorySync) {
            return Err(TransactionError::internal(
                "Injected configuration directory fsync failure.",
            ));
        }
        sync_directory(parent)
            .map_err(|_| TransactionError::internal("Cannot fsync the configuration directory."))
    })();
    if result.is_err() && !renamed {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| WriteFailure {
        error,
        may_have_changed: renamed,
    })
}

fn restore_config(path: &Path, original: &OriginalConfig) -> Result<(), TransactionError> {
    match &original.bytes {
        Some(bytes) => atomic_replace_config(path, bytes, original).map_err(|failure| {
            TransactionError::new(500, "ROLLBACK_FAILED", failure.error.detail, false)
        }),
        None => {
            if path.exists() {
                fs::remove_file(path).map_err(|_| {
                    TransactionError::new(
                        500,
                        "ROLLBACK_FAILED",
                        "Cannot remove the configuration created by the failed transaction.",
                        false,
                    )
                })?;
                sync_directory(path.parent().unwrap_or_else(|| Path::new("."))).map_err(|_| {
                    TransactionError::new(
                        500,
                        "ROLLBACK_FAILED",
                        "Cannot fsync the configuration directory during recovery.",
                        false,
                    )
                })?;
            }
            Ok(())
        }
    }
}

#[cfg(unix)]
fn preserve_owner(file: &File, original: &OriginalConfig) -> Result<(), TransactionError> {
    use std::os::unix::io::AsRawFd;
    if same_owner(
        original.uid,
        original.gid,
        unsafe { libc::geteuid() },
        unsafe { libc::getegid() },
    ) {
        return Ok(());
    }
    // The transaction runs with the Agent's configured filesystem authority. Keeping the
    // previous uid/gid prevents an atomic rename from silently changing ownership.
    let result = unsafe { libc::fchown(file.as_raw_fd(), original.uid, original.gid) };
    if result == 0 {
        Ok(())
    } else {
        Err(TransactionError::internal(
            "Cannot preserve configuration file ownership.",
        ))
    }
}

#[cfg(unix)]
fn same_owner(owner_uid: u32, owner_gid: u32, effective_uid: u32, effective_gid: u32) -> bool {
    owner_uid == effective_uid && owner_gid == effective_gid
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("cannot fsync directory: {error}"))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "starry-config-store-{name}-{}-{}",
                std::process::id(),
                new_id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn every_relay_quality_change_is_at_least_medium_risk() {
        let relay_quality = json!({
            "enabled": false,
            "strategy": "adaptive",
            "weights": {"rtt": 4000, "jitter": 2000, "loss": 2500, "load": 1500}
        });
        let cases = [
            (
                json!({}),
                json!({"relay_quality": relay_quality.clone()}),
                "/relay_quality",
                "add",
            ),
            (
                json!({"relay_quality": relay_quality.clone()}),
                json!({}),
                "/relay_quality",
                "remove",
            ),
            (
                json!({"relay_quality": relay_quality.clone()}),
                json!({"relay_quality": {
                    "enabled": true,
                    "strategy": "adaptive",
                    "weights": {"rtt": 4000, "jitter": 2000, "loss": 2500, "load": 1500}
                }}),
                "/relay_quality/enabled",
                "replace",
            ),
            (
                json!({"relay_quality": relay_quality}),
                json!({"relay_quality": {
                    "enabled": false,
                    "strategy": "adaptive",
                    "weights": {"rtt": 3500, "jitter": 2500, "loss": 2500, "load": 1500}
                }}),
                "/relay_quality/weights/rtt",
                "replace",
            ),
        ];

        for (before, after, expected_pointer, expected_kind) in cases {
            let mut changes = Vec::new();
            collect_changes("", Some(&before), Some(&after), &mut changes);
            assert!(changes.iter().any(|change| {
                change["pointer"] == expected_pointer && change["kind"] == expected_kind
            }));
            assert_eq!(classify_risk(&changes, Some(&before), &after), "medium");
        }

        let unrelated = vec![json!({
            "pointer": "/relay_quality_backup/enabled",
            "kind": "replace"
        })];
        assert_eq!(classify_risk(&unrelated, None, &json!({})), "low");
    }

    #[test]
    fn atomic_replace_and_recovery_preserve_exact_bytes_and_mode() {
        let root = TestDirectory::new("atomic");
        let config_path = root.0.join("config.yaml");
        fs::write(&config_path, b"version: 3\nrelay_servers: []\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&config_path, fs::Permissions::from_mode(0o640)).unwrap();
        }
        let manager = ConfigTransactions::open(
            new_id(),
            config_path.clone(),
            root.0.join("history"),
            1024 * 1024,
        )
        .unwrap();
        let original = manager.read_original().unwrap();
        let replacement = b"version: 3\nrelay_servers:\n  - relay.example.test:21117\n";
        atomic_replace_config(&config_path, replacement, &original).unwrap();
        assert_eq!(fs::read(&config_path).unwrap(), replacement);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(fs::metadata(&config_path).unwrap().mode() & 0o777, 0o640);
        }
        restore_config(&config_path, &original).unwrap();
        manager.verify_restored_bytes(&original).unwrap();
        assert_eq!(
            fs::read(&config_path).unwrap(),
            b"version: 3\nrelay_servers: []\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_authority_requires_the_effective_uid_and_primary_gid() {
        use std::os::unix::fs::PermissionsExt;

        assert!(same_owner(1000, 1000, 1000, 1000));
        assert!(!same_owner(0, 1000, 1000, 1000));
        assert!(!same_owner(1000, 2000, 1000, 1000));

        let root = TestDirectory::new("write-authority");
        let config_path = root.0.join("config.yaml");
        fs::write(&config_path, b"version: 3\n").unwrap();
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o640)).unwrap();
        let manager =
            ConfigTransactions::open(new_id(), config_path, root.0.join("history"), 1024 * 1024)
                .unwrap();
        manager.validate_write_authority().unwrap();
    }

    #[test]
    fn every_atomic_write_failure_stage_is_classified_and_recoverable() {
        let root = TestDirectory::new("write-faults");
        let config_path = root.0.join("config.yaml");
        let original_bytes = b"version: 3\nrelay_servers: []\n";
        let candidate = b"version: 3\nrelay_servers: [relay.example.test:21117]\n";
        fs::write(&config_path, original_bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&config_path, fs::Permissions::from_mode(0o640)).unwrap();
        }
        let manager = ConfigTransactions::open(
            new_id(),
            config_path.clone(),
            root.0.join("history"),
            1024 * 1024,
        )
        .unwrap();
        let original = manager.read_original().unwrap();

        for stage in [
            ConfigWriteFault::TemporaryCreate,
            ConfigWriteFault::Write,
            ConfigWriteFault::FileSync,
            ConfigWriteFault::Rename,
        ] {
            let failure =
                atomic_replace_config_with_fault(&config_path, candidate, &original, Some(stage))
                    .unwrap_err();
            assert!(!failure.may_have_changed, "stage {stage:?}");
            assert_eq!(
                fs::read(&config_path).unwrap(),
                original_bytes,
                "stage {stage:?}"
            );
            assert!(fs::read_dir(&root.0).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            }));
        }

        let failure = atomic_replace_config_with_fault(
            &config_path,
            candidate,
            &original,
            Some(ConfigWriteFault::DirectorySync),
        )
        .unwrap_err();
        assert!(failure.may_have_changed);
        assert_eq!(fs::read(&config_path).unwrap(), candidate);
        restore_config(&config_path, &original).unwrap();
        manager.verify_restored_bytes(&original).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn managed_config_rejects_symbolic_and_hard_links() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = TestDirectory::new("linked-config");
        let target = root.0.join("target.yaml");
        fs::write(&target, b"version: 3\n").unwrap();
        let symlink_path = root.0.join("symlink.yaml");
        symlink(&target, &symlink_path).unwrap();
        assert_eq!(
            read_bounded(&symlink_path, 1024).unwrap_err().code,
            "CONFIG_INVALID"
        );

        let hardlink_path = root.0.join("hardlink.yaml");
        fs::hard_link(&target, &hardlink_path).unwrap();
        assert_eq!(
            read_bounded(&hardlink_path, 1024).unwrap_err().code,
            "CONFIG_INVALID"
        );

        let real_parent = root.0.join("real-parent");
        fs::create_dir(&real_parent).unwrap();
        let nested = real_parent.join("config.yaml");
        fs::write(&nested, b"version: 3\n").unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o640)).unwrap();
        let linked_parent = root.0.join("linked-parent");
        symlink(&real_parent, &linked_parent).unwrap();
        assert_eq!(
            read_bounded(&linked_parent.join("config.yaml"), 1024)
                .unwrap_err()
                .code,
            "CONFIG_INVALID"
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_replace_rejects_a_changed_parent_identity() {
        use std::os::unix::fs::PermissionsExt;

        let root = TestDirectory::new("parent-identity");
        let managed = root.0.join("managed");
        fs::create_dir(&managed).unwrap();
        let config_path = managed.join("config.yaml");
        fs::write(&config_path, b"version: 3\n").unwrap();
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o640)).unwrap();
        let manager =
            ConfigTransactions::open(new_id(), config_path.clone(), root.0.join("history"), 1024)
                .unwrap();
        let original = manager.read_original().unwrap();

        let moved = root.0.join("managed-original");
        fs::rename(&managed, &moved).unwrap();
        fs::create_dir(&managed).unwrap();
        fs::write(&config_path, b"replacement-parent\n").unwrap();
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o640)).unwrap();
        let failure = atomic_replace_config(&config_path, b"version: 3\n", &original).unwrap_err();
        assert!(!failure.may_have_changed);
        assert_eq!(failure.error.code, "CONFIG_INVALID");
        assert_eq!(fs::read(&config_path).unwrap(), b"replacement-parent\n");
    }

    #[cfg(unix)]
    #[test]
    fn durable_state_rejects_linked_directories_and_mismatched_record_identity() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = TestDirectory::new("state-confinement");
        let redirected = root.0.join("redirected");
        fs::create_dir(&redirected).unwrap();
        fs::set_permissions(&redirected, fs::Permissions::from_mode(0o700)).unwrap();
        let linked_history = root.0.join("linked-history");
        symlink(&redirected, &linked_history).unwrap();
        assert!(ConfigTransactions::open(
            new_id(),
            root.0.join("config.yaml"),
            linked_history,
            1024,
        )
        .is_err());

        let history = root.0.join("history");
        for directory in [
            history.clone(),
            history.join("operations"),
            history.join("idempotency"),
            history.join("revisions"),
            history.join("audit"),
            history.join("recovery"),
        ] {
            fs::create_dir_all(&directory).unwrap();
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let operation = Operation {
            id: new_id(),
            audit_id: None,
            kind: "config_apply".to_owned(),
            state: "succeeded".to_owned(),
            created_at: now(),
            updated_at: now(),
            activation_ack: None,
            error: None,
        };
        atomic_json(
            &history
                .join("operations")
                .join(format!("{}.json", new_id())),
            &operation,
        )
        .unwrap();
        let error =
            match ConfigTransactions::open(new_id(), root.0.join("config.yaml"), history, 1024) {
                Ok(_) => panic!("mismatched durable identity was accepted"),
                Err(error) => error,
            };
        assert!(error.contains("identity does not match"));
    }

    #[cfg(unix)]
    #[test]
    fn expired_terminal_transaction_state_is_pruned_on_open() {
        use std::os::unix::fs::PermissionsExt;

        let root = TestDirectory::new("retention");
        let history = root.0.join("history");
        for directory in [
            history.clone(),
            history.join("operations"),
            history.join("idempotency"),
            history.join("revisions"),
            history.join("audit"),
            history.join("recovery"),
        ] {
            fs::create_dir_all(&directory).unwrap();
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let old = (Utc::now() - ChronoDuration::days(2)).to_rfc3339();
        let operation_id = new_id();
        let audit_id = new_id();
        let operation = Operation {
            id: operation_id.clone(),
            audit_id: Some(audit_id.clone()),
            kind: "config_apply".to_owned(),
            state: "succeeded".to_owned(),
            created_at: old.clone(),
            updated_at: old.clone(),
            activation_ack: Some(json!({"accepted": true})),
            error: None,
        };
        atomic_json(
            &history
                .join("operations")
                .join(format!("{operation_id}.json")),
            &operation,
        )
        .unwrap();
        let key_hash = idempotency_hash("retention-test-key");
        let idempotency = IdempotencyRecord {
            key_hash: key_hash.clone(),
            instance_id: new_id(),
            caller_digest: digest(b"caller"),
            request_digest: digest(b"request"),
            kind: "config_apply".to_owned(),
            created_at: old.clone(),
            operation_id: Some(operation_id.clone()),
            response: None,
            error: None,
        };
        atomic_json(
            &history.join("idempotency").join(format!("{key_hash}.json")),
            &idempotency,
        )
        .unwrap();
        let audit = AuditRecord {
            audit_id: audit_id.clone(),
            operation_id: operation_id.clone(),
            request_id: new_id(),
            instance_id: new_id(),
            action: "config_apply".to_owned(),
            actor: "test-admin".to_owned(),
            service: "test-service".to_owned(),
            certificate_uri_san: "spiffe://test/service".to_owned(),
            traceparent: None,
            before_etag: strong_etag(b"before"),
            after_etag: Some(strong_etag(b"after")),
            generation: Some(2),
            candidate_digest: digest(b"candidate"),
            result: "succeeded".to_owned(),
            error_code: None,
            recovery_result: Some("not_required".to_owned()),
            idempotency_key_hash: key_hash.clone(),
            comment: String::new(),
            created_at: old.clone(),
            updated_at: old,
        };
        atomic_json(
            &history.join("audit").join(format!("{audit_id}.json")),
            &audit,
        )
        .unwrap();
        atomic_bytes(
            &history
                .join("recovery")
                .join(format!("{operation_id}.yaml")),
            b"version: 3\n",
            0o600,
        )
        .unwrap();
        atomic_json(
            &history
                .join("recovery")
                .join(format!("{operation_id}.json")),
            &json!({"operation_id": operation_id}),
        )
        .unwrap();

        let manager =
            ConfigTransactions::open(new_id(), root.0.join("config.yaml"), history.clone(), 1024)
                .unwrap();
        assert_eq!(manager.operation(&operation.id).unwrap_err().status, 404);
        for path in [
            history
                .join("operations")
                .join(format!("{}.json", operation.id)),
            history.join("idempotency").join(format!("{key_hash}.json")),
            history.join("audit").join(format!("{audit_id}.json")),
            history
                .join("recovery")
                .join(format!("{}.yaml", operation.id)),
            history
                .join("recovery")
                .join(format!("{}.json", operation.id)),
        ] {
            assert!(
                !path.exists(),
                "expired durable record remained: {}",
                path.display()
            );
        }
    }

    #[test]
    fn restart_marks_unresolved_durable_work_for_manual_intervention() {
        let root = TestDirectory::new("restart");
        let history = root.0.join("history");
        for directory in [
            history.clone(),
            history.join("operations"),
            history.join("idempotency"),
            history.join("revisions"),
            history.join("audit"),
            history.join("recovery"),
        ] {
            fs::create_dir_all(&directory).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let operation_id = new_id();
        let operation = Operation {
            id: operation_id.clone(),
            audit_id: Some(new_id()),
            kind: "config_apply".to_owned(),
            state: "running".to_owned(),
            created_at: now(),
            updated_at: now(),
            activation_ack: None,
            error: None,
        };
        atomic_json(
            &history
                .join("operations")
                .join(format!("{operation_id}.json")),
            &operation,
        )
        .unwrap();
        let audit = AuditRecord {
            audit_id: new_id(),
            operation_id: operation_id.clone(),
            request_id: new_id(),
            instance_id: new_id(),
            action: "config_apply".to_owned(),
            actor: "test-admin".to_owned(),
            service: "test-service".to_owned(),
            certificate_uri_san: "spiffe://test/service".to_owned(),
            traceparent: Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_owned()),
            before_etag: strong_etag(b"before"),
            after_etag: None,
            generation: None,
            candidate_digest: digest(b"candidate"),
            result: "running".to_owned(),
            error_code: None,
            recovery_result: None,
            idempotency_key_hash: idempotency_hash("1234567890123456"),
            comment: String::new(),
            created_at: now(),
            updated_at: now(),
        };
        atomic_json(
            &history
                .join("audit")
                .join(format!("{}.json", audit.audit_id)),
            &audit,
        )
        .unwrap();
        let reload_key_hash = idempotency_hash("restart-reload-key");
        let pending_reload = IdempotencyRecord {
            key_hash: reload_key_hash.clone(),
            instance_id: new_id(),
            caller_digest: digest(b"restart-test-caller"),
            request_digest: digest(b"reload-request"),
            kind: "runtime_reload".to_owned(),
            created_at: now(),
            operation_id: None,
            response: None,
            error: None,
        };
        atomic_json(
            &history
                .join("idempotency")
                .join(format!("{reload_key_hash}.json")),
            &pending_reload,
        )
        .unwrap();

        let manager = ConfigTransactions::open(
            new_id(),
            root.0.join("config.yaml"),
            history.clone(),
            1024 * 1024,
        )
        .unwrap();
        let recovered = manager.operation(&operation_id).unwrap();
        assert_eq!(recovered.state, "manual_intervention_required");
        assert_eq!(recovered.error.unwrap()["code"], "ROLLBACK_FAILED");
        assert_eq!(
            manager.ensure_writable().unwrap_err().code,
            "ROLLBACK_FAILED"
        );
        let reload: IdempotencyRecord = serde_json::from_slice(
            &fs::read(
                history
                    .join("idempotency")
                    .join(format!("{reload_key_hash}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(reload.error.unwrap().code, "ROLLBACK_FAILED");
        let updated_audit: AuditRecord = serde_json::from_slice(
            &fs::read(
                history
                    .join("audit")
                    .join(format!("{}.json", audit.audit_id)),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(updated_audit.result, "manual_intervention_required");
        assert_eq!(
            updated_audit.recovery_result.as_deref(),
            Some("agent_restarted")
        );
    }
}

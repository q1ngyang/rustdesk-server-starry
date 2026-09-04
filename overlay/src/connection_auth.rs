use crate::starry_config::{
    self, ConnectionAuthConfig, ConnectionAuthMode, IntrospectionConfig, SubsystemAck,
};
use base64::{decode_config, encode_config, URL_SAFE_NO_PAD};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use once_cell::sync::Lazy;
use serde_derive::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const INTROSPECTION_RESPONSE_LIMIT: usize = 64 * 1024;
const MAX_INTROSPECTION_CONCURRENCY: usize = 64;

static ACTIVE: Lazy<RwLock<Arc<AuthRuntime>>> =
    Lazy::new(|| RwLock::new(Arc::new(AuthRuntime::disabled())));
static METRICS: AuthMetrics = AuthMetrics::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectionAttemptKind {
    PunchHole,
    RequestRelay,
    FastMediaRenewal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SignalTransport {
    UnsupportedUdp,
    Tcp,
    SecureTcp,
    WebSocket,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AuthDecision {
    pub(crate) proceed: bool,
    pub(crate) verdict: &'static str,
    pub(crate) reason: &'static str,
    pub(crate) mode: ConnectionAuthMode,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AuthStatus {
    pub(crate) configured_mode: ConnectionAuthMode,
    pub(crate) effective_mode: ConnectionAuthMode,
    pub(crate) verifier_state: &'static str,
    pub(crate) key_count: usize,
    pub(crate) key_age_seconds: Option<u64>,
    pub(crate) metrics: AuthMetricsSnapshot,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AuthMetricsSnapshot {
    pub(crate) attempts: u64,
    pub(crate) allowed: u64,
    pub(crate) denied: u64,
    pub(crate) audit_would_deny: u64,
    pub(crate) cache_hits: u64,
    pub(crate) introspection_requests: u64,
    pub(crate) introspection_failures: u64,
}

pub(crate) struct PreparedAuth {
    runtime: Arc<AuthRuntime>,
}

struct AuthRuntime {
    configured_mode: ConnectionAuthMode,
    effective_mode: ConnectionAuthMode,
    config: ConnectionAuthConfig,
    keys: RwLock<KeyState>,
    jwks_client: reqwest::Client,
    jwks_file: Option<PathBuf>,
    introspection: Option<IntrospectionRuntime>,
    cancelled: AtomicBool,
}

struct KeyState {
    keys: HashMap<String, DecodingKey>,
    loaded_at_epoch_seconds: Option<u64>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JwksCacheMetadata {
    version: u8,
    fetched_at_epoch_seconds: u64,
    jwks_sha256: String,
}

struct IntrospectionRuntime {
    client: reqwest::Client,
    config: IntrospectionConfig,
    cache: Mutex<IntrospectionCache>,
    permits: Arc<hbb_common::tokio::sync::Semaphore>,
}

#[derive(Default)]
struct IntrospectionCache {
    entries: HashMap<[u8; 32], CacheEntry>,
    sequence: u64,
}

struct CacheEntry {
    active: bool,
    reason: &'static str,
    expires_at: Instant,
    sequence: u64,
}

#[derive(Debug)]
struct LocalClaims {
    subject: String,
    expires_at: u64,
}

#[derive(Debug, Deserialize)]
struct Claims {
    iss: String,
    aud: Audience,
    token_use: String,
    scope: Scope,
    sub: String,
    user_id: u64,
    auth_version: u64,
    jti: String,
    iat: u64,
    nbf: u64,
    exp: u64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Scope {
    Text(String),
    Values(Vec<String>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JwksDocument {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Jwk {
    kty: String,
    crv: String,
    #[serde(rename = "use")]
    key_use: String,
    alg: String,
    kid: String,
    x: String,
    #[serde(default)]
    key_ops: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct IntrospectionResponse {
    active: bool,
    #[serde(default)]
    sub: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

struct AuthMetrics {
    attempts: AtomicU64,
    allowed: AtomicU64,
    denied: AtomicU64,
    audit_would_deny: AtomicU64,
    cache_hits: AtomicU64,
    introspection_requests: AtomicU64,
    introspection_failures: AtomicU64,
}

impl AuthMetrics {
    const fn new() -> Self {
        Self {
            attempts: AtomicU64::new(0),
            allowed: AtomicU64::new(0),
            denied: AtomicU64::new(0),
            audit_would_deny: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            introspection_requests: AtomicU64::new(0),
            introspection_failures: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> AuthMetricsSnapshot {
        AuthMetricsSnapshot {
            attempts: self.attempts.load(Ordering::Relaxed),
            allowed: self.allowed.load(Ordering::Relaxed),
            denied: self.denied.load(Ordering::Relaxed),
            audit_would_deny: self.audit_would_deny.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            introspection_requests: self.introspection_requests.load(Ordering::Relaxed),
            introspection_failures: self.introspection_failures.load(Ordering::Relaxed),
        }
    }
}

impl Audience {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::One(value) => value == expected,
            Self::Many(values) => values.iter().any(|value| value == expected),
        }
    }
}

impl Scope {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::Text(value) => value
                .split_ascii_whitespace()
                .any(|value| value == expected),
            Self::Values(values) => values.iter().any(|value| value == expected),
        }
    }
}

impl AuthRuntime {
    fn disabled() -> Self {
        Self {
            configured_mode: ConnectionAuthMode::Off,
            effective_mode: ConnectionAuthMode::Off,
            config: ConnectionAuthConfig::default(),
            keys: RwLock::new(KeyState {
                keys: HashMap::new(),
                loaded_at_epoch_seconds: None,
            }),
            jwks_client: default_jwks_client(),
            jwks_file: None,
            introspection: None,
            cancelled: AtomicBool::new(false),
        }
    }

    fn key_state(&self) -> (usize, Option<u64>, bool) {
        let Ok(keys) = self.keys.read() else {
            return (0, None, true);
        };
        let age = keys
            .loaded_at_epoch_seconds
            .and_then(|loaded| epoch_seconds().checked_sub(loaded));
        let stale = age
            .map(|age| age > self.config.jwks.max_stale_seconds)
            .unwrap_or(true);
        (keys.keys.len(), age, stale)
    }

    fn verify_local_at(&self, token: &str, now: u64) -> Result<LocalClaims, &'static str> {
        if token.is_empty() {
            return Err("missing_token");
        }
        if token.len() > self.config.max_token_bytes {
            return Err("malformed_token");
        }
        if !canonical_compact_jwt(token) {
            return Err("malformed_token");
        }
        let header = decode_header(token).map_err(|_| "malformed_token")?;
        if header.alg != Algorithm::EdDSA {
            return Err("unsupported_alg");
        }
        if header.typ.as_deref() != Some("at+jwt") {
            return Err("malformed_token");
        }
        let kid = header
            .kid
            .as_deref()
            .filter(|kid| !kid.is_empty())
            .ok_or("unknown_kid")?;
        let keys = self.keys.read().map_err(|_| "key_stale")?;
        let age = epoch_seconds()
            .checked_sub(keys.loaded_at_epoch_seconds.ok_or("key_stale")?)
            .ok_or("key_stale")?;
        if age > self.config.jwks.max_stale_seconds {
            return Err("key_stale");
        }
        let key = keys.keys.get(kid).ok_or("unknown_kid")?;
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_exp = false;
        validation.validate_nbf = false;
        validation.leeway = 0;
        validation.set_required_spec_claims(&["iss", "aud", "sub"]);
        validation.set_issuer(&[self.config.issuer.as_str()]);
        validation.set_audience(&[self.config.audience.as_str()]);
        let token = decode::<Claims>(token, key, &validation).map_err(|error| {
            use jsonwebtoken::errors::ErrorKind;
            match error.kind() {
                ErrorKind::InvalidSignature => "bad_signature",
                ErrorKind::InvalidIssuer => "wrong_issuer",
                ErrorKind::InvalidAudience => "wrong_audience",
                ErrorKind::InvalidAlgorithm | ErrorKind::InvalidAlgorithmName => "unsupported_alg",
                _ => "malformed_token",
            }
        })?;
        let claims = token.claims;
        if claims.iss != self.config.issuer {
            return Err("wrong_issuer");
        }
        if !claims.aud.contains(&self.config.audience) {
            return Err("wrong_audience");
        }
        if claims.token_use != self.config.token_use {
            return Err("wrong_token_use");
        }
        if !claims.scope.contains(&self.config.required_scope) {
            return Err("missing_scope");
        }
        if claims.user_id == 0 || claims.sub != claims.user_id.to_string() {
            return Err("subject_mismatch");
        }
        if claims.auth_version == 0 || uuid::Uuid::parse_str(&claims.jti).is_err() {
            return Err("malformed_token");
        }
        let skew = self.config.clock_skew_seconds;
        if claims.nbf > now.saturating_add(skew) || claims.iat > now.saturating_add(skew) {
            return Err("not_yet_valid");
        }
        if claims.exp.saturating_add(skew) < now {
            return Err("expired");
        }
        if claims.exp <= claims.iat {
            return Err("malformed_token");
        }
        Ok(LocalClaims {
            subject: claims.sub,
            expires_at: claims.exp,
        })
    }

    async fn authorize_at(&self, token: &str, now: u64) -> Result<(), &'static str> {
        let local = self.verify_local_at(token, now)?;
        self.introspect(token, &local, now).await
    }

    async fn introspect(
        &self,
        token: &str,
        local: &LocalClaims,
        now: u64,
    ) -> Result<(), &'static str> {
        let Some(runtime) = self.introspection.as_ref() else {
            return if self.config.introspection.required {
                Err("introspection_unavailable")
            } else {
                Ok(())
            };
        };
        let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        if let Some(entry) = runtime.cache_lookup(&hash) {
            METRICS.cache_hits.fetch_add(1, Ordering::Relaxed);
            return if entry.0 { Ok(()) } else { Err(entry.1) };
        }
        let permit = runtime
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| "introspection_unavailable")?;
        METRICS
            .introspection_requests
            .fetch_add(1, Ordering::Relaxed);
        let result = runtime.request(token).await;
        drop(permit);
        let (active, reason) = match result {
            Ok(response) if response.active => match response.sub.as_deref() {
                Some(subject) if subject == local.subject => (true, "allow"),
                _ => (false, "subject_mismatch"),
            },
            Ok(response) => (false, introspection_reason(response.reason.as_deref())),
            Err(reason) => {
                METRICS
                    .introspection_failures
                    .fetch_add(1, Ordering::Relaxed);
                (false, reason)
            }
        };
        runtime.cache_store(hash, active, reason, local.expires_at, now);
        if active {
            Ok(())
        } else {
            Err(reason)
        }
    }

    async fn refresh_jwks(&self) -> Result<(), String> {
        if self.config.jwks.url.is_empty() {
            return Ok(());
        }
        let raw = fetch_limited(&self.jwks_client, &self.config.jwks.url).await?;
        let keys = parse_jwks(&raw)?;
        let fetched_at = epoch_seconds();
        if let Some(path) = self.jwks_file.as_deref() {
            write_jwks_cache(path, &raw, fetched_at)?;
        }
        let mut state = self
            .keys
            .write()
            .map_err(|_| "JWKS state lock is unavailable".to_owned())?;
        state.keys = keys;
        state.loaded_at_epoch_seconds = Some(fetched_at);
        Ok(())
    }
}

impl IntrospectionRuntime {
    fn cache_lookup(&self, hash: &[u8; 32]) -> Option<(bool, &'static str)> {
        let mut cache = self.cache.lock().ok()?;
        let now = Instant::now();
        cache.entries.retain(|_, entry| entry.expires_at > now);
        cache
            .entries
            .get(hash)
            .map(|entry| (entry.active, entry.reason))
    }

    fn cache_store(
        &self,
        hash: [u8; 32],
        active: bool,
        reason: &'static str,
        token_exp: u64,
        now_epoch: u64,
    ) {
        let ttl = if active {
            self.config
                .positive_cache_seconds
                .min(token_exp.saturating_sub(now_epoch))
        } else {
            self.config.negative_cache_seconds
        };
        if ttl == 0 {
            return;
        }
        let Ok(mut cache) = self.cache.lock() else {
            return;
        };
        cache.sequence = cache.sequence.saturating_add(1);
        let sequence = cache.sequence;
        cache.entries.insert(
            hash,
            CacheEntry {
                active,
                reason,
                expires_at: Instant::now() + Duration::from_secs(ttl),
                sequence,
            },
        );
        while cache.entries.len() > self.config.max_cache_entries {
            let Some(oldest) = cache
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.sequence)
                .map(|(hash, _)| *hash)
            else {
                break;
            };
            cache.entries.remove(&oldest);
        }
    }

    async fn request(&self, token: &str) -> Result<IntrospectionResponse, &'static str> {
        let deadline = Instant::now() + Duration::from_millis(self.config.timeout_ms);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("introspection_timeout");
            }
            let response = hbb_common::tokio::time::timeout(
                remaining,
                self.client
                    .post(&self.config.url)
                    .json(&serde_json::json!({"token": token}))
                    .send(),
            )
            .await
            .map_err(|_| "introspection_timeout")?;
            match response {
                Ok(response) if response.status().is_success() => {
                    return read_json_limited(response).await;
                }
                Ok(response) if response.status().is_server_error() && attempt == 1 => continue,
                Ok(_) => return Err("introspection_unavailable"),
                Err(error) if (error.is_connect() || error.is_timeout()) && attempt == 1 => {
                    continue
                }
                Err(error) if error.is_timeout() => return Err("introspection_timeout"),
                Err(_) => return Err("introspection_unavailable"),
            }
        }
    }
}

pub(crate) async fn prepare(
    config: &ConnectionAuthConfig,
    must_login: bool,
) -> Result<PreparedAuth, String> {
    let effective_mode = starry_config::effective_connection_auth_mode(config.mode, must_login);
    if effective_mode == ConnectionAuthMode::Off {
        return Ok(PreparedAuth {
            runtime: Arc::new(AuthRuntime::disabled()),
        });
    }
    if config.mode == ConnectionAuthMode::Off && must_login {
        return Err(
            "must-login requires an explicit v3 connection_auth verifier configuration".to_owned(),
        );
    }
    let base = starry_config::config_directory();
    let mut keys = HashMap::new();
    let mut loaded_at_epoch_seconds = None;
    let mut local_freshness_error = None;
    let jwks_file = (!config.jwks.file.is_empty()).then(|| resolve_path(&base, &config.jwks.file));
    if !config.jwks.file.is_empty() {
        let path = jwks_file
            .as_deref()
            .expect("a non-empty JWKS file has a resolved path");
        let raw =
            fs::read(path).map_err(|err| format!("cannot read JWKS {}: {err}", path.display()))?;
        keys = parse_jwks(&raw)?;
        match load_jwks_freshness(path, &raw, !config.jwks.url.is_empty()) {
            Ok(fetched_at) => loaded_at_epoch_seconds = fetched_at,
            Err(err) => local_freshness_error = Some(err),
        }
    }
    let jwks_client = build_jwks_client(&base, &config.jwks)?;
    if !config.jwks.url.is_empty() {
        match fetch_limited(&jwks_client, &config.jwks.url).await {
            Ok(raw) => {
                let refreshed = parse_jwks(&raw)?;
                let fetched_at = epoch_seconds();
                if let Some(path) = jwks_file.as_deref() {
                    write_jwks_cache(path, &raw, fetched_at)?;
                }
                keys = refreshed;
                loaded_at_epoch_seconds = Some(fetched_at);
            }
            Err(err) if keys.is_empty() && effective_mode == ConnectionAuthMode::Enforce => {
                return Err(format!("initial JWKS refresh failed: {err}"));
            }
            Err(err) => {
                if effective_mode == ConnectionAuthMode::Enforce {
                    if let Some(freshness_error) = local_freshness_error.as_deref() {
                        return Err(format!(
                            "initial JWKS refresh failed and cached freshness is invalid: {err}; {freshness_error}"
                        ));
                    }
                    let age = loaded_at_epoch_seconds
                        .and_then(|loaded| epoch_seconds().checked_sub(loaded))
                        .ok_or_else(|| {
                            format!(
                                "initial JWKS refresh failed and cached freshness is unavailable: {err}"
                            )
                        })?;
                    if age > config.jwks.max_stale_seconds {
                        return Err(format!(
                            "initial JWKS refresh failed and cached keyset is stale: {err}"
                        ));
                    }
                }
                hbb_common::log::warn!("Initial JWKS refresh retained the local keyset: {err}")
            }
        }
    }
    if effective_mode == ConnectionAuthMode::Enforce && keys.is_empty() {
        return Err("enforce mode has no valid Ed25519 verification key".to_owned());
    }
    if effective_mode == ConnectionAuthMode::Enforce {
        let age = loaded_at_epoch_seconds
            .and_then(|loaded| epoch_seconds().checked_sub(loaded))
            .ok_or_else(|| "enforce mode cannot prove JWKS freshness".to_owned())?;
        if age > config.jwks.max_stale_seconds {
            return Err("enforce mode JWKS cache is stale".to_owned());
        }
    }
    let introspection = if config.introspection.url.is_empty() {
        None
    } else {
        Some(build_introspection(&base, &config.introspection)?)
    };
    if config.introspection.required && introspection.is_none() {
        return Err("required introspection client is unavailable".to_owned());
    }
    Ok(PreparedAuth {
        runtime: Arc::new(AuthRuntime {
            configured_mode: config.mode,
            effective_mode,
            config: config.clone(),
            keys: RwLock::new(KeyState {
                loaded_at_epoch_seconds: (!keys.is_empty())
                    .then_some(loaded_at_epoch_seconds)
                    .flatten(),
                keys,
            }),
            jwks_client,
            jwks_file,
            introspection,
            cancelled: AtomicBool::new(false),
        }),
    })
}

pub(crate) fn activate(prepared: PreparedAuth) -> SubsystemAck {
    let detail = {
        let (count, _, _) = prepared.runtime.key_state();
        format!(
            "{:?} verifier ready with {count} Ed25519 key(s)",
            prepared.runtime.effective_mode
        )
        .to_ascii_lowercase()
    };
    if let Ok(mut active) = ACTIVE.write() {
        active.cancelled.store(true, Ordering::Release);
        *active = prepared.runtime.clone();
    }
    start_jwks_refresh(prepared.runtime);
    SubsystemAck {
        subsystem: "connection_auth".to_owned(),
        accepted: true,
        detail,
    }
}

pub(crate) async fn restore(config: Option<&ConnectionAuthConfig>, must_login: bool) -> String {
    let config = config.cloned().unwrap_or_default();
    match prepare(&config, must_login).await {
        Ok(prepared) => {
            activate(prepared);
            "connection authentication restored".to_owned()
        }
        Err(err) => format!("connection authentication restore failed: {err}"),
    }
}

pub(crate) async fn authorize_connection_attempt(
    token: &str,
    _kind: ConnectionAttemptKind,
    transport: SignalTransport,
    _effective_ip: IpAddr,
) -> AuthDecision {
    METRICS.attempts.fetch_add(1, Ordering::Relaxed);
    let runtime = ACTIVE
        .read()
        .map(|runtime| runtime.clone())
        .unwrap_or_else(|_| Arc::new(AuthRuntime::disabled()));
    if transport == SignalTransport::UnsupportedUdp {
        METRICS.denied.fetch_add(1, Ordering::Relaxed);
        return AuthDecision {
            proceed: false,
            verdict: "deny",
            reason: "unsupported_transport",
            mode: runtime.effective_mode,
        };
    }
    if runtime.effective_mode == ConnectionAuthMode::Off {
        METRICS.allowed.fetch_add(1, Ordering::Relaxed);
        return AuthDecision {
            proceed: true,
            verdict: "skipped",
            reason: "mode_off",
            mode: runtime.effective_mode,
        };
    }
    let now = epoch_seconds();
    let result = runtime.authorize_at(token, now).await;
    match (runtime.effective_mode, result) {
        (_, Ok(())) => {
            METRICS.allowed.fetch_add(1, Ordering::Relaxed);
            AuthDecision {
                proceed: true,
                verdict: "allow",
                reason: "allow",
                mode: runtime.effective_mode,
            }
        }
        (ConnectionAuthMode::Audit, Err(reason)) => {
            METRICS.audit_would_deny.fetch_add(1, Ordering::Relaxed);
            AuthDecision {
                proceed: true,
                verdict: "would_deny",
                reason,
                mode: runtime.effective_mode,
            }
        }
        (_, Err(reason)) => {
            METRICS.denied.fetch_add(1, Ordering::Relaxed);
            AuthDecision {
                proceed: false,
                verdict: "deny",
                reason,
                mode: runtime.effective_mode,
            }
        }
    }
}

pub(crate) fn status() -> AuthStatus {
    let runtime = ACTIVE
        .read()
        .map(|runtime| runtime.clone())
        .unwrap_or_else(|_| Arc::new(AuthRuntime::disabled()));
    let (key_count, key_age_seconds, stale) = runtime.key_state();
    let verifier_state = if runtime.effective_mode == ConnectionAuthMode::Off {
        "disabled"
    } else if key_count == 0 {
        "unavailable"
    } else if stale {
        "degraded"
    } else {
        "ready"
    };
    AuthStatus {
        configured_mode: runtime.configured_mode,
        effective_mode: runtime.effective_mode,
        verifier_state,
        key_count,
        key_age_seconds,
        metrics: METRICS.snapshot(),
    }
}

pub(crate) fn must_login_floor() -> bool {
    let enabled = |value: &str| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "y" | "yes" | "true" | "on"
        )
    };
    std::env::var("MUST_LOGIN")
        .ok()
        .as_deref()
        .is_some_and(enabled)
        || std::env::var("MUST-LOGIN")
            .ok()
            .as_deref()
            .is_some_and(enabled)
        || std::env::args().any(|arg| arg == "--must-login")
}

fn parse_jwks(raw: &[u8]) -> Result<HashMap<String, DecodingKey>, String> {
    let document: JwksDocument =
        serde_json::from_slice(raw).map_err(|err| format!("invalid JWKS document: {err}"))?;
    if document.keys.is_empty() {
        return Err("JWKS document contains no keys".to_owned());
    }
    let mut keys = HashMap::new();
    for key in document.keys {
        if key.kty != "OKP"
            || key.crv != "Ed25519"
            || key.key_use != "sig"
            || key.alg != "EdDSA"
            || key.kid.is_empty()
            || key.kid.len() > 128
        {
            return Err(
                "JWKS contains a key outside the OKP/Ed25519/EdDSA signing profile".to_owned(),
            );
        }
        if key
            .key_ops
            .as_ref()
            .is_some_and(|operations| operations.as_slice() != ["verify"])
        {
            return Err("JWKS key_ops must contain only verify".to_owned());
        }
        let raw_key = decode_config(&key.x, URL_SAFE_NO_PAD)
            .map_err(|_| "JWKS contains an invalid base64url public key".to_owned())?;
        if raw_key.len() != 32 {
            return Err("Ed25519 JWK public key must contain exactly 32 bytes".to_owned());
        }
        if keys
            .insert(key.kid, DecodingKey::from_ed_der(&raw_key))
            .is_some()
        {
            return Err("JWKS contains a duplicate kid".to_owned());
        }
    }
    Ok(keys)
}

fn build_introspection(
    base: &Path,
    config: &IntrospectionConfig,
) -> Result<IntrospectionRuntime, String> {
    let client = build_mtls_client(
        base,
        &config.url,
        &config.ca_file,
        &config.cert_file,
        &config.key_file,
        &config.server_name,
        Duration::from_millis(config.timeout_ms),
        "introspection",
    )?;
    Ok(IntrospectionRuntime {
        client,
        config: config.clone(),
        cache: Mutex::new(IntrospectionCache::default()),
        permits: Arc::new(hbb_common::tokio::sync::Semaphore::new(
            MAX_INTROSPECTION_CONCURRENCY,
        )),
    })
}

fn build_jwks_client(
    base: &Path,
    config: &starry_config::JwksConfig,
) -> Result<reqwest::Client, String> {
    if config.url.is_empty() {
        return Ok(default_jwks_client());
    }
    build_mtls_client(
        base,
        &config.url,
        &config.ca_file,
        &config.cert_file,
        &config.key_file,
        &config.server_name,
        Duration::from_secs(10),
        "JWKS",
    )
}

fn build_mtls_client(
    base: &Path,
    endpoint: &str,
    ca_file: &str,
    cert_file: &str,
    key_file: &str,
    server_name: &str,
    timeout: Duration,
    label: &str,
) -> Result<reqwest::Client, String> {
    let parsed = url::Url::parse(endpoint).map_err(|err| format!("invalid {label} URL: {err}"))?;
    if parsed.host_str() != Some(server_name) {
        return Err(format!(
            "{label} URL host must equal configured server_name"
        ));
    }
    let ca_path = resolve_path(base, ca_file);
    let ca = fs::read(&ca_path)
        .map_err(|err| format!("cannot read {label} CA {}: {err}", ca_path.display()))?;
    let ca = reqwest::Certificate::from_pem(&ca)
        .map_err(|_| format!("{label} CA is not a valid PEM certificate"))?;

    let cert_path = resolve_path(base, cert_file);
    let key_path = resolve_path(base, key_file);
    let cert = fs::read(&cert_path).map_err(|err| {
        format!(
            "cannot read {label} client certificate {}: {err}",
            cert_path.display()
        )
    })?;
    let key = fs::read(&key_path).map_err(|err| {
        format!(
            "cannot read {label} client key {}: {err}",
            key_path.display()
        )
    })?;
    let identity = parse_client_identity(&cert, &key, label)?;

    reqwest::Client::builder()
        .connect_timeout(timeout)
        .timeout(timeout)
        // Kessoku's internal mTLS listener closes idle HTTP connections after
        // 30 seconds.  Retire pooled connections sooner so a 30-second JWKS
        // refresh never races a server-side idle close and then falls back to
        // last-known-good keys for an otherwise healthy issuer.
        .pool_idle_timeout(Duration::from_secs(15))
        .https_only(true)
        .min_tls_version(reqwest::tls::Version::TLS_1_3)
        .redirect(reqwest::redirect::Policy::none())
        .tls_built_in_root_certs(false)
        .add_root_certificate(ca)
        .identity(identity)
        .build()
        .map_err(|err| format!("cannot build {label} client: {err}"))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn parse_client_identity(
    certificate: &[u8],
    private_key: &[u8],
    label: &str,
) -> Result<reqwest::Identity, String> {
    // The upstream target-specific dependency uses reqwest's native-tls
    // backend on macOS and Windows. That backend accepts separate certificate
    // and PKCS#8 key PEM documents instead of rustls' combined PEM identity.
    reqwest::Identity::from_pkcs8_pem(certificate, private_key)
        .map_err(|_| format!("{label} client identity is not valid certificate/PKCS#8 key PEM"))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn parse_client_identity(
    certificate: &[u8],
    private_key: &[u8],
    label: &str,
) -> Result<reqwest::Identity, String> {
    let mut identity = certificate.to_vec();
    identity.extend_from_slice(b"\n");
    identity.extend_from_slice(private_key);
    reqwest::Identity::from_pem(&identity)
        .map_err(|_| format!("{label} client identity is not valid PEM"))
}

fn default_jwks_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn start_jwks_refresh(runtime: Arc<AuthRuntime>) {
    if runtime.config.jwks.url.is_empty() {
        return;
    }
    hbb_common::tokio::spawn(async move {
        let interval = Duration::from_secs(runtime.config.jwks.refresh_interval_seconds);
        loop {
            hbb_common::tokio::time::sleep(interval).await;
            if runtime.cancelled.load(Ordering::Acquire) {
                return;
            }
            if let Err(err) = runtime.refresh_jwks().await {
                hbb_common::log::warn!(
                    "JWKS refresh failed; retaining last-known-good keys: {err}"
                );
            }
        }
    });
}

async fn fetch_limited(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|err| format!("JWKS request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("JWKS endpoint returned HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > INTROSPECTION_RESPONSE_LIMIT as u64)
    {
        return Err("JWKS response exceeds 64 KiB".to_owned());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| format!("cannot read JWKS response: {err}"))?
    {
        if body.len().saturating_add(chunk.len()) > INTROSPECTION_RESPONSE_LIMIT {
            return Err("JWKS response exceeds 64 KiB".to_owned());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn jwks_sha256(raw: &[u8]) -> String {
    Sha256::digest(raw)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn jwks_metadata_path(path: &Path) -> Result<PathBuf, String> {
    let mut file_name = path
        .file_name()
        .ok_or_else(|| "JWKS cache path has no file name".to_owned())?
        .to_os_string();
    file_name.push(".metadata.json");
    Ok(path.with_file_name(file_name))
}

fn load_jwks_freshness(
    path: &Path,
    raw: &[u8],
    require_metadata: bool,
) -> Result<Option<u64>, String> {
    let metadata_path = jwks_metadata_path(path)?;
    match fs::read(&metadata_path) {
        Ok(metadata_raw) => {
            if metadata_raw.len() > 4_096 {
                return Err("JWKS cache metadata exceeds 4 KiB".to_owned());
            }
            let metadata: JwksCacheMetadata = serde_json::from_slice(&metadata_raw)
                .map_err(|err| format!("invalid JWKS cache metadata: {err}"))?;
            if metadata.version != 1 {
                return Err("unsupported JWKS cache metadata version".to_owned());
            }
            if metadata.jwks_sha256 != jwks_sha256(raw) {
                return Err("JWKS cache metadata digest does not match the keyset".to_owned());
            }
            if metadata.fetched_at_epoch_seconds > epoch_seconds().saturating_add(300) {
                return Err("JWKS cache metadata has an invalid future timestamp".to_owned());
            }
            Ok(Some(metadata.fetched_at_epoch_seconds))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && require_metadata => Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let modified = fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .and_then(|modified| {
                    modified
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))
                })
                .map_err(|err| format!("cannot read JWKS file freshness: {err}"))?
                .as_secs();
            if modified > epoch_seconds().saturating_add(300) {
                return Err("JWKS file has an invalid future modification time".to_owned());
            }
            Ok(Some(modified))
        }
        Err(err) => Err(format!(
            "cannot read JWKS cache metadata {}: {err}",
            metadata_path.display()
        )),
    }
}

fn write_jwks_cache(path: &Path, raw: &[u8], fetched_at: u64) -> Result<(), String> {
    let metadata = serde_json::to_vec(&JwksCacheMetadata {
        version: 1,
        fetched_at_epoch_seconds: fetched_at,
        jwks_sha256: jwks_sha256(raw),
    })
    .map_err(|err| format!("cannot serialize JWKS cache metadata: {err}"))?;
    atomic_write(path, raw)?;
    atomic_write(&jwks_metadata_path(path)?, &metadata)
}

fn atomic_write(path: &Path, raw: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|err| format!("cannot create JWKS cache directory: {err}"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "JWKS cache path has no valid file name".to_owned())?;
    let temporary = parent.join(format!(".{file_name}.{}.starry.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|err| format!("cannot create JWKS cache temporary file: {err}"))?;
        file.write_all(raw)
            .map_err(|err| format!("cannot write JWKS cache: {err}"))?;
        file.sync_all()
            .map_err(|err| format!("cannot fsync JWKS cache: {err}"))?;
        drop(file);
        fs::rename(&temporary, path)
            .map_err(|err| format!("cannot atomically replace JWKS cache: {err}"))?;
        #[cfg(unix)]
        {
            fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|err| format!("cannot fsync JWKS cache directory: {err}"))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

async fn read_json_limited(
    mut response: reqwest::Response,
) -> Result<IntrospectionResponse, &'static str> {
    if response
        .content_length()
        .is_some_and(|length| length > INTROSPECTION_RESPONSE_LIMIT as u64)
    {
        return Err("introspection_unavailable");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "introspection_unavailable")?
    {
        if body.len().saturating_add(chunk.len()) > INTROSPECTION_RESPONSE_LIMIT {
            return Err("introspection_unavailable");
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| "introspection_unavailable")
}

fn introspection_reason(reason: Option<&str>) -> &'static str {
    match reason {
        Some("user_disabled") => "user_disabled",
        Some("expired") => "expired",
        Some("deleted_user") => "user_disabled",
        _ => "revoked",
    }
}

fn canonical_compact_jwt(token: &str) -> bool {
    let mut segments = token.split('.');
    for _ in 0..3 {
        let Some(segment) = segments.next().filter(|segment| !segment.is_empty()) else {
            return false;
        };
        let Ok(decoded) = decode_config(segment, URL_SAFE_NO_PAD) else {
            return false;
        };
        if encode_config(decoded, URL_SAFE_NO_PAD) != segment {
            return false;
        }
    }
    segments.next().is_none()
}

fn resolve_path(base: &Path, configured: &str) -> PathBuf {
    let path = PathBuf::from(configured);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        runtime::Builder,
    };
    use rcgen::{
        BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair,
        PKCS_ECDSA_P256_SHA256,
    };
    use rustls::{
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
        server::WebPkiClientVerifier,
        RootCertStore, ServerConfig,
    };
    use serde_json::{json, Value};
    use sodiumoxide::crypto::sign;
    use tokio_rustls::TlsAcceptor;

    fn fixture_runtime() -> AuthRuntime {
        let config = ConnectionAuthConfig {
            mode: ConnectionAuthMode::Enforce,
            issuer: "https://api.example.test".to_owned(),
            audience: "rustdesk-connect".to_owned(),
            token_use: "access".to_owned(),
            required_scope: "connect:initiate".to_owned(),
            max_token_bytes: 8_192,
            clock_skew_seconds: 0,
            jwks: starry_config::JwksConfig {
                max_stale_seconds: 3_600,
                ..Default::default()
            },
            introspection: IntrospectionConfig::default(),
        };
        let keys = parse_jwks(include_bytes!("../contracts/auth/v1/fixtures/jwks.json")).unwrap();
        AuthRuntime {
            configured_mode: ConnectionAuthMode::Enforce,
            effective_mode: ConnectionAuthMode::Enforce,
            config,
            keys: RwLock::new(KeyState {
                keys,
                loaded_at_epoch_seconds: Some(epoch_seconds()),
            }),
            jwks_client: default_jwks_client(),
            jwks_file: None,
            introspection: None,
            cancelled: AtomicBool::new(false),
        }
    }

    fn fixture(name: &str) -> String {
        let raw = match name {
            "active" => include_str!("../contracts/auth/v1/fixtures/active.jwt.txt"),
            "expired" => include_str!("../contracts/auth/v1/fixtures/expired.jwt.txt"),
            "wrong-audience" => {
                include_str!("../contracts/auth/v1/fixtures/wrong-audience.jwt.txt")
            }
            _ => unreachable!(),
        };
        raw.trim().to_owned()
    }

    #[test]
    fn strict_fixture_profile_accepts_only_the_active_expected_audience_token() {
        let runtime = fixture_runtime();
        assert!(runtime
            .verify_local_at(&fixture("active"), 1_893_456_000)
            .is_ok());
        assert_eq!(
            runtime
                .verify_local_at(&fixture("expired"), 1_893_456_000)
                .unwrap_err(),
            "expired"
        );
        assert_eq!(
            runtime
                .verify_local_at(&fixture("wrong-audience"), 1_893_456_000)
                .unwrap_err(),
            "wrong_audience"
        );
    }

    #[test]
    fn deterministic_token_and_jwks_mutation_corpus_never_panics() {
        let runtime = fixture_runtime();
        let active = fixture("active");
        let accepted = runtime.verify_local_at(&active, 1_893_456_000);
        assert!(accepted.is_ok());

        // Exercise every byte boundary and every individual bit in the signed
        // fixture. This corpus runs in ordinary CI, unlike an opt-in fuzzer,
        // and locks the parser's fail-closed behavior on every build.
        for end in 0..active.len() {
            let _ = runtime.verify_local_at(&active[..end], 1_893_456_000);
        }
        for index in 0..active.len() {
            for bit in 0..8 {
                let mut mutated = active.as_bytes().to_vec();
                mutated[index] ^= 1 << bit;
                let mutated = String::from_utf8_lossy(&mutated);
                assert!(runtime.verify_local_at(&mutated, 1_893_456_000).is_err());
            }
        }

        let jwks = include_bytes!("../contracts/auth/v1/fixtures/jwks.json");
        assert!(parse_jwks(jwks).is_ok());
        for end in 0..jwks.len() {
            let _ = parse_jwks(&jwks[..end]);
        }
        for index in 0..jwks.len() {
            let mut mutated = jwks.to_vec();
            mutated[index] ^= 0x80;
            let _ = parse_jwks(&mutated);
        }

        let mut state = 0x6a09_e667_f3bc_c909_u64;
        for length in [0, 1, 2, 3, 31, 255, 1_024, 8_192, 8_193, 65_536] {
            let mut bytes = Vec::with_capacity(length);
            for _ in 0..length {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                bytes.push((state & 0x7f) as u8);
            }
            let token = String::from_utf8(bytes).unwrap();
            let _ = runtime.verify_local_at(&token, 1_893_456_000);
        }
    }

    #[test]
    fn strict_profile_classifies_all_local_token_failures_without_fallback() {
        sodiumoxide::init().unwrap();
        let (runtime, secret) = generated_runtime();
        let valid_claims = test_claims();
        let valid = signed_token(&secret, "EdDSA", Some("generated-key"), &valid_claims);
        assert!(runtime.verify_local_at(&valid, 1_100).is_ok());
        assert_eq!(
            runtime.verify_local_at("", 1_100).unwrap_err(),
            "missing_token"
        );
        assert_eq!(
            runtime
                .verify_local_at(&"x".repeat(runtime.config.max_token_bytes + 1), 1_100)
                .unwrap_err(),
            "malformed_token"
        );
        assert_eq!(
            runtime.verify_local_at("not-a-jwt", 1_100).unwrap_err(),
            "malformed_token"
        );
        assert_eq!(
            runtime
                .verify_local_at(
                    &signed_token(&secret, "HS256", Some("generated-key"), &valid_claims),
                    1_100,
                )
                .unwrap_err(),
            "unsupported_alg"
        );
        assert_eq!(
            runtime
                .verify_local_at(&signed_token(&secret, "EdDSA", None, &valid_claims), 1_100)
                .unwrap_err(),
            "unknown_kid"
        );
        assert_eq!(
            runtime
                .verify_local_at(
                    &signed_token(&secret, "EdDSA", Some("removed-key"), &valid_claims),
                    1_100,
                )
                .unwrap_err(),
            "unknown_kid"
        );
        let (_, wrong_secret) = sign::gen_keypair();
        assert_eq!(
            runtime
                .verify_local_at(
                    &signed_token(&wrong_secret, "EdDSA", Some("generated-key"), &valid_claims,),
                    1_100,
                )
                .unwrap_err(),
            "bad_signature"
        );

        for (field, value, reason) in [
            ("iss", json!("https://wrong.example.test"), "wrong_issuer"),
            ("aud", json!("wrong-audience"), "wrong_audience"),
            ("token_use", json!("refresh"), "wrong_token_use"),
            ("scope", json!("profile:read"), "missing_scope"),
            ("user_id", json!(1_002), "subject_mismatch"),
            ("auth_version", json!(0), "malformed_token"),
            ("jti", json!("not-a-uuid"), "malformed_token"),
            ("nbf", json!(1_500), "not_yet_valid"),
            ("iat", json!(1_500), "not_yet_valid"),
            ("exp", json!(1_000), "expired"),
        ] {
            let mut claims = valid_claims.clone();
            claims[field] = value;
            let token = signed_token(&secret, "EdDSA", Some("generated-key"), &claims);
            assert_eq!(runtime.verify_local_at(&token, 1_100).unwrap_err(), reason);
        }

        let mut malformed_window = valid_claims;
        malformed_window["exp"] = json!(900);
        malformed_window["iat"] = json!(900);
        malformed_window["nbf"] = json!(900);
        let token = signed_token(&secret, "EdDSA", Some("generated-key"), &malformed_window);
        assert_eq!(
            runtime.verify_local_at(&token, 900).unwrap_err(),
            "malformed_token"
        );
    }

    #[test]
    fn rejects_symmetric_private_and_duplicate_jwks_material() {
        for raw in [
            br#"{"keys":[{"kty":"oct","crv":"Ed25519","use":"sig","alg":"EdDSA","kid":"x","x":"AA"}]}"#.as_slice(),
            br#"{"keys":[{"kty":"OKP","crv":"Ed25519","use":"sig","alg":"EdDSA","kid":"x","x":"swHHZ5fMnHfaDvSfO5QwRTGYUL2dnh9j1iXhNiaU8tk","d":"private"}]}"#.as_slice(),
            br#"{"keys":[{"kty":"OKP","crv":"Ed25519","use":"sig","alg":"EdDSA","kid":"x","x":"swHHZ5fMnHfaDvSfO5QwRTGYUL2dnh9j1iXhNiaU8tk"},{"kty":"OKP","crv":"Ed25519","use":"sig","alg":"EdDSA","kid":"x","x":"swHHZ5fMnHfaDvSfO5QwRTGYUL2dnh9j1iXhNiaU8tk"}]}"#.as_slice(),
        ] {
            assert!(parse_jwks(raw).is_err());
        }
    }

    #[test]
    fn stale_last_known_good_keyset_fails_closed() {
        let runtime = fixture_runtime();
        runtime.keys.write().unwrap().loaded_at_epoch_seconds =
            Some(epoch_seconds().saturating_sub(3_601));
        assert_eq!(
            runtime
                .verify_local_at(&fixture("active"), 1_893_456_000)
                .unwrap_err(),
            "key_stale"
        );
    }

    #[test]
    fn persisted_jwks_freshness_is_digest_bound_and_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("jwks.json");
        let raw = include_bytes!("../contracts/auth/v1/fixtures/jwks.json");
        let fetched_at = epoch_seconds().saturating_sub(3_601);
        write_jwks_cache(&path, raw, fetched_at).unwrap();

        assert_eq!(
            load_jwks_freshness(&path, raw, true).unwrap(),
            Some(fetched_at)
        );
        assert!(load_jwks_freshness(&path, b"{\"keys\":[]}", true).is_err());

        fs::remove_file(jwks_metadata_path(&path).unwrap()).unwrap();
        assert_eq!(load_jwks_freshness(&path, raw, true).unwrap(), None);
    }

    #[test]
    fn deterministic_cache_evicts_oldest_without_storing_raw_tokens() {
        let runtime = IntrospectionRuntime {
            client: reqwest::Client::new(),
            config: IntrospectionConfig {
                max_cache_entries: 1,
                positive_cache_seconds: 10,
                ..Default::default()
            },
            cache: Mutex::new(IntrospectionCache::default()),
            permits: Arc::new(hbb_common::tokio::sync::Semaphore::new(1)),
        };
        runtime.cache_store(
            [1; 32],
            true,
            "allow",
            epoch_seconds() + 60,
            epoch_seconds(),
        );
        runtime.cache_store(
            [2; 32],
            true,
            "allow",
            epoch_seconds() + 60,
            epoch_seconds(),
        );
        assert!(runtime.cache_lookup(&[1; 32]).is_none());
        assert_eq!(runtime.cache_lookup(&[2; 32]), Some((true, "allow")));
    }

    #[test]
    fn local_failure_skips_introspection_and_success_uses_the_hash_cache() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let url = format!("http://{}/introspect", listener.local_addr().unwrap());
                let requests = Arc::new(AtomicU64::new(0));
                let server = spawn_json_server(
                    listener,
                    requests.clone(),
                    200,
                    r#"{"active":true,"sub":"42"}"#.to_owned(),
                );
                let mut runtime = fixture_runtime();
                attach_test_introspection(&mut runtime, &url, false, 500);

                assert_eq!(
                    runtime
                        .authorize_at("not-a-compact-jwt", 1_893_456_000)
                        .await
                        .unwrap_err(),
                    "malformed_token"
                );
                assert_eq!(requests.load(Ordering::SeqCst), 0);
                runtime
                    .authorize_at(&fixture("active"), 1_893_456_000)
                    .await
                    .unwrap();
                assert_eq!(requests.load(Ordering::SeqCst), 1);
                runtime
                    .authorize_at(&fixture("active"), 1_893_456_000)
                    .await
                    .unwrap();
                assert_eq!(requests.load(Ordering::SeqCst), 1);
                server.abort();
            });
    }

    #[test]
    fn introspection_request_matches_kessoku_strict_token_only_dto() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let url = format!("http://{}/introspect", listener.local_addr().unwrap());
                let captured = Arc::new(Mutex::new(None));
                let captured_by_server = captured.clone();
                let server = hbb_common::tokio::spawn(async move {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    let mut chunk = [0_u8; 1024];
                    let body = loop {
                        let read = stream.read(&mut chunk).await.unwrap();
                        assert_ne!(read, 0, "introspection request ended before its body");
                        request.extend_from_slice(&chunk[..read]);
                        let Some(header_end) = request
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                        else {
                            continue;
                        };
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let content_length = headers
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                            .expect("reqwest introspection request has Content-Length");
                        let body_start = header_end + 4;
                        if request.len() < body_start + content_length {
                            continue;
                        }
                        break request[body_start..body_start + content_length].to_vec();
                    };
                    *captured_by_server.lock().unwrap() = Some(body);
                    let response_body = r#"{"active":true,"sub":"42"}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                        response_body.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                });

                let mut runtime = fixture_runtime();
                attach_test_introspection(&mut runtime, &url, true, 500);
                let active = fixture("active");
                runtime
                    .authorize_at(&active, 1_893_456_000)
                    .await
                    .unwrap();
                server.await.unwrap();

                let body = captured.lock().unwrap().take().unwrap();
                let document: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(document, json!({"token": active}));
                assert_eq!(document.as_object().unwrap().len(), 1);
            });
    }

    #[test]
    fn active_introspection_requires_the_locally_verified_subject() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                for body in [
                    r#"{"active":true}"#,
                    r#"{"active":true,"sub":"different-user"}"#,
                ] {
                    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                    let url = format!("http://{}/introspect", listener.local_addr().unwrap());
                    let requests = Arc::new(AtomicU64::new(0));
                    let server =
                        spawn_json_server(listener, requests.clone(), 200, body.to_owned());
                    let mut runtime = fixture_runtime();
                    attach_test_introspection(&mut runtime, &url, true, 500);

                    assert_eq!(
                        runtime
                            .authorize_at(&fixture("active"), 1_893_456_000)
                            .await
                            .unwrap_err(),
                        "subject_mismatch"
                    );
                    assert_eq!(requests.load(Ordering::SeqCst), 1);
                    server.abort();
                }
            });
    }

    #[test]
    fn configured_introspection_failure_is_fail_closed_even_without_required_flag() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let url = format!("http://{}/introspect", listener.local_addr().unwrap());
                let requests = Arc::new(AtomicU64::new(0));
                let server = spawn_json_server(listener, requests.clone(), 503, "{}".to_owned());
                let mut runtime = fixture_runtime();
                attach_test_introspection(&mut runtime, &url, false, 500);

                assert_eq!(
                    runtime
                        .authorize_at(&fixture("active"), 1_893_456_000)
                        .await
                        .unwrap_err(),
                    "introspection_unavailable"
                );
                assert_eq!(requests.load(Ordering::SeqCst), 2);
                server.abort();
            });
    }

    #[test]
    fn jwks_refresh_client_uses_pinned_ca_and_client_certificate() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
                let mut ca_params = CertificateParams::new(Vec::new()).unwrap();
                ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
                let ca = ca_params.self_signed(&ca_key).unwrap();

                let server_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
                let mut server_params =
                    CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
                server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
                let server_certificate = server_params
                    .signed_by(&server_key, &ca, &ca_key)
                    .unwrap();

                let client_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
                let mut client_params = CertificateParams::new(Vec::new()).unwrap();
                client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
                let client_certificate = client_params
                    .signed_by(&client_key, &ca, &ca_key)
                    .unwrap();

                let directory = tempfile::tempdir().unwrap();
                let ca_path = directory.path().join("ca.pem");
                let client_path = directory.path().join("client.pem");
                let client_key_path = directory.path().join("client-key.pem");
                fs::write(&ca_path, ca.pem()).unwrap();
                fs::write(&client_path, client_certificate.pem()).unwrap();
                fs::write(&client_key_path, client_key.serialize_pem()).unwrap();

                let mut client_roots = RootCertStore::empty();
                client_roots
                    .add(CertificateDer::from(ca.der().to_vec()))
                    .unwrap();
                let verifier = WebPkiClientVerifier::builder(Arc::new(client_roots))
                    .build()
                    .unwrap();
                let tls = ServerConfig::builder()
                    .with_client_cert_verifier(verifier)
                    .with_single_cert(
                        vec![CertificateDer::from(
                            server_certificate.der().to_vec(),
                        )],
                        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                            server_key.serialize_der(),
                        )),
                    )
                    .unwrap();
                let acceptor = TlsAcceptor::from(Arc::new(tls));
                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let port = listener.local_addr().unwrap().port();
                let jwks = include_bytes!("../contracts/auth/v1/fixtures/jwks.json").to_vec();
                let served = jwks.clone();
                let server = hbb_common::tokio::spawn(async move {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut stream = acceptor.accept(stream).await.unwrap();
                    assert!(stream
                        .get_ref()
                        .1
                        .peer_certificates()
                        .is_some_and(|certificates| !certificates.is_empty()));
                    let mut request = Vec::new();
                    let mut chunk = [0_u8; 1024];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        let read = stream.read(&mut chunk).await.unwrap();
                        assert_ne!(read, 0);
                        request.extend_from_slice(&chunk[..read]);
                    }
                    assert!(String::from_utf8_lossy(&request)
                        .starts_with("GET /api/internal/v1/auth/jwks HTTP/1.1\r\n"));
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        served.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                    stream.write_all(&served).await.unwrap();
                });

                let config = starry_config::JwksConfig {
                    url: format!("https://localhost:{port}/api/internal/v1/auth/jwks"),
                    ca_file: ca_path.to_string_lossy().into_owned(),
                    cert_file: client_path.to_string_lossy().into_owned(),
                    key_file: client_key_path.to_string_lossy().into_owned(),
                    server_name: "localhost".to_owned(),
                    ..Default::default()
                };
                let client = build_jwks_client(Path::new("."), &config).unwrap();
                let received = fetch_limited(&client, &config.url).await.unwrap();
                assert_eq!(received, jwks);
                server.await.unwrap();
            });
    }

    fn attach_test_introspection(
        runtime: &mut AuthRuntime,
        url: &str,
        required: bool,
        timeout_ms: u64,
    ) {
        let config = IntrospectionConfig {
            required,
            url: url.to_owned(),
            timeout_ms,
            positive_cache_seconds: 10,
            negative_cache_seconds: 1,
            max_cache_entries: 100,
            ..Default::default()
        };
        runtime.config.introspection = config.clone();
        runtime.introspection = Some(IntrospectionRuntime {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            config,
            cache: Mutex::new(IntrospectionCache::default()),
            permits: Arc::new(hbb_common::tokio::sync::Semaphore::new(4)),
        });
    }

    fn spawn_json_server(
        listener: TcpListener,
        requests: Arc<AtomicU64>,
        status: u16,
        body: String,
    ) -> hbb_common::tokio::task::JoinHandle<()> {
        hbb_common::tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                requests.fetch_add(1, Ordering::SeqCst);
                let mut request = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let Ok(read) = stream.read(&mut chunk).await else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let reason = if status == 200 {
                    "OK"
                } else {
                    "Service Unavailable"
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        })
    }

    #[test]
    fn jwks_refresh_atomically_rotates_keys_and_retains_last_known_good_on_failure() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                sodiumoxide::init().unwrap();
                let (public, secret) = sign::gen_keypair();
                let rotated = serde_json::to_vec(&serde_json::json!({
                    "keys": [{
                        "kty": "OKP",
                        "crv": "Ed25519",
                        "use": "sig",
                        "alg": "EdDSA",
                        "kid": "rotated-key",
                        "key_ops": ["verify"],
                        "x": base64::encode_config(public.0, URL_SAFE_NO_PAD)
                    }]
                }))
                .unwrap();
                let directory = std::env::temp_dir().join(format!(
                    "starry-jwks-rotation-{}-{}",
                    std::process::id(),
                    uuid::Uuid::new_v4()
                ));
                fs::create_dir_all(&directory).unwrap();
                let cache_path = directory.join("jwks.json");

                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let url = format!("http://{}/jwks", listener.local_addr().unwrap());
                let server = spawn_json_server(
                    listener,
                    Arc::new(AtomicU64::new(0)),
                    200,
                    String::from_utf8(rotated.clone()).unwrap(),
                );
                let mut runtime = fixture_runtime();
                runtime.config.jwks.url = url;
                runtime.jwks_client = reqwest::Client::new();
                runtime.jwks_file = Some(cache_path.clone());
                runtime.refresh_jwks().await.unwrap();
                server.abort();

                assert_eq!(fs::read(&cache_path).unwrap(), rotated);
                let persisted_freshness = load_jwks_freshness(&cache_path, &rotated, true).unwrap();
                assert!(persisted_freshness.is_some());
                let token = signed_token(&secret, "EdDSA", Some("rotated-key"), &test_claims());
                assert!(runtime.verify_local_at(&token, 1_100).is_ok());
                assert_eq!(
                    runtime
                        .verify_local_at(&fixture("active"), 1_893_456_000)
                        .unwrap_err(),
                    "unknown_kid"
                );

                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                runtime.config.jwks.url = format!("http://{}/jwks", listener.local_addr().unwrap());
                let server = spawn_json_server(
                    listener,
                    Arc::new(AtomicU64::new(0)),
                    200,
                    "{not-json".to_owned(),
                );
                assert!(runtime.refresh_jwks().await.is_err());
                server.abort();
                assert!(runtime.verify_local_at(&token, 1_100).is_ok());
                assert_eq!(fs::read(&cache_path).unwrap(), rotated);
                assert!(fs::read_dir(&directory).unwrap().all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp")));
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    assert_eq!(
                        fs::metadata(&cache_path).unwrap().permissions().mode() & 0o777,
                        0o600
                    );
                }
                let _ = fs::remove_dir_all(directory);
            });
    }

    fn generated_runtime() -> (AuthRuntime, sign::SecretKey) {
        let (public, secret) = sign::gen_keypair();
        let raw = serde_json::to_vec(&serde_json::json!({
            "keys": [{
                "kty": "OKP",
                "crv": "Ed25519",
                "use": "sig",
                "alg": "EdDSA",
                "kid": "generated-key",
                "x": base64::encode_config(public.0, URL_SAFE_NO_PAD)
            }]
        }))
        .unwrap();
        let mut runtime = fixture_runtime();
        runtime.keys = RwLock::new(KeyState {
            keys: parse_jwks(&raw).unwrap(),
            loaded_at_epoch_seconds: Some(epoch_seconds()),
        });
        (runtime, secret)
    }

    fn test_claims() -> Value {
        serde_json::json!({
            "iss": "https://api.example.test",
            "aud": "rustdesk-connect",
            "token_use": "access",
            "scope": "connect:initiate profile:read",
            "sub": "1001",
            "user_id": 1_001,
            "auth_version": 1,
            "jti": "01941f29-7c30-7000-8000-000000001001",
            "iat": 1_000,
            "nbf": 1_000,
            "exp": 2_000
        })
    }

    fn signed_token(
        secret: &sign::SecretKey,
        algorithm: &str,
        kid: Option<&str>,
        claims: &Value,
    ) -> String {
        let mut header = serde_json::json!({"alg": algorithm, "typ": "at+jwt"});
        if let Some(kid) = kid {
            header["kid"] = Value::String(kid.to_owned());
        }
        let header = base64::encode_config(serde_json::to_vec(&header).unwrap(), URL_SAFE_NO_PAD);
        let claims = base64::encode_config(serde_json::to_vec(claims).unwrap(), URL_SAFE_NO_PAD);
        let input = format!("{header}.{claims}");
        let signature = sign::sign_detached(input.as_bytes(), secret);
        format!(
            "{input}.{}",
            base64::encode_config(signature.as_ref(), URL_SAFE_NO_PAD)
        )
    }
}

mod auth;
mod config_store;
mod local_client;

use auth::{AuthFailure, ControlPrincipal, ServiceJwtVerifier};
use axum::{
    body::Body,
    extract::{ContentLengthLimit, Extension, Path as AxumPath, RawBody},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use config_store::{ConfigTransactions, RuntimeSnapshot, TransactionCaller, TransactionError};
use hbb_common::tokio::{
    net::TcpListener,
    sync::Semaphore,
    time::{timeout, Duration},
};
use hyper::body::HttpBody as _;
use hyper::service::service_fn;
use rustls::{
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
    server::WebPkiClientVerifier,
    RootCertStore, ServerConfig,
};
use serde_derive::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    fs::{self, OpenOptions},
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;
use x509_parser::{extensions::GeneralName, parse_x509_certificate};

const CONTROL_BODY_LIMIT: u64 = 1024 * 1024 + 4096;
const MAX_CONNECTIONS: usize = 256;
const CONNECTION_DEADLINE_SECONDS: u64 = 30;
const REQUESTS_PER_MINUTE: u32 = 120;
const STARRY_PATCH_VERSION: &str = "1.3.0";
const CONTROL_SCHEMA: &str = include_str!("../contracts/config/v4/config.schema.json");
const CONTROL_UI_SCHEMA: &str = include_str!("../contracts/config/v4/config.ui-schema.json");

fn starry_version() -> String {
    format!(
        "{}-patch-v{}",
        env!("CARGO_PKG_VERSION"),
        STARRY_PATCH_VERSION
    )
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentConfig {
    version: u8,
    instance_id_file: String,
    listen: SocketAddr,
    tls: TlsConfig,
    service_jwt: ServiceJwtConfig,
    local_control: LocalControlConfig,
    config: ManagedConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TlsConfig {
    ca_file: String,
    cert_file: String,
    key_file: String,
    allowed_client_uri_sans: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ServiceJwtConfig {
    issuer: String,
    jwks_file: String,
    audience_prefix: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalControlConfig {
    address: SocketAddr,
    token_file: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedConfig {
    path: String,
    backup_dir: String,
    max_bytes: usize,
    #[serde(default)]
    write_enabled: bool,
}

struct AgentState {
    instance_id: String,
    local_address: SocketAddr,
    max_config_bytes: usize,
    write_enabled: bool,
    verifier: ServiceJwtVerifier,
    transactions: Arc<ConfigTransactions>,
    rate: Mutex<HashMap<String, RateWindow>>,
}

struct RateWindow {
    started: Instant,
    requests: u32,
}

#[derive(Clone)]
struct ClientCertificate {
    uri_san: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct AgentError {
    code: String,
    detail: String,
    retryable: bool,
}

#[derive(Serialize)]
struct Problem {
    #[serde(rename = "type")]
    problem_type: String,
    title: String,
    status: u16,
    code: String,
    detail: String,
    request_id: String,
    retryable: bool,
    errors: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigCandidate {
    document: String,
    format: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigApplyRequest {
    plan_id: String,
    candidate_digest: String,
    #[serde(default)]
    comment: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigRollbackRequest {
    revision_id: String,
    #[serde(default)]
    comment: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeReloadRequest {
    expected_source_digest: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PeerVerifyRequest {
    id: String,
    uuid: String,
    #[serde(default)]
    activation_epoch: u64,
    #[serde(default)]
    activation_id: String,
    #[serde(default)]
    route_leases: Vec<String>,
}

pub async fn run(config_path: impl AsRef<Path>) -> Result<(), String> {
    let config_path = config_path.as_ref();
    let raw = fs::read(config_path).map_err(|err| {
        format!(
            "cannot read control Agent config {}: {err}",
            config_path.display()
        )
    })?;
    let config: AgentConfig =
        serde_yml::from_slice(&raw).map_err(|err| format!("invalid Agent config: {err}"))?;
    validate_config(&config)?;
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    let instance_path = resolve_path(base, &config.instance_id_file);
    let instance_id = load_or_create_instance_id(&instance_path)?;
    local_client::configure_auth_token_file(resolve_path(base, &config.local_control.token_file))?;
    let verifier = ServiceJwtVerifier::load(&config.service_jwt, base, &instance_id)?;
    let tls = load_tls(&config.tls, base)?;
    let transactions = ConfigTransactions::open(
        instance_id.clone(),
        resolve_path(base, &config.config.path),
        resolve_path(base, &config.config.backup_dir),
        config.config.max_bytes,
    )?;
    if config.config.write_enabled {
        transactions.validate_write_authority()?;
    }
    let allowed_sans: HashSet<String> =
        config.tls.allowed_client_uri_sans.iter().cloned().collect();
    let state = Arc::new(AgentState {
        instance_id,
        local_address: config.local_control.address,
        max_config_bytes: config.config.max_bytes,
        write_enabled: config.config.write_enabled,
        verifier,
        transactions,
        rate: Mutex::new(HashMap::new()),
    });
    let instance_id = state.instance_id.clone();
    let app = router(state);
    let listener = TcpListener::bind(config.listen)
        .await
        .map_err(|err| format!("cannot bind control Agent {}: {err}", config.listen))?;
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    hbb_common::log::info!(
        "Starry Control Agent {} listening with mandatory mTLS on {}",
        instance_id,
        config.listen
    );

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|err| format!("control Agent accept failed: {err}"))?;
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let acceptor = acceptor.clone();
        let app = app.clone();
        let allowed_sans = allowed_sans.clone();
        hbb_common::tokio::spawn(async move {
            let Ok(Ok(tls_stream)) =
                timeout(Duration::from_secs(10), acceptor.accept(stream)).await
            else {
                return;
            };
            let certificate = tls_stream
                .get_ref()
                .1
                .peer_certificates()
                .and_then(|certificates| certificates.first())
                .and_then(|certificate| allowed_uri_san(certificate.as_ref(), &allowed_sans));
            let service = service_fn(move |mut request| {
                let response_request_id = request_id(request.headers());
                if let Ok(value) = HeaderValue::from_str(&response_request_id) {
                    request.headers_mut().insert("x-request-id", value);
                }
                request.extensions_mut().insert(ClientCertificate {
                    uri_san: certificate.clone(),
                });
                let app = app.clone();
                async move {
                    let response = app.oneshot(request).await;
                    let mut response = response.unwrap_or_else(|error| match error {});
                    if let Ok(value) = HeaderValue::from_str(&response_request_id) {
                        response.headers_mut().insert("x-request-id", value);
                    }
                    Ok::<_, Infallible>(response)
                }
            });
            let mut http = hyper::server::conn::Http::new();
            http.http1_only(true).http1_keep_alive(false);
            let _ = timeout(
                Duration::from_secs(CONNECTION_DEADLINE_SECONDS),
                http.serve_connection(tls_stream, service),
            )
            .await;
            drop(permit);
        });
    }
}

fn router(state: Arc<AgentState>) -> Router {
    Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/control/v1/capabilities", get(capabilities))
        .route("/control/v1/status", get(status))
        .route("/control/v1/relays", get(relays))
        .route("/control/v1/config/schema", get(config_schema))
        .route("/control/v1/config", get(config_get))
        .route("/control/v1/config/history", get(config_history))
        .route("/control/v1/operations/:id", get(operation_get))
        // matchit 0.5 treats every colon in a route pattern as a wildcard,
        // including the literal action separator used by this API. One exact
        // action dispatcher preserves the public paths without ambiguous routes.
        .route("/control/v1/:action", post(control_action))
        .fallback(axum::routing::any(not_found))
        .layer(Extension(state))
}

async fn control_action(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
    AxumPath(action): AxumPath<String>,
    RawBody(mut body): RawBody<Body>,
) -> Response {
    let request_id = request_id(&headers);
    let (scope, mutation) = match action.as_str() {
        "peers:verify" => ("starry.peer.verify", false),
        "allocations:simulate" => ("starry.relay.simulate", false),
        "config:validate" => ("starry.config.validate", false),
        "config:plan" => ("starry.config.plan", true),
        "config:apply" => ("starry.config.apply", true),
        "config:rollback" => ("starry.config.rollback", true),
        "runtime:reload" => ("starry.runtime.reload", true),
        _ => return not_found(headers).await.into_response(),
    };
    if let Err(problem) = verify_principal(&state, &certificate, &headers, scope, &request_id) {
        return problem.into_response();
    }
    if mutation && !state.write_enabled {
        return ApiProblem::new(
            404,
            "REQUEST_INVALID",
            "The requested control endpoint is disabled by the read-only profile.",
            false,
            request_id,
        )
        .into_response();
    }
    let declared_length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let Some(declared_length) = declared_length else {
        return ApiProblem::new(
            411,
            "REQUEST_INVALID",
            "Content-Length is required for control action requests.",
            false,
            request_id,
        )
        .into_response();
    };
    if declared_length > CONTROL_BODY_LIMIT {
        return ApiProblem::new(
            413,
            "CONFIG_TOO_LARGE",
            "The control request body exceeds the protocol limit.",
            false,
            request_id,
        )
        .into_response();
    }
    let mut bytes = Vec::with_capacity(declared_length as usize);
    while let Some(chunk) = body.data().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => return invalid_json(&headers).into_response(),
        };
        if bytes.len().saturating_add(chunk.len()) > CONTROL_BODY_LIMIT as usize {
            return ApiProblem::new(
                413,
                "CONFIG_TOO_LARGE",
                "The control request body exceeds the protocol limit.",
                false,
                request_id,
            )
            .into_response();
        }
        bytes.extend_from_slice(&chunk);
    }
    macro_rules! decoded {
        ($type:ty, $handler:ident) => {
            match serde_json::from_slice::<$type>(&bytes) {
                Ok(value) => $handler(
                    Extension(state),
                    Extension(certificate),
                    headers,
                    ContentLengthLimit(Json(value)),
                )
                .await
                .into_response(),
                Err(_) => invalid_json(&headers).into_response(),
            }
        };
    }
    match action.as_str() {
        "peers:verify" => decoded!(PeerVerifyRequest, peer_verify),
        "allocations:simulate" => decoded!(Value, simulate),
        "config:validate" => decoded!(ConfigCandidate, config_validate),
        "config:plan" => decoded!(ConfigCandidate, config_plan),
        "config:apply" => decoded!(ConfigApplyRequest, config_apply),
        "config:rollback" => decoded!(ConfigRollbackRequest, config_rollback),
        "runtime:reload" => decoded!(RuntimeReloadRequest, runtime_reload),
        _ => not_found(headers).await.into_response(),
    }
}

fn invalid_json(headers: &HeaderMap) -> ApiProblem {
    ApiProblem::new(
        400,
        "REQUEST_INVALID",
        "The control request body is not valid JSON for this endpoint.",
        false,
        request_id(headers),
    )
}

async fn health_live() -> impl IntoResponse {
    Json(json!({"status": "live"}))
}

async fn health_ready(Extension(state): Extension<Arc<AgentState>>) -> Response {
    let request_id = new_request_id();
    match local_client::call(state.local_address, &request_id, "status", json!({})).await {
        Ok(status) if status.get("ready").and_then(Value::as_bool) == Some(true) => {
            (StatusCode::OK, Json(json!({"status": "ready"}))).into_response()
        }
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "not_ready"})),
        )
            .into_response(),
    }
}

async fn capabilities(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiProblem> {
    let request_id = request_id(&headers);
    authorize(
        &state,
        &certificate,
        &headers,
        "starry.control.read",
        &request_id,
    )?;
    let _ = local_client::call(state.local_address, &request_id, "capabilities", json!({}))
        .await
        .map_err(|error| ApiProblem::from_agent(error, request_id.clone()))?;
    let runtime = local_client::call(
        state.local_address,
        &request_id,
        "config.runtime_state",
        json!({}),
    )
    .await
    .map_err(|error| ApiProblem::from_agent(error, request_id.clone()))?;
    let active_schema = runtime
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(4);
    let mut capabilities = json!({
        "relay_inventory": 1,
        "allocation_simulation": 1,
        "connection_auth": 1,
        "relay_quality": 1,
        "relay_active_probe": 1,
        "relay_probe_protocol": 1,
        "relay_load_protocol": 1,
        "relay_telemetry_schema": 1,
        "fast_relay_authorization": 1,
        "profile_activation_lease": 1,
        "peer_registry": 2
    });
    if state.write_enabled {
        let object = capabilities
            .as_object_mut()
            .expect("static capabilities value is an object");
        object.insert("config_transaction".to_owned(), json!(1));
        object.insert("config_rollback".to_owned(), json!(1));
    }
    Ok(Json(json!({
        "protocol": {"name": "starry-control", "version": "1.0.0", "major": 1},
        "instance": {
            "id": state.instance_id,
            "role": "hbbs",
            "starry_version": starry_version(),
            "upstream_version": env!("CARGO_PKG_VERSION")
        },
        "capabilities": capabilities,
        "config": {
            "supported_schema_versions": [1, 2, 3, 4],
            "active_schema_version": active_schema,
            "schema_digest": digest(CONTROL_SCHEMA.as_bytes())
        },
        "limits": {
            "max_config_bytes": state.max_config_bytes,
            "max_plan_lifetime_seconds": 600,
            "operation_retention_seconds": 86400
        }
    })))
}

async fn status(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiProblem> {
    proxy_empty(state, certificate, headers, "starry.control.read", "status").await
}

async fn relays(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiProblem> {
    proxy_empty(state, certificate, headers, "starry.relay.read", "relays").await
}

async fn simulate(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
    ContentLengthLimit(Json(params)): ContentLengthLimit<Json<Value>, CONTROL_BODY_LIMIT>,
) -> Result<Json<Value>, ApiProblem> {
    let request_id = request_id(&headers);
    authorize(
        &state,
        &certificate,
        &headers,
        "starry.relay.simulate",
        &request_id,
    )?;
    let result = local_client::call(
        state.local_address,
        &request_id,
        "allocation.simulate",
        params,
    )
    .await
    .map_err(|error| ApiProblem::from_agent(error, request_id))?;
    Ok(Json(result))
}

async fn peer_verify(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
    ContentLengthLimit(Json(params)): ContentLengthLimit<
        Json<PeerVerifyRequest>,
        CONTROL_BODY_LIMIT,
    >,
) -> Result<Json<Value>, ApiProblem> {
    let request_id = request_id(&headers);
    authorize(
        &state,
        &certificate,
        &headers,
        "starry.peer.verify",
        &request_id,
    )?;
    let mut result = local_client::call(
        state.local_address,
        &request_id,
        "peer.verify",
        serde_json::to_value(params).map_err(|_| ApiProblem::internal(request_id.clone()))?,
    )
    .await
    .map_err(|error| ApiProblem::from_agent(error, request_id.clone()))?;
    let object = result
        .as_object_mut()
        .ok_or_else(|| ApiProblem::internal(request_id.clone()))?;
    object.insert(
        "instance_id".to_owned(),
        Value::String(state.instance_id.clone()),
    );
    Ok(Json(result))
}

async fn config_schema(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
) -> Result<Response, ApiProblem> {
    let request_id = request_id(&headers);
    authorize(
        &state,
        &certificate,
        &headers,
        "starry.config.read",
        &request_id,
    )?;
    let schema: Value = serde_json::from_str(CONTROL_SCHEMA)
        .map_err(|_| ApiProblem::internal(request_id.clone()))?;
    let ui_schema: Value = serde_json::from_str(CONTROL_UI_SCHEMA)
        .map_err(|_| ApiProblem::internal(request_id.clone()))?;
    Ok(json_with_etag(
        json!({"schema": schema, "ui_schema": ui_schema}),
        &digest(CONTROL_SCHEMA.as_bytes()),
    ))
}

async fn config_get(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
) -> Result<Response, ApiProblem> {
    let request_id = request_id(&headers);
    authorize(
        &state,
        &certificate,
        &headers,
        "starry.config.read",
        &request_id,
    )?;
    let raw = state
        .transactions
        .read_config()
        .map_err(|error| ApiProblem::from_transaction(error, request_id.clone()))?;
    let document = String::from_utf8(raw.clone()).map_err(|_| {
        ApiProblem::new(
            409,
            "CONFIG_INVALID",
            "The managed configuration is not valid UTF-8 YAML and must be repaired locally.",
            false,
            request_id.clone(),
        )
    })?;
    let etag = digest(&raw);
    let mut runtime = local_client::call(
        state.local_address,
        &request_id,
        "config.runtime_state",
        json!({}),
    )
    .await
    .map_err(|error| ApiProblem::from_agent(error, request_id))?;
    let runtime_digest = runtime.get("source_digest").and_then(Value::as_str);
    let drift = if raw.is_empty() {
        runtime_digest.is_some()
    } else {
        runtime_digest != Some(etag.as_str())
    };
    let object = runtime
        .as_object_mut()
        .ok_or_else(|| ApiProblem::internal(new_request_id()))?;
    object.insert("etag".to_owned(), Value::String(format!("\"{etag}\"")));
    object.insert("drift".to_owned(), Value::Bool(drift));
    object.insert("document".to_owned(), Value::String(document));
    object.insert("format".to_owned(), Value::String("yaml".to_owned()));
    Ok(json_with_etag(runtime, &etag))
}

async fn config_validate(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
    ContentLengthLimit(Json(candidate)): ContentLengthLimit<
        Json<ConfigCandidate>,
        CONTROL_BODY_LIMIT,
    >,
) -> Result<Json<Value>, ApiProblem> {
    let request_id = request_id(&headers);
    authorize(
        &state,
        &certificate,
        &headers,
        "starry.config.validate",
        &request_id,
    )?;
    if candidate.format != "yaml" {
        return Err(ApiProblem::new(
            400,
            "REQUEST_INVALID",
            "Configuration format must be yaml.",
            false,
            request_id,
        ));
    }
    if candidate.document.len() > state.max_config_bytes {
        return Err(ApiProblem::new(
            413,
            "CONFIG_TOO_LARGE",
            "The configuration candidate exceeds the configured byte limit.",
            false,
            request_id,
        ));
    }
    let parsed = match crate::starry_config::parse_document(candidate.document.as_bytes()) {
        Ok(parsed) => parsed,
        Err(diagnostics) => {
            return Ok(Json(json!({
                "valid": false,
                "source_digest": null,
                "effective_digest": null,
                "diagnostics": diagnostics.errors
            })))
        }
    };
    match crate::starry_config::validate_config(parsed) {
        Ok(validated) => Ok(Json(json!({
            "valid": true,
            "source_digest": validated.source_digest,
            "effective_digest": validated.effective_digest,
            "diagnostics": []
        }))),
        Err(diagnostics) => Ok(Json(json!({
            "valid": false,
            "source_digest": null,
            "effective_digest": null,
            "diagnostics": diagnostics.errors
        }))),
    }
}

async fn config_plan(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
    ContentLengthLimit(Json(candidate)): ContentLengthLimit<
        Json<ConfigCandidate>,
        CONTROL_BODY_LIMIT,
    >,
) -> Result<Json<Value>, ApiProblem> {
    let request_id = request_id(&headers);
    let principal = authorize(
        &state,
        &certificate,
        &headers,
        "starry.config.plan",
        &request_id,
    )?;
    let supplied_etag = required_if_match(&headers, &request_id)?;
    let document = candidate_bytes(candidate, state.max_config_bytes, &request_id)?;
    let validated = validate_candidate_for_write(&document, &request_id)?;
    let runtime = runtime_snapshot(&state, &request_id).await?;
    let plan = state
        .transactions
        .create_plan(
            document,
            &validated,
            &supplied_etag,
            runtime,
            transaction_caller(principal, &headers),
        )
        .map_err(|error| ApiProblem::from_transaction(error, request_id))?;
    Ok(Json(plan))
}

async fn config_apply(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
    ContentLengthLimit(Json(request)): ContentLengthLimit<
        Json<ConfigApplyRequest>,
        CONTROL_BODY_LIMIT,
    >,
) -> Result<Response, ApiProblem> {
    let request_id = request_id(&headers);
    let principal = authorize(
        &state,
        &certificate,
        &headers,
        "starry.config.apply",
        &request_id,
    )?;
    uuid::Uuid::parse_str(&request.plan_id).map_err(|_| {
        ApiProblem::new(
            400,
            "REQUEST_INVALID",
            "plan_id must be a UUID.",
            false,
            request_id.clone(),
        )
    })?;
    config_store::validate_digest(&request.candidate_digest)
        .map_err(|error| ApiProblem::from_transaction(error, request_id.clone()))?;
    let supplied_etag = required_if_match(&headers, &request_id)?;
    let idempotency_key = required_idempotency_key(&headers, &request_id)?;
    let runtime = runtime_snapshot(&state, &request_id).await?;
    let operation = state
        .transactions
        .accept_apply(
            &request.plan_id,
            &request.candidate_digest,
            &supplied_etag,
            &idempotency_key,
            request.comment.as_deref(),
            &transaction_caller(principal, &headers),
            runtime,
            &request_id,
            state.local_address,
        )
        .map_err(|error| ApiProblem::from_transaction(error, request_id))?;
    Ok((StatusCode::ACCEPTED, Json(operation)).into_response())
}

async fn config_history(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiProblem> {
    let request_id = request_id(&headers);
    authorize(
        &state,
        &certificate,
        &headers,
        "starry.config.read",
        &request_id,
    )?;
    let revisions = state
        .transactions
        .history()
        .map_err(|error| ApiProblem::from_transaction(error, request_id))?;
    Ok(Json(json!({"revisions": revisions})))
}

async fn config_rollback(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
    ContentLengthLimit(Json(request)): ContentLengthLimit<
        Json<ConfigRollbackRequest>,
        CONTROL_BODY_LIMIT,
    >,
) -> Result<Response, ApiProblem> {
    let request_id = request_id(&headers);
    let principal = authorize(
        &state,
        &certificate,
        &headers,
        "starry.config.rollback",
        &request_id,
    )?;
    let supplied_etag = required_if_match(&headers, &request_id)?;
    let idempotency_key = required_idempotency_key(&headers, &request_id)?;
    let runtime = runtime_snapshot(&state, &request_id).await?;
    let operation = state
        .transactions
        .accept_rollback(
            &request.revision_id,
            &supplied_etag,
            &idempotency_key,
            request.comment.as_deref(),
            &transaction_caller(principal, &headers),
            runtime,
            &request_id,
            state.local_address,
        )
        .map_err(|error| ApiProblem::from_transaction(error, request_id))?;
    Ok((StatusCode::ACCEPTED, Json(operation)).into_response())
}

async fn operation_get(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<config_store::Operation>, ApiProblem> {
    let request_id = request_id(&headers);
    authorize(
        &state,
        &certificate,
        &headers,
        "starry.control.read",
        &request_id,
    )?;
    state
        .transactions
        .operation(&id)
        .map(Json)
        .map_err(|error| ApiProblem::from_transaction(error, request_id))
}

async fn runtime_reload(
    Extension(state): Extension<Arc<AgentState>>,
    Extension(certificate): Extension<ClientCertificate>,
    headers: HeaderMap,
    ContentLengthLimit(Json(request)): ContentLengthLimit<
        Json<RuntimeReloadRequest>,
        CONTROL_BODY_LIMIT,
    >,
) -> Result<Json<Value>, ApiProblem> {
    let request_id = request_id(&headers);
    let principal = authorize(
        &state,
        &certificate,
        &headers,
        "starry.runtime.reload",
        &request_id,
    )?;
    let idempotency_key = required_idempotency_key(&headers, &request_id)?;
    state
        .transactions
        .runtime_reload(
            &request.expected_source_digest,
            &idempotency_key,
            &request_id,
            &transaction_caller(principal, &headers),
            state.local_address,
        )
        .await
        .map(Json)
        .map_err(|error| ApiProblem::from_transaction(error, request_id))
}

fn candidate_bytes(
    candidate: ConfigCandidate,
    max_bytes: usize,
    request_id: &str,
) -> Result<Vec<u8>, ApiProblem> {
    if candidate.format != "yaml" {
        return Err(ApiProblem::new(
            400,
            "REQUEST_INVALID",
            "Configuration format must be yaml.",
            false,
            request_id.to_owned(),
        ));
    }
    let document = candidate.document.into_bytes();
    if document.len() > max_bytes {
        return Err(ApiProblem::new(
            413,
            "CONFIG_TOO_LARGE",
            "The configuration candidate exceeds the configured byte limit.",
            false,
            request_id.to_owned(),
        ));
    }
    Ok(document)
}

fn validate_candidate_for_write(
    document: &[u8],
    request_id: &str,
) -> Result<crate::starry_config::ValidatedConfig, ApiProblem> {
    crate::starry_config::parse_document(document)
        .and_then(crate::starry_config::validate_config)
        .map_err(|diagnostics| {
            let errors = serde_json::to_value(diagnostics.errors)
                .ok()
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default();
            ApiProblem::new(
                400,
                "CONFIG_INVALID",
                "The configuration candidate is invalid.",
                false,
                request_id.to_owned(),
            )
            .with_errors(errors)
        })
}

async fn runtime_snapshot(
    state: &AgentState,
    request_id: &str,
) -> Result<RuntimeSnapshot, ApiProblem> {
    let value = local_client::call(
        state.local_address,
        request_id,
        "config.runtime_state",
        json!({}),
    )
    .await
    .map_err(|error| ApiProblem::from_agent(error, request_id.to_owned()))?;
    RuntimeSnapshot::from_value(&value)
        .map_err(|error| ApiProblem::from_transaction(error, request_id.to_owned()))
}

fn transaction_caller(principal: ControlPrincipal, headers: &HeaderMap) -> TransactionCaller {
    TransactionCaller {
        service: principal.service,
        actor: principal.actor,
        certificate_uri_san: principal.certificate_uri_san,
        traceparent: valid_traceparent(headers),
    }
}

fn valid_traceparent(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("traceparent")?.to_str().ok()?;
    let mut parts = value.split('-');
    let version = parts.next()?;
    let trace_id = parts.next()?;
    let parent_id = parts.next()?;
    let flags = parts.next()?;
    if parts.next().is_some()
        || version.len() != 2
        || version == "ff"
        || trace_id.len() != 32
        || trace_id.bytes().all(|byte| byte == b'0')
        || parent_id.len() != 16
        || parent_id.bytes().all(|byte| byte == b'0')
        || flags.len() != 2
        || !value
            .bytes()
            .filter(|byte| *byte != b'-')
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    Some(value.to_owned())
}

fn required_if_match(headers: &HeaderMap, request_id: &str) -> Result<String, ApiProblem> {
    let value = headers
        .get(header::IF_MATCH)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiProblem::new(
                428,
                "PRECONDITION_REQUIRED",
                "If-Match is required for this configuration request.",
                false,
                request_id.to_owned(),
            )
        })?;
    if value.len() != 73
        || !value.starts_with("\"sha256:")
        || !value.ends_with('"')
        || !value[8..72]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ApiProblem::new(
            400,
            "REQUEST_INVALID",
            "If-Match must contain one strong Starry SHA-256 ETag.",
            false,
            request_id.to_owned(),
        ));
    }
    Ok(value.to_owned())
}

fn required_idempotency_key(headers: &HeaderMap, request_id: &str) -> Result<String, ApiProblem> {
    let value = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiProblem::new(
                428,
                "PRECONDITION_REQUIRED",
                "Idempotency-Key is required for this request.",
                false,
                request_id.to_owned(),
            )
        })?;
    config_store::validate_idempotency_key(value)
        .map_err(|error| ApiProblem::from_transaction(error, request_id.to_owned()))?;
    Ok(value.to_owned())
}

async fn proxy_empty(
    state: Arc<AgentState>,
    certificate: ClientCertificate,
    headers: HeaderMap,
    scope: &'static str,
    method: &'static str,
) -> Result<Json<Value>, ApiProblem> {
    let request_id = request_id(&headers);
    authorize(&state, &certificate, &headers, scope, &request_id)?;
    let result = local_client::call(state.local_address, &request_id, method, json!({}))
        .await
        .map_err(|error| ApiProblem::from_agent(error, request_id))?;
    Ok(Json(result))
}

async fn not_found(headers: HeaderMap) -> ApiProblem {
    ApiProblem::new(
        404,
        "REQUEST_INVALID",
        "The requested control endpoint does not exist.",
        false,
        request_id(&headers),
    )
}

fn authorize(
    state: &AgentState,
    certificate: &ClientCertificate,
    headers: &HeaderMap,
    scope: &str,
    request_id: &str,
) -> Result<ControlPrincipal, ApiProblem> {
    let principal = verify_principal(state, certificate, headers, scope, request_id)?;
    let key = format!("{}\0{}", principal.certificate_uri_san, principal.service);
    let mut rate = state
        .rate
        .lock()
        .map_err(|_| ApiProblem::internal(request_id.to_owned()))?;
    let entry = rate.entry(key).or_insert(RateWindow {
        started: Instant::now(),
        requests: 0,
    });
    if entry.started.elapsed() >= Duration::from_secs(60) {
        entry.started = Instant::now();
        entry.requests = 0;
    }
    if entry.requests >= REQUESTS_PER_MINUTE {
        return Err(ApiProblem::new(
            429,
            "REQUEST_INVALID",
            "The control request rate limit was exceeded.",
            true,
            request_id.to_owned(),
        ));
    }
    entry.requests += 1;
    hbb_common::log::info!(
        "Control request authorized: request_id={} service={} actor={} cert_uri={} scope={}",
        request_id,
        principal.service,
        principal.actor,
        principal.certificate_uri_san,
        scope
    );
    Ok(principal)
}

fn verify_principal(
    state: &AgentState,
    certificate: &ClientCertificate,
    headers: &HeaderMap,
    scope: &str,
    request_id: &str,
) -> Result<ControlPrincipal, ApiProblem> {
    let bearer = bearer_token(headers);
    state
        .verifier
        .verify(bearer, certificate.uri_san.as_deref(), scope)
        .map_err(|failure| ApiProblem::from_auth(failure, request_id.to_owned()))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("Bearer") && !token.contains(char::is_whitespace)).then_some(token)
}

fn validate_config(config: &AgentConfig) -> Result<(), String> {
    if config.version != 1 {
        return Err("control Agent config version must be 1".to_owned());
    }
    if !config.local_control.address.ip().is_loopback() {
        return Err("local_control.address must be loopback".to_owned());
    }
    if config.local_control.token_file.trim().is_empty() {
        return Err("local_control.token_file must not be empty".to_owned());
    }
    if config.config.max_bytes == 0 || config.config.max_bytes > 1024 * 1024 {
        return Err("config.max_bytes must be between 1 and 1048576".to_owned());
    }
    let issuer = url::Url::parse(&config.service_jwt.issuer)
        .map_err(|err| format!("invalid service_jwt.issuer: {err}"))?;
    if issuer.scheme() != "https"
        || issuer.host_str().is_none()
        || !issuer.username().is_empty()
        || issuer.password().is_some()
        || issuer.query().is_some()
        || issuer.fragment().is_some()
    {
        return Err("service_jwt.issuer must be an HTTPS issuer URL".to_owned());
    }
    if config.tls.allowed_client_uri_sans.is_empty() {
        return Err("tls.allowed_client_uri_sans must not be empty".to_owned());
    }
    let mut seen = HashSet::new();
    for san in &config.tls.allowed_client_uri_sans {
        let parsed =
            url::Url::parse(san).map_err(|err| format!("invalid allowed client URI SAN: {err}"))?;
        if parsed.scheme().is_empty() || parsed.host_str().is_none() || !seen.insert(san.to_owned())
        {
            return Err("allowed client URI SANs must be unique absolute URIs".to_owned());
        }
    }
    if !config.service_jwt.audience_prefix.ends_with(':')
        || config.service_jwt.audience_prefix.len() > 128
    {
        return Err(
            "service_jwt.audience_prefix must be a bounded URN prefix ending in ':'".to_owned(),
        );
    }
    for value in [
        &config.instance_id_file,
        &config.tls.ca_file,
        &config.tls.cert_file,
        &config.tls.key_file,
        &config.service_jwt.jwks_file,
        &config.config.path,
        &config.config.backup_dir,
    ] {
        if value.trim().is_empty() || value.chars().any(|ch| matches!(ch, '\0' | '\n' | '\r')) {
            return Err(
                "control Agent file references must be non-empty single-line paths".to_owned(),
            );
        }
    }
    Ok(())
}

fn load_tls(config: &TlsConfig, base: &Path) -> Result<ServerConfig, String> {
    let certs = read_certificates(&resolve_path(base, &config.cert_file))?;
    let key = read_private_key(&resolve_path(base, &config.key_file))?;
    let ca = read_certificates(&resolve_path(base, &config.ca_file))?;
    let mut roots = RootCertStore::empty();
    let (added, _) = roots.add_parsable_certificates(ca);
    if added == 0 {
        return Err("control Agent client CA contains no usable certificate".to_owned());
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|err| format!("invalid control Agent client CA: {err}"))?;
    let mut server = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .map_err(|err| format!("invalid control Agent TLS identity: {err}"))?;
    server.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(server)
}

fn read_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let certs = CertificateDer::pem_file_iter(path)
        .map_err(|err| format!("cannot read TLS certificate {}: {err}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("invalid PEM certificate {}: {err}", path.display()))?;
    if certs.is_empty() {
        return Err(format!("TLS certificate file {} is empty", path.display()));
    }
    Ok(certs)
}

fn read_private_key(path: &Path) -> Result<PrivateKeyDer<'static>, String> {
    PrivateKeyDer::from_pem_file(path)
        .map_err(|err| format!("invalid TLS private key {}: {err}", path.display()))
}

fn allowed_uri_san(raw: &[u8], allowed: &HashSet<String>) -> Option<String> {
    let (_, certificate) = parse_x509_certificate(raw).ok()?;
    let san = certificate.subject_alternative_name().ok()??;
    let matches: HashSet<String> = san
        .value
        .general_names
        .iter()
        .filter_map(|name| match name {
            GeneralName::URI(uri) if allowed.contains(*uri) => Some((*uri).to_owned()),
            _ => None,
        })
        .collect();
    (matches.len() == 1)
        .then(|| matches.into_iter().next())
        .flatten()
}

fn load_or_create_instance_id(path: &Path) -> Result<String, String> {
    if let Ok(raw) = fs::read_to_string(path) {
        return parse_instance_id(&raw);
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|err| format!("cannot create instance ID directory: {err}"))?;
    let id = uuid::Uuid::now_v7().to_string();
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(id.as_bytes())
                .and_then(|_| file.write_all(b"\n"))
                .and_then(|_| file.sync_all())
                .map_err(|err| format!("cannot persist instance ID: {err}"))?;
            #[cfg(unix)]
            fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|err| format!("cannot fsync instance ID directory: {err}"))?;
            Ok(id)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let raw = fs::read_to_string(path)
                .map_err(|err| format!("cannot read concurrently created instance ID: {err}"))?;
            parse_instance_id(&raw)
        }
        Err(err) => Err(format!("cannot create instance ID: {err}")),
    }
}

fn parse_instance_id(raw: &str) -> Result<String, String> {
    let id = raw.trim();
    let parsed = uuid::Uuid::parse_str(id).map_err(|_| "instance ID file is invalid".to_owned())?;
    if parsed.get_version_num() != 7 {
        return Err("instance ID must be UUIDv7".to_owned());
    }
    Ok(parsed.to_string())
}

pub(super) fn resolve_path(base: &Path, configured: &str) -> PathBuf {
    let path = PathBuf::from(configured);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn digest(raw: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(raw))
}

fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .map(|value| value.to_string())
        .unwrap_or_else(new_request_id)
}

fn new_request_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn json_with_etag(value: Value, etag: &str) -> Response {
    let mut response = Json(value).into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("\"{etag}\"")) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

struct ApiProblem(Problem);

impl ApiProblem {
    fn new(
        status: u16,
        code: impl Into<String>,
        detail: impl Into<String>,
        retryable: bool,
        request_id: String,
    ) -> Self {
        let code = code.into();
        Self(Problem {
            problem_type: format!(
                "https://starry.invalid/problems/{}",
                code.to_ascii_lowercase().replace('_', "-")
            ),
            title: problem_title(&code).to_owned(),
            status,
            code,
            detail: detail.into(),
            request_id,
            retryable,
            errors: Vec::new(),
        })
    }

    fn from_agent(error: AgentError, request_id: String) -> Self {
        let status = match error.code.as_str() {
            "CONFIG_TOO_LARGE" => 413,
            "STARRY_NOT_READY" | "LOCAL_CONTROL_UNAVAILABLE" | "LOCAL_CONTROL_TIMEOUT" => 503,
            "IP_INVALID" | "TRANSPORT_INVALID" | "REQUEST_INVALID" => 400,
            _ => 502,
        };
        Self::new(
            status,
            error.code,
            error.detail,
            error.retryable,
            request_id,
        )
    }

    fn from_auth(error: AuthFailure, request_id: String) -> Self {
        Self::new(error.status, error.code, error.detail, false, request_id)
    }

    fn from_transaction(error: TransactionError, request_id: String) -> Self {
        Self::new(
            error.status,
            error.code,
            error.detail,
            error.retryable,
            request_id,
        )
        .with_errors(error.errors)
    }

    fn with_errors(mut self, errors: Vec<Value>) -> Self {
        self.0.errors = errors;
        self
    }

    fn internal(request_id: String) -> Self {
        Self::new(
            500,
            "STARRY_NOT_READY",
            "The control Agent cannot complete the request.",
            true,
            request_id,
        )
    }
}

impl IntoResponse for ApiProblem {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Json(self.0)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

impl AgentError {
    pub(super) fn new(code: impl Into<String>, detail: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
            retryable,
        }
    }

    pub(super) fn internal(detail: impl Into<String>) -> Self {
        Self::new("STARRY_NOT_READY", detail, true)
    }

    pub(super) fn local_unavailable() -> Self {
        Self::new(
            "LOCAL_CONTROL_UNAVAILABLE",
            "HBBS local control is unavailable.",
            true,
        )
    }

    pub(super) fn local_timeout() -> Self {
        Self::new(
            "LOCAL_CONTROL_TIMEOUT",
            "HBBS local control timed out.",
            true,
        )
    }
}

fn problem_title(code: &str) -> &'static str {
    match code {
        "AUTH_REQUIRED" => "Authentication required",
        "TOKEN_INVALID" | "AUTH_KEY_UNAVAILABLE" => "Invalid authentication",
        "TOKEN_EXPIRED" => "Authentication expired",
        "CLIENT_CERT_DENIED" => "Client certificate denied",
        "SCOPE_DENIED" => "Scope denied",
        "LOCAL_CONTROL_UNAVAILABLE" | "LOCAL_CONTROL_TIMEOUT" => "HBBS unavailable",
        "CONFIG_TOO_LARGE" | "CONFIG_INVALID" => "Invalid configuration",
        "CONFIG_ETAG_MISMATCH" => "Configuration changed",
        "PRECONDITION_REQUIRED" => "Precondition required",
        "PLAN_EXPIRED" => "Plan expired",
        "PLAN_STALE" => "Plan stale",
        "RESTART_REQUIRED" => "Restart required",
        "OPERATION_IN_PROGRESS" => "Operation in progress",
        "IDEMPOTENCY_KEY_REUSED" => "Idempotency key reused",
        "ROLLBACK_FAILED" => "Rollback failed",
        "STARRY_NOT_READY" => "Starry is not ready",
        _ => "Invalid request",
    }
}

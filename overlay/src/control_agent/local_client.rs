use super::AgentError;
use crate::local_control::{MAGIC, MAX_FRAME_BYTES};
use hbb_common::{
    timeout,
    tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    },
};
use serde_json::Value;
use std::{net::SocketAddr, path::PathBuf, sync::OnceLock};

const LOCAL_TIMEOUT_MS: u64 = 5_000;
static AUTH_TOKEN_FILE: OnceLock<PathBuf> = OnceLock::new();

pub(super) fn configure_auth_token_file(path: PathBuf) -> Result<(), String> {
    crate::local_control::read_auth_token_file(&path)?;
    AUTH_TOKEN_FILE
        .set(path)
        .map_err(|_| "local control token file was configured more than once".to_owned())
}

pub(super) async fn call(
    address: SocketAddr,
    request_id: &str,
    method: &str,
    params: Value,
) -> Result<Value, AgentError> {
    let token_path = AUTH_TOKEN_FILE.get().ok_or_else(|| {
        AgentError::new(
            "LOCAL_CONTROL_UNAUTHORIZED",
            "The local control authentication token is not configured.",
            false,
        )
    })?;
    let auth_token = crate::local_control::read_auth_token_file(token_path).map_err(|_| {
        AgentError::new(
            "LOCAL_CONTROL_UNAUTHORIZED",
            "The local control authentication token is unavailable.",
            false,
        )
    })?;
    let payload = serde_json::to_vec(&serde_json::json!({
        "request_id": request_id,
        "method": method,
        "auth_token": auth_token,
        "params": params
    }))
    .map_err(|_| AgentError::internal("cannot serialize local control request"))?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(AgentError::new(
            "CONFIG_TOO_LARGE",
            "The local control request exceeds the protocol limit.",
            false,
        ));
    }
    let mut stream = timeout(LOCAL_TIMEOUT_MS, TcpStream::connect(address))
        .await
        .map_err(|_| AgentError::local_timeout())?
        .map_err(|_| AgentError::local_unavailable())?;
    let mut frame = Vec::with_capacity(MAGIC.len() + 4 + payload.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    timeout(LOCAL_TIMEOUT_MS, stream.write_all(&frame))
        .await
        .map_err(|_| AgentError::local_timeout())?
        .map_err(|_| AgentError::local_unavailable())?;

    let mut magic = vec![0_u8; MAGIC.len()];
    timeout(LOCAL_TIMEOUT_MS, stream.read_exact(&mut magic))
        .await
        .map_err(|_| AgentError::local_timeout())?
        .map_err(|_| AgentError::local_unavailable())?;
    if magic != MAGIC {
        return Err(AgentError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "HBBS returned an invalid local control magic.",
            false,
        ));
    }
    let mut length = [0_u8; 4];
    timeout(LOCAL_TIMEOUT_MS, stream.read_exact(&mut length))
        .await
        .map_err(|_| AgentError::local_timeout())?
        .map_err(|_| AgentError::local_unavailable())?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(AgentError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "HBBS returned an invalid local control frame length.",
            false,
        ));
    }
    let mut body = vec![0_u8; length];
    timeout(LOCAL_TIMEOUT_MS, stream.read_exact(&mut body))
        .await
        .map_err(|_| AgentError::local_timeout())?
        .map_err(|_| AgentError::local_unavailable())?;
    let response: Value = serde_json::from_slice(&body).map_err(|_| {
        AgentError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "HBBS returned invalid local control JSON.",
            false,
        )
    })?;
    if response.get("request_id").and_then(Value::as_str) != Some(request_id) {
        return Err(AgentError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "HBBS returned a mismatched local control request ID.",
            false,
        ));
    }
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        return response.get("result").cloned().ok_or_else(|| {
            AgentError::new(
                "LOCAL_CONTROL_PROTOCOL_ERROR",
                "HBBS omitted the local control result.",
                false,
            )
        });
    }
    let error = response.get("error").cloned().unwrap_or(Value::Null);
    Err(AgentError::new(
        error
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("LOCAL_CONTROL_PROTOCOL_ERROR"),
        error
            .get("detail")
            .and_then(Value::as_str)
            .unwrap_or("HBBS rejected the local control request."),
        error
            .get("retryable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    ))
}

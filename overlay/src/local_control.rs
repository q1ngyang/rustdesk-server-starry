use hbb_common::{
    timeout,
    tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use std::{fmt, fs, io, path::Path};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub(crate) const MAGIC: &[u8] = b"STARRYCTL/1\n";
pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub(crate) const AUTH_TOKEN_FILE_ENV: &str = "STARRY_LOCAL_CONTROL_TOKEN_FILE";
const INITIAL_READ_BYTES: usize = 1024;
const READ_TIMEOUT_MS: u64 = 5_000;
const WRITE_TIMEOUT_MS: u64 = 5_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub(crate) request_id: String,
    pub(crate) method: String,
    pub(crate) auth_token: String,
    #[serde(default = "empty_object")]
    pub(crate) params: Value,
}

impl fmt::Debug for Request {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Request")
            .field("request_id", &self.request_id)
            .field("method", &self.method)
            .field("auth_token", &"[REDACTED]")
            .field("params", &self.params)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum IncomingRequest {
    Framed(Request),
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ErrorBody {
    pub(crate) code: String,
    pub(crate) detail: String,
    pub(crate) retryable: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct Response {
    pub(crate) request_id: String,
    pub(crate) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<ErrorBody>,
}

impl Response {
    pub(crate) fn success(request_id: String, result: Value) -> Self {
        Self {
            request_id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub(crate) fn error(
        request_id: String,
        code: impl Into<String>,
        detail: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            request_id,
            ok: false,
            result: None,
            error: Some(ErrorBody {
                code: code.into(),
                detail: detail.into(),
                retryable,
            }),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ProtocolError {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
    /// The magic was recognized, so a structured response is safe.
    pub(crate) framed: bool,
}

impl ProtocolError {
    fn new(code: &'static str, detail: impl Into<String>, framed: bool) -> Self {
        Self {
            code,
            detail: detail.into(),
            framed,
        }
    }
}

pub(crate) async fn read_request<R>(reader: &mut R) -> Result<IncomingRequest, ProtocolError>
where
    R: AsyncRead + Unpin,
{
    timeout(READ_TIMEOUT_MS, read_request_inner(reader))
        .await
        .map_err(|_| {
            ProtocolError::new(
                "LOCAL_CONTROL_TIMEOUT",
                "local control request timed out",
                false,
            )
        })?
}

async fn read_request_inner<R>(reader: &mut R) -> Result<IncomingRequest, ProtocolError>
where
    R: AsyncRead + Unpin,
{
    let mut initial = vec![0_u8; INITIAL_READ_BYTES];
    let mut used = reader
        .read(&mut initial)
        .await
        .map_err(|err| io_error("cannot read local control request", err, false))?;
    if used == 0 {
        return Err(ProtocolError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "local control request is empty",
            false,
        ));
    }

    if used < MAGIC.len() && MAGIC.starts_with(&initial[..used]) {
        reader
            .read_exact(&mut initial[used..MAGIC.len()])
            .await
            .map_err(|err| io_error("truncated local control magic", err, true))?;
        used = MAGIC.len();
    }

    if used >= MAGIC.len() && &initial[..MAGIC.len()] == MAGIC {
        return read_framed_request(reader, &initial[..used]).await;
    }

    Err(ProtocolError::new(
        "LOCAL_CONTROL_PROTOCOL_ERROR",
        "legacy text control is disabled; STARRYCTL/1 framing is required",
        false,
    ))
}

async fn read_framed_request<R>(
    reader: &mut R,
    initial: &[u8],
) -> Result<IncomingRequest, ProtocolError>
where
    R: AsyncRead + Unpin,
{
    let mut cursor = MAGIC.len();
    let mut length_bytes = [0_u8; 4];
    read_exact_buffered(reader, initial, &mut cursor, &mut length_bytes).await?;
    let length = u32::from_be_bytes(length_bytes) as usize;
    if length == 0 {
        return Err(ProtocolError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "local control JSON payload is empty",
            true,
        ));
    }
    if length > MAX_FRAME_BYTES {
        return Err(ProtocolError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            format!("local control frame is {length} bytes; maximum is {MAX_FRAME_BYTES}"),
            true,
        ));
    }

    let mut payload = vec![0_u8; length];
    read_exact_buffered(reader, initial, &mut cursor, &mut payload).await?;
    if cursor < initial.len() {
        return Err(ProtocolError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "unexpected bytes follow the local control frame",
            true,
        ));
    }
    let request: Request = serde_json::from_slice(&payload).map_err(|err| {
        ProtocolError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            format!("invalid local control JSON: {err}"),
            true,
        )
    })?;
    validate_request(&request)?;
    Ok(IncomingRequest::Framed(request))
}

async fn read_exact_buffered<R>(
    reader: &mut R,
    initial: &[u8],
    cursor: &mut usize,
    output: &mut [u8],
) -> Result<(), ProtocolError>
where
    R: AsyncRead + Unpin,
{
    let available = initial.len().saturating_sub(*cursor).min(output.len());
    if available > 0 {
        output[..available].copy_from_slice(&initial[*cursor..*cursor + available]);
        *cursor += available;
    }
    if available < output.len() {
        reader
            .read_exact(&mut output[available..])
            .await
            .map_err(|err| io_error("truncated local control frame", err, true))?;
    }
    Ok(())
}

fn validate_request(request: &Request) -> Result<(), ProtocolError> {
    if request.request_id.is_empty() || request.request_id.len() > 128 {
        return Err(ProtocolError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "request_id must contain between 1 and 128 bytes",
            true,
        ));
    }
    if request.method.is_empty() || request.method.len() > 128 {
        return Err(ProtocolError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "method must contain between 1 and 128 bytes",
            true,
        ));
    }
    if request
        .request_id
        .chars()
        .chain(request.method.chars())
        .any(|ch| ch.is_control())
    {
        return Err(ProtocolError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "request_id and method must not contain control characters",
            true,
        ));
    }
    if !request.params.is_object() {
        return Err(ProtocolError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "params must be a JSON object",
            true,
        ));
    }
    if request.auth_token.len() < 32 || request.auth_token.len() > 256 {
        return Err(ProtocolError::new(
            "LOCAL_CONTROL_UNAUTHORIZED",
            "local control authentication failed",
            true,
        ));
    }
    Ok(())
}

pub(crate) fn authenticate(request: &Request) -> Result<(), ProtocolError> {
    let path = std::env::var_os(AUTH_TOKEN_FILE_ENV)
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(unauthorized)?;
    let expected = read_auth_token_file(&path).map_err(|_| unauthorized())?;
    if !constant_time_eq(expected.as_bytes(), request.auth_token.as_bytes()) {
        return Err(unauthorized());
    }
    Ok(())
}

pub(crate) fn read_auth_token_file(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|err| {
        format!(
            "cannot inspect local control token {}: {err}",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "local control token {} is not a regular file",
            path.display()
        ));
    }
    #[cfg(unix)]
    if metadata.mode() & 0o077 != 0 {
        return Err(format!(
            "local control token {} must not grant group or other permissions",
            path.display()
        ));
    }
    let raw = fs::read(path)
        .map_err(|err| format!("cannot read local control token {}: {err}", path.display()))?;
    let token = std::str::from_utf8(&raw)
        .map_err(|_| "local control token must be UTF-8".to_owned())?
        .trim();
    if token.len() < 32
        || token.len() > 256
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("local control token must be 32..256 base64url characters".to_owned());
    }
    Ok(token.to_owned())
}

fn unauthorized() -> ProtocolError {
    ProtocolError::new(
        "LOCAL_CONTROL_UNAUTHORIZED",
        "local control authentication failed",
        true,
    )
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        let left = left.get(index).copied().unwrap_or(0);
        let right = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(left ^ right);
    }
    difference == 0
}

pub(crate) async fn write_response<W>(
    writer: &mut W,
    response: &Response,
) -> Result<(), ProtocolError>
where
    W: AsyncWrite + Unpin,
{
    let payload = serde_json::to_vec(response).map_err(|err| {
        ProtocolError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            format!("cannot serialize local control response: {err}"),
            true,
        )
    })?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::new(
            "LOCAL_CONTROL_PROTOCOL_ERROR",
            "local control response exceeds the frame limit",
            true,
        ));
    }
    let mut frame = Vec::with_capacity(MAGIC.len() + 4 + payload.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    timeout(WRITE_TIMEOUT_MS, writer.write_all(&frame))
        .await
        .map_err(|_| {
            ProtocolError::new(
                "LOCAL_CONTROL_TIMEOUT",
                "local control response timed out",
                true,
            )
        })?
        .map_err(|err| io_error("cannot write local control response", err, true))
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

fn io_error(context: &str, error: io::Error, framed: bool) -> ProtocolError {
    ProtocolError::new(
        "LOCAL_CONTROL_PROTOCOL_ERROR",
        format!("{context}: {error}"),
        framed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::tokio::{
        io::{duplex, AsyncWriteExt},
        runtime::Builder,
    };

    fn request_payload() -> Vec<u8> {
        br#"{"request_id":"request-1","method":"status","auth_token":"abcdefghijklmnopqrstuvwxyzABCDEF","params":{}}"#.to_vec()
    }

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = MAGIC.to_vec();
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    #[test]
    fn accepts_fragmented_magic_length_and_json() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut client, mut server) = duplex(4096);
                let bytes = frame(&request_payload());
                let writer = hbb_common::tokio::spawn(async move {
                    for chunk in bytes.chunks(3) {
                        client.write_all(chunk).await.unwrap();
                        hbb_common::tokio::task::yield_now().await;
                    }
                });
                let request = read_request(&mut server).await.unwrap();
                writer.await.unwrap();
                match request {
                    IncomingRequest::Framed(request) => {
                        assert_eq!(request.request_id, "request-1");
                        assert_eq!(request.method, "status");
                    }
                }
            });
    }

    #[test]
    fn rejects_oversized_frame_before_allocating_payload() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut client, mut server) = duplex(128);
                let writer = hbb_common::tokio::spawn(async move {
                    client.write_all(MAGIC).await.unwrap();
                    client
                        .write_all(&((MAX_FRAME_BYTES as u32) + 1).to_be_bytes())
                        .await
                        .unwrap();
                });
                let error = read_request(&mut server).await.unwrap_err();
                writer.await.unwrap();
                assert!(error.framed);
                assert!(error.detail.contains("maximum"));
            });
    }

    #[test]
    fn rejects_trailing_frame_smuggling_bytes() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut client, mut server) = duplex(4096);
                let mut bytes = frame(&request_payload());
                bytes.extend_from_slice(b"smuggled");
                client.write_all(&bytes).await.unwrap();
                let error = read_request(&mut server).await.unwrap_err();
                assert!(error.framed);
                assert!(error.detail.contains("unexpected bytes"));
            });
    }

    #[test]
    fn rejects_legacy_text_commands() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut client, mut server) = duplex(128);
                client.write_all(b"relay-servers").await.unwrap();
                let error = read_request(&mut server).await.unwrap_err();
                assert!(!error.framed);
                assert!(error.detail.contains("legacy text control is disabled"));
            });
    }

    #[test]
    fn accepts_an_exact_one_mib_json_frame() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut value = serde_json::json!({
                    "request_id": "boundary-request",
                    "method": "boundary.test",
                    "auth_token": "abcdefghijklmnopqrstuvwxyzABCDEF",
                    "params": {"padding": ""}
                });
                let empty = serde_json::to_vec(&value).unwrap();
                let padding = "x".repeat(MAX_FRAME_BYTES - empty.len());
                value["params"]["padding"] = Value::String(padding);
                let payload = serde_json::to_vec(&value).unwrap();
                assert_eq!(payload.len(), MAX_FRAME_BYTES);
                let bytes = frame(&payload);
                let (mut client, mut server) = duplex(64 * 1024);
                let writer = hbb_common::tokio::spawn(async move {
                    client.write_all(&bytes).await.unwrap();
                });
                let request = read_request(&mut server).await.unwrap();
                writer.await.unwrap();
                match request {
                    IncomingRequest::Framed(request) => {
                        assert_eq!(request.request_id, "boundary-request");
                        assert_eq!(request.method, "boundary.test");
                    }
                }
            });
    }

    #[test]
    fn rejects_truncated_and_invalid_json_frames_without_panicking() {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut client, mut server) = duplex(256);
                let writer = hbb_common::tokio::spawn(async move {
                    client.write_all(MAGIC).await.unwrap();
                    client.write_all(&128_u32.to_be_bytes()).await.unwrap();
                    client.write_all(b"{\"request_id\":").await.unwrap();
                });
                let error = read_request(&mut server).await.unwrap_err();
                writer.await.unwrap();
                assert!(error.framed);
                assert!(error.detail.contains("truncated"));

                let invalid = b"{not-json";
                let (mut client, mut server) = duplex(256);
                client.write_all(&frame(invalid)).await.unwrap();
                let error = read_request(&mut server).await.unwrap_err();
                assert!(error.framed);
                assert!(error.detail.contains("invalid local control JSON"));
            });
    }
}

use fs2::FileExt;
use once_cell::sync::OnceCell;
use rcgen::KeyPair;
use serde_derive::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
use x509_parser::{parse_x509_certificate, pem::parse_x509_pem};

static ENROLLMENT_LOCK: OnceCell<File> = OnceCell::new();
const MAX_STATE_FILE_BYTES: u64 = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentMarker {
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelayRuntimeConfig {
    version: u32,
    node_id: String,
    relay_server: String,
    public_endpoint: String,
    telemetry_secret_file: String,
    max_sessions: u32,
    capacity_bandwidth_bps: u64,
    draining: bool,
    relay_pool: String,
    profile: String,
    #[serde(default)]
    wss_endpoint: Option<String>,
    // Central activation policy is enforced by the Control Agent. HBBR must
    // still deserialize the immutable field so endpoint/config drift cannot be
    // hidden, but it never treats the bit as authority to join a Relay pool.
    #[serde(rename = "activate_after_health")]
    _activate_after_health: bool,
    #[serde(default)]
    fast_media_udp_port: Option<u16>,
}

/// Verifies an optional paired Relay identity before HBBR opens any listener.
/// Manual and official deployments remain unchanged when the enrollment env is
/// absent. Once enrollment is requested, every mismatch fails closed.
pub(crate) fn preflight(server_public_key: &str) -> Result<(), String> {
    let require_persistent = env_flag("STARRY_REQUIRE_PERSISTENT_STATE")?;
    let require_enrollment = env_flag("STARRY_REQUIRE_RELAY_ENROLLMENT")?;
    let Some(directory) = std::env::var_os("STARRY_RELAY_ENROLLMENT_DIR") else {
        if require_enrollment {
            return Err("relay_enrollment_required_but_missing".to_owned());
        }
        if require_persistent {
            let data_dir = std::env::var_os("RELAY_DATA_DIR")
                .map(PathBuf::from)
                .ok_or_else(|| "relay_data_dir_required".to_owned())?;
            if !data_dir.is_absolute() {
                return Err("relay_data_dir_not_absolute".to_owned());
            }
            validate_directory(&data_dir)?;
            validate_persistent_backing(&data_dir)?;
        }
        return Ok(());
    };
    let directory = PathBuf::from(directory);
    if !directory.is_absolute() {
        return Err("relay_enrollment_path_not_absolute".to_owned());
    }
    validate_directory(&directory)?;
    validate_persistent_backing(&directory)?;
    let data_dir = std::env::var_os("RELAY_DATA_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "relay_data_dir_required_for_enrollment".to_owned())?;
    if !data_dir.is_absolute()
        || directory != data_dir.join("starry/enrollment")
        || !directory.starts_with(&data_dir)
    {
        return Err("relay_enrollment_outside_relay_data_dir".to_owned());
    }

    let required = [
        "node-id",
        "node-key.pem",
        "node-cert.pem",
        "relay-ca.pem",
        "center-public-key",
        "telemetry.secret",
        "relay-config.json",
        "relay-compat.env",
        "host-id",
        "enrollment.json",
    ];
    for name in required {
        validate_regular_file(&directory.join(name), is_private_file(name))?;
    }

    let marker: EnrollmentMarker = read_json(&directory.join("enrollment.json"))?;
    let runtime: RelayRuntimeConfig = read_json(&directory.join("relay-config.json"))?;
    if marker.version != 1
        || marker.purpose != "relay"
        || uuid::Uuid::parse_str(&marker.enrollment_id).is_err()
        || !valid_sha256(&marker.configuration_digest)
        || runtime.version != 1
        || marker.generation == 0
        || marker.node_id.is_empty()
        || marker.node_id.len() > 128
        || marker.node_id != runtime.node_id
        || marker.relay_server != runtime.relay_server
        || runtime.public_endpoint != runtime.relay_server
        || runtime.max_sessions == 0
        || runtime.max_sessions > 1_000_000
        || runtime.capacity_bandwidth_bps == 0
        || runtime.relay_pool.is_empty()
        || runtime.relay_pool.len() > 128
        || !matches!(
            runtime.profile.as_str(),
            "native" | "native-wss" | "native-wss-fastmedia"
        )
        || (runtime.profile == "native" && runtime.wss_endpoint.is_some())
        || (runtime.profile != "native" && runtime.wss_endpoint.is_none())
        || runtime
            .wss_endpoint
            .as_deref()
            .is_some_and(|endpoint| !valid_telemetry_endpoint(endpoint))
        || (runtime.profile == "native-wss-fastmedia") != runtime.fast_media_udp_port.is_some()
        || marker.key_fingerprint.len() != 71
        || !marker.key_fingerprint.starts_with("sha256:")
        || runtime.fast_media_udp_port == Some(0)
    {
        return Err("relay_enrollment_marker_invalid".to_owned());
    }
    let node_id = read_trimmed(&directory.join("node-id"), 128)?;
    if node_id != marker.node_id {
        return Err("relay_enrollment_node_mismatch".to_owned());
    }
    validate_node_identity(
        &directory.join("node-key.pem"),
        &directory.join("node-cert.pem"),
        &marker.key_fingerprint,
    )?;
    validate_certificate(&directory.join("relay-ca.pem"))?;
    validate_certificate_chain(
        &directory.join("node-cert.pem"),
        &directory.join("relay-ca.pem"),
    )?;
    let host_id = read_trimmed(&directory.join("host-id"), 128)?;
    let expected_host = host_binding()?;
    if host_id != expected_host || marker.host_binding != expected_host {
        return Err("relay_enrollment_host_clone_detected".to_owned());
    }
    let public_key = read_trimmed(&directory.join("center-public-key"), 256)?;
    if public_key != server_public_key.trim()
        || base64::decode(&public_key)
            .map(|value| value.len() != 32)
            .unwrap_or(true)
    {
        return Err("relay_enrollment_center_key_mismatch".to_owned());
    }
    let telemetry_secret = read_trimmed(&directory.join("telemetry.secret"), 128)?;
    if base64::decode_config(&telemetry_secret, base64::URL_SAFE_NO_PAD)
        .map(|value| value.len() != 32)
        .unwrap_or(true)
    {
        return Err("relay_enrollment_telemetry_secret_invalid".to_owned());
    }
    let configured_secret = PathBuf::from(&runtime.telemetry_secret_file);
    if configured_secret != directory.join("telemetry.secret") {
        return Err("relay_enrollment_telemetry_path_mismatch".to_owned());
    }
    if std::env::var_os("STARRY_RELAY_TELEMETRY_SECRET_FILE").map(PathBuf::from)
        != Some(configured_secret.clone())
    {
        return Err("relay_enrollment_telemetry_env_mismatch".to_owned());
    }
    if std::env::var("STARRY_RELAY_PUBLIC_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_owned())
        .as_deref()
        != Some(runtime.public_endpoint.as_str())
    {
        return Err("relay_enrollment_endpoint_drift".to_owned());
    }
    if exact_env_u32("STARRY_RELAY_MAX_SESSIONS") != Some(runtime.max_sessions) {
        return Err("relay_enrollment_capacity_drift".to_owned());
    }
    if exact_env_u64("STARRY_RELAY_CAPACITY_BANDWIDTH_BPS") != Some(runtime.capacity_bandwidth_bps)
    {
        return Err("relay_enrollment_bandwidth_capacity_drift".to_owned());
    }
    let expected_draining = if runtime.draining { "1" } else { "0" };
    if std::env::var("STARRY_RELAY_DRAINING").as_deref() != Ok(expected_draining) {
        return Err("relay_enrollment_draining_drift".to_owned());
    }
    if exact_env_u16("STARRY_RELAY_FAST_MEDIA_UDP_PORT") != runtime.fast_media_udp_port {
        return Err("relay_enrollment_fast_media_port_drift".to_owned());
    }
    validate_compatibility_export(&directory, &runtime, &public_key, &telemetry_secret)?;

    let lock_path = directory.join("active.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|_| "relay_enrollment_lock_unavailable".to_owned())?;
    set_private_permissions(&lock_path)?;
    lock.try_lock_exclusive()
        .map_err(|_| "relay_enrollment_identity_already_active".to_owned())?;
    ENROLLMENT_LOCK
        .set(lock)
        .map_err(|_| "relay_enrollment_preflight_repeated".to_owned())?;
    Ok(())
}

fn is_private_file(name: &str) -> bool {
    matches!(name, "node-key.pem" | "telemetry.secret")
}

fn validate_directory(path: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "relay_enrollment_directory_missing".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("relay_enrollment_directory_unsafe".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o022 != 0 {
            return Err("relay_enrollment_directory_permissions".to_owned());
        }
    }
    Ok(())
}

fn validate_regular_file(path: &Path, private: bool) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "relay_enrollment_file_missing".to_owned())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_STATE_FILE_BYTES
    {
        return Err("relay_enrollment_file_unsafe".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1
            || metadata.mode() & 0o022 != 0
            || (private && metadata.mode() & 0o077 != 0)
        {
            return Err("relay_enrollment_file_permissions".to_owned());
        }
    }
    Ok(())
}

fn validate_node_identity(
    key_path: &Path,
    certificate_path: &Path,
    expected_fingerprint: &str,
) -> Result<(), String> {
    let private_key = fs::read_to_string(key_path)
        .map_err(|_| "relay_enrollment_node_key_unreadable".to_owned())?;
    let key = KeyPair::from_pem(&private_key)
        .map_err(|_| "relay_enrollment_node_key_invalid".to_owned())?;
    let key_fingerprint = format!("sha256:{:x}", Sha256::digest(key.public_key_der()));
    if key_fingerprint != expected_fingerprint {
        return Err("relay_enrollment_node_key_mismatch".to_owned());
    }
    let certificate_fingerprint = certificate_public_key_fingerprint(certificate_path)?;
    if certificate_fingerprint != expected_fingerprint {
        return Err("relay_enrollment_node_certificate_mismatch".to_owned());
    }
    Ok(())
}

fn validate_certificate(path: &Path) -> Result<(), String> {
    let _ = certificate_public_key_fingerprint(path)?;
    Ok(())
}

fn certificate_public_key_fingerprint(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|_| "relay_enrollment_certificate_unreadable".to_owned())?;
    let (_, pem) =
        parse_x509_pem(&bytes).map_err(|_| "relay_enrollment_certificate_invalid".to_owned())?;
    let (_, certificate) = parse_x509_certificate(&pem.contents)
        .map_err(|_| "relay_enrollment_certificate_invalid".to_owned())?;
    if certificate.validity().not_after.timestamp() <= hbb_common::get_time() / 1_000 {
        return Err("relay_enrollment_certificate_expired".to_owned());
    }
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(certificate.tbs_certificate.subject_pki.raw)
    ))
}

fn validate_certificate_chain(leaf_path: &Path, ca_path: &Path) -> Result<(), String> {
    let leaf_bytes =
        fs::read(leaf_path).map_err(|_| "relay_enrollment_certificate_unreadable".to_owned())?;
    let (_, leaf_pem) = parse_x509_pem(&leaf_bytes)
        .map_err(|_| "relay_enrollment_certificate_invalid".to_owned())?;
    let (_, leaf) = parse_x509_certificate(&leaf_pem.contents)
        .map_err(|_| "relay_enrollment_certificate_invalid".to_owned())?;
    let ca_bytes =
        fs::read(ca_path).map_err(|_| "relay_enrollment_certificate_unreadable".to_owned())?;
    let (_, ca_pem) =
        parse_x509_pem(&ca_bytes).map_err(|_| "relay_enrollment_certificate_invalid".to_owned())?;
    let (_, ca) = parse_x509_certificate(&ca_pem.contents)
        .map_err(|_| "relay_enrollment_certificate_invalid".to_owned())?;
    if leaf.issuer() != ca.subject()
        || ca.verify_signature(None).is_err()
        || leaf.verify_signature(Some(ca.public_key())).is_err()
    {
        return Err("relay_enrollment_certificate_chain_invalid".to_owned());
    }
    Ok(())
}

fn exact_env_u16(name: &str) -> Option<u16> {
    std::env::var(name).ok()?.parse().ok()
}

fn exact_env_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.parse().ok()
}

fn exact_env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}

fn env_flag(name: &str) -> Result<bool, String> {
    match std::env::var(name).as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("") | Ok("0") | Ok("false") => Ok(false),
        Ok("1") | Ok("true") => Ok(true),
        _ => Err(format!("{name}_invalid")),
    }
}

fn validate_compatibility_export(
    directory: &Path,
    runtime: &RelayRuntimeConfig,
    public_key: &str,
    telemetry_secret: &str,
) -> Result<(), String> {
    let mut expected = format!(
        "KEY={public_key}\nSTARRY_RELAY_TELEMETRY_SECRET_FILE={}\nSTARRY_RELAY_PUBLIC_ENDPOINT={}\nSTARRY_RELAY_MAX_SESSIONS={}\nSTARRY_RELAY_CAPACITY_BANDWIDTH_BPS={}\nSTARRY_RELAY_DRAINING={}\nSTARRY_RELAY_ENROLLMENT_DIR={}\n",
        directory.join("telemetry.secret").display(),
        runtime.public_endpoint,
        runtime.max_sessions,
        runtime.capacity_bandwidth_bps,
        if runtime.draining { 1 } else { 0 },
        directory.display(),
    );
    if let Some(port) = runtime.fast_media_udp_port {
        expected.push_str(&format!("STARRY_RELAY_FAST_MEDIA_UDP_PORT={port}\n"));
    }
    let actual = fs::read_to_string(directory.join("relay-compat.env"))
        .map_err(|_| "relay_enrollment_compat_unreadable".to_owned())?;
    if actual != expected || actual.contains(telemetry_secret) {
        return Err("relay_enrollment_compat_drift".to_owned());
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|_| "relay_enrollment_file_unreadable".to_owned())?;
    serde_json::from_slice(&bytes).map_err(|_| "relay_enrollment_json_invalid".to_owned())
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

fn valid_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn read_trimmed(path: &Path, maximum: usize) -> Result<String, String> {
    let value = fs::read_to_string(path)
        .map_err(|_| "relay_enrollment_file_unreadable".to_owned())?
        .trim()
        .to_owned();
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err("relay_enrollment_value_invalid".to_owned());
    }
    Ok(value)
}

fn host_binding() -> Result<String, String> {
    let raw = std::env::var("STARRY_RELAY_HOST_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| machine_uid::get().ok())
        .ok_or_else(|| "relay_enrollment_host_identity_unavailable".to_owned())?;
    if raw.len() > 512 || raw.chars().any(char::is_control) {
        return Err("relay_enrollment_host_identity_invalid".to_owned());
    }
    Ok(format!("sha256:{:x}", Sha256::digest(raw.as_bytes())))
}

#[cfg(target_os = "linux")]
fn validate_persistent_backing(path: &Path) -> Result<(), String> {
    let canonical =
        fs::canonicalize(path).map_err(|_| "relay_enrollment_directory_missing".to_owned())?;
    let mountinfo = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|_| "relay_enrollment_mountinfo_unavailable".to_owned())?;
    let mut best: Option<(usize, &str)> = None;
    for line in mountinfo.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let mut left_fields = left.split_whitespace();
        let mountpoint = left_fields.nth(4).unwrap_or_default().replace("\\040", " ");
        let fs_type = right.split_whitespace().next().unwrap_or_default();
        let mountpoint_path = Path::new(&mountpoint);
        if canonical.starts_with(mountpoint_path)
            && best.is_none_or(|(length, _)| mountpoint.len() > length)
        {
            best = Some((mountpoint.len(), fs_type));
        }
    }
    match best {
        Some((_, "overlay" | "tmpfs")) => Err("relay_enrollment_not_persistent".to_owned()),
        Some(_) => Ok(()),
        None => Err("relay_enrollment_mount_unknown".to_owned()),
    }
}

#[cfg(not(target_os = "linux"))]
fn validate_persistent_backing(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn set_private_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| "relay_enrollment_lock_permissions".to_owned())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn host_binding_never_exposes_the_source_identity() {
        std::env::set_var("STARRY_RELAY_HOST_ID", "private-host-identity");
        let binding = host_binding().unwrap();
        std::env::remove_var("STARRY_RELAY_HOST_ID");
        assert!(binding.starts_with("sha256:"));
        assert!(!binding.contains("private-host"));
    }

    #[test]
    fn paired_runtime_format_accepts_every_v1_field_and_rejects_drift() {
        let runtime = json!({
            "version": 1,
            "node_id": "relay-sg",
            "relay_server": "relay.example:21117",
            "public_endpoint": "relay.example:21117",
            "telemetry_secret_file": "/var/lib/relay/starry/enrollment/telemetry.secret",
            "max_sessions": 100,
            "capacity_bandwidth_bps": 1_000_000_000_u64,
            "draining": false,
            "relay_pool": "primary",
            "profile": "native-wss-fastmedia",
            "wss_endpoint": "wss://relay.example:21119/ws/telemetry",
            "activate_after_health": true,
            "fast_media_udp_port": 22119
        });
        let parsed: RelayRuntimeConfig = serde_json::from_value(runtime.clone()).unwrap();
        assert_eq!(parsed.fast_media_udp_port, Some(22119));
        let mut drifted = runtime;
        drifted["unapproved_field"] = json!(true);
        assert!(serde_json::from_value::<RelayRuntimeConfig>(drifted).is_err());
    }
}

use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use x509_parser::{parse_x509_certificate, pem::parse_x509_pem};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MINIMUM_ROLLBACK_CERTIFICATE_SECONDS: u64 = 90 * 24 * 60 * 60;

#[derive(Clone, Debug)]
pub struct DowngradeOptions {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub runtime_state: Option<PathBuf>,
    pub runtime_state_value: Option<Value>,
    pub certificates: Vec<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DrainState {
    version: u32,
    observed_at_unix: u64,
    fast_media_runtime_enabled: bool,
    fast_media_active_allocations: u64,
    fast_media_active_authorizations: u64,
    fast_media_active_streams: u64,
    last_fast_media_authorization_expires_at_unix: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DowngradeReport {
    pub from_schema: u64,
    pub to_schema: u64,
    pub ready: bool,
    pub blockers: Vec<String>,
    pub removed_json_pointers: Vec<String>,
    pub certificate_window_seconds: u64,
    pub output: Option<String>,
    pub document: String,
}

pub fn preview_or_export(options: DowngradeOptions) -> Result<DowngradeReport, String> {
    validate_regular_file(&options.input, MAX_CONFIG_BYTES, false)?;
    let raw = fs::read(&options.input).map_err(|_| "downgrade_input_unreadable".to_owned())?;
    let mut document: Value =
        serde_yml::from_slice(&raw).map_err(|_| "downgrade_input_yaml_invalid".to_owned())?;
    if document.get("version").and_then(Value::as_u64) != Some(5) {
        return Err("downgrade_requires_schema_v5_input".to_owned());
    }
    let mut blockers = Vec::new();
    if document
        .pointer("/fast_mode/relay/fast_media_v1_enabled")
        .and_then(Value::as_bool)
        == Some(true)
    {
        blockers.push("fast_media_must_be_disabled_before_downgrade".to_owned());
    }
    if document
        .pointer("/fast_mode/relay/fast_compat_enabled")
        .and_then(Value::as_bool)
        == Some(true)
        && document
            .pointer("/relay_quality/enabled")
            .and_then(Value::as_bool)
            != Some(true)
    {
        blockers.push("schema_v4_fast_compat_requires_relay_quality".to_owned());
    }

    let now = unix_seconds();
    let drain_state = if let Some(value) = options.runtime_state_value.as_ref() {
        parse_drain_state(value.clone())
    } else if let Some(path) = options.runtime_state.as_deref() {
        read_drain_state(path)
    } else {
        Err("fast_media_drain_state_required".to_owned())
    };
    match drain_state {
        Ok(state) => {
            if state.version != 1
                || state.observed_at_unix > now.saturating_add(30)
                || now.saturating_sub(state.observed_at_unix) > 60
            {
                blockers.push("fast_media_drain_state_is_stale".to_owned());
            }
            if state.fast_media_active_allocations != 0
                || state.fast_media_active_authorizations != 0
                || state.fast_media_active_streams != 0
            {
                blockers.push("fast_media_is_not_drained".to_owned());
            }
            if state.fast_media_runtime_enabled {
                blockers.push("fast_media_runtime_must_be_disabled".to_owned());
            }
            if state.last_fast_media_authorization_expires_at_unix > now {
                blockers.push("fast_media_authorization_ttl_not_elapsed".to_owned());
            }
        }
        Err(code) => blockers.push(code),
    }

    if options.certificates.is_empty() {
        blockers.push("rollback_certificate_set_required".to_owned());
    }
    let mut minimum_remaining = u64::MAX;
    for certificate in &options.certificates {
        match certificate_remaining_seconds(certificate, now) {
            Ok(remaining) => {
                minimum_remaining = minimum_remaining.min(remaining);
                if remaining < MINIMUM_ROLLBACK_CERTIFICATE_SECONDS {
                    blockers.push("rollback_certificate_window_below_90_days".to_owned());
                }
            }
            Err(code) => blockers.push(code),
        }
    }
    if minimum_remaining == u64::MAX {
        minimum_remaining = 0;
    }

    let removed = remove_v5_only_fields(&mut document);
    document["version"] = Value::from(4_u64);
    let yaml = serde_yml::to_string(&document)
        .map_err(|_| "downgrade_export_serialization_failed".to_owned())?;
    if let Err(diagnostics) = crate::starry_config::parse_document(yaml.as_bytes())
        .and_then(crate::starry_config::validate_config)
    {
        if diagnostics.errors.is_empty() {
            blockers.push("schema_v4_export_validation_failed".to_owned());
        } else {
            blockers.extend(
                diagnostics
                    .errors
                    .into_iter()
                    .take(16)
                    .map(|error| format!("schema_v4_validation:{}", error.code)),
            );
        }
    }
    blockers.sort();
    blockers.dedup();
    let ready = blockers.is_empty();
    if let Some(path) = options.output.as_deref() {
        if !ready {
            return Err(format!("downgrade_blocked:{}", blockers.join(",")));
        }
        atomic_create(path, yaml.as_bytes())?;
    }
    Ok(DowngradeReport {
        from_schema: 5,
        to_schema: 4,
        ready,
        blockers,
        removed_json_pointers: removed,
        certificate_window_seconds: minimum_remaining,
        output: options.output.map(|path| path.display().to_string()),
        document: yaml,
    })
}

fn read_drain_state(path: &Path) -> Result<DrainState, String> {
    validate_regular_file(path, 64 * 1024, false)?;
    let value =
        serde_json::from_slice(&fs::read(path).map_err(|_| "drain_state_unreadable".to_owned())?)
            .map_err(|_| "drain_state_invalid".to_owned())?;
    parse_drain_state(value)
}

fn parse_drain_state(value: Value) -> Result<DrainState, String> {
    serde_json::from_value(value).map_err(|_| "drain_state_invalid".to_owned())
}

fn certificate_remaining_seconds(path: &Path, now: u64) -> Result<u64, String> {
    validate_regular_file(path, 1024 * 1024, false)?;
    let bytes = fs::read(path).map_err(|_| "rollback_certificate_unreadable".to_owned())?;
    let (_, pem) = parse_x509_pem(&bytes).map_err(|_| "rollback_certificate_invalid".to_owned())?;
    let (_, certificate) = parse_x509_certificate(&pem.contents)
        .map_err(|_| "rollback_certificate_invalid".to_owned())?;
    let not_before = certificate.validity().not_before.timestamp();
    let expiry = certificate.validity().not_after.timestamp();
    if not_before < 0 || expiry <= 0 || not_before as u64 > now {
        return Err("rollback_certificate_invalid".to_owned());
    }
    Ok((expiry as u64).saturating_sub(now))
}

fn remove_object_key(root: &mut Value, pointer: &str, key: &str, removed: &mut Vec<String>) {
    if root
        .pointer_mut(pointer)
        .and_then(Value::as_object_mut)
        .and_then(|object| object.remove(key))
        .is_some()
    {
        removed.push(format!("{pointer}/{key}"));
    }
}

fn remove_v5_only_fields(document: &mut Value) -> Vec<String> {
    let mut removed = Vec::new();
    remove_object_key(
        document,
        "/fast_mode/relay",
        "fast_media_v1_enabled",
        &mut removed,
    );
    remove_object_key(
        document,
        "/fast_mode/relay",
        "relay_max_datagram",
        &mut removed,
    );
    if let Some(endpoints) = document
        .pointer_mut("/websocket_signal/relay_health/endpoints")
        .and_then(Value::as_array_mut)
    {
        for (index, endpoint) in endpoints.iter_mut().enumerate() {
            if endpoint
                .as_object_mut()
                .and_then(|object| object.remove("fast_media_udp_port"))
                .is_some()
            {
                removed.push(format!(
                    "/websocket_signal/relay_health/endpoints/{index}/fast_media_udp_port"
                ));
            }
        }
    }
    removed
}

fn validate_regular_file(path: &Path, maximum: u64, private: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "state_file_missing".to_owned())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > maximum {
        return Err("state_file_unsafe".to_owned());
    }
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
            return Err("state_file_permissions".to_owned());
        }
    }
    Ok(())
}

fn atomic_create(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() {
        return Err("downgrade_output_exists".to_owned());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "downgrade_output_path_invalid".to_owned())?;
    fs::create_dir_all(parent).map_err(|_| "downgrade_output_directory_failed".to_owned())?;
    let parent_metadata =
        fs::symlink_metadata(parent).map_err(|_| "downgrade_output_directory_failed".to_owned())?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err("downgrade_output_directory_unsafe".to_owned());
    }
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        uuid::Uuid::now_v7()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o640);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| "downgrade_output_create_failed".to_owned())?;
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        let _ = fs::remove_file(&temporary);
        return Err("downgrade_output_write_failed".to_owned());
    }
    drop(file);
    if fs::hard_link(&temporary, path).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(if path.exists() {
            "downgrade_output_exists".to_owned()
        } else {
            "downgrade_output_commit_failed".to_owned()
        });
    }
    fs::remove_file(&temporary).map_err(|_| "downgrade_output_cleanup_failed".to_owned())?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "downgrade_output_sync_failed".to_owned())
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair};

    #[test]
    fn v5_only_fields_are_removed_without_mutating_source_value() {
        let mut value: Value = serde_json::json!({
            "version": 5,
            "fast_mode": {"relay": {
                "fast_compat_enabled": false,
                "fast_media_v1_enabled": false,
                "relay_max_datagram": 1200
            }},
            "websocket_signal": {"relay_health": {"endpoints": [{
                "relay": "relay.example:21117",
                "fast_media_udp_port": 22119
            }]}}
        });
        let original = value.clone();
        let removed = remove_v5_only_fields(&mut value);
        assert!(original
            .pointer("/fast_mode/relay/relay_max_datagram")
            .is_some());
        assert!(value
            .pointer("/fast_mode/relay/relay_max_datagram")
            .is_none());
        assert!(value
            .pointer("/websocket_signal/relay_health/endpoints/0/fast_media_udp_port")
            .is_none());
        assert_eq!(removed.len(), 3);
    }

    #[test]
    fn schema_v5_preview_and_export_are_side_effect_free_and_v4_readable() {
        let temporary = tempfile::tempdir().unwrap();
        let input = temporary.path().join("config-v5.yaml");
        let output = temporary.path().join("exports/config-v4.yaml");
        let certificate_path = temporary.path().join("agent-cert.pem");
        let enrollment_state = temporary.path().join("enrollment.json");
        let source = br#"version: 5
relay_servers:
  - relay.example:21117
websocket_signal:
  relay_health:
    endpoints:
      - relay: relay.example:21117
        url: wss://relay.example/ws/telemetry
        telemetry_secret_file: /var/lib/rustdesk-server-starry/relay-secrets/relay.example/telemetry.secret
        fast_media_udp_port: 22119
fast_mode:
  relay:
    fast_compat_enabled: false
    fast_media_v1_enabled: false
    relay_max_datagram: 1200
"#;
        fs::write(&input, source).unwrap();
        fs::write(&enrollment_state, b"preserve-newer-enrollment-state\n").unwrap();
        let key = KeyPair::generate().unwrap();
        let certificate = CertificateParams::new(vec!["agent.example".to_owned()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        fs::write(&certificate_path, certificate.pem()).unwrap();
        let runtime_state = serde_json::json!({
            "version": 1,
            "observed_at_unix": unix_seconds(),
            "fast_media_runtime_enabled": false,
            "fast_media_active_allocations": 0,
            "fast_media_active_authorizations": 0,
            "fast_media_active_streams": 0,
            "last_fast_media_authorization_expires_at_unix": 0
        });
        let options = || DowngradeOptions {
            input: input.clone(),
            output: None,
            runtime_state: None,
            runtime_state_value: Some(runtime_state.clone()),
            certificates: vec![certificate_path.clone()],
        };

        let preview = preview_or_export(options()).unwrap();
        assert!(preview.ready, "unexpected blockers: {:?}", preview.blockers);
        assert_eq!(preview.from_schema, 5);
        assert_eq!(preview.to_schema, 4);
        assert!(preview.certificate_window_seconds >= MINIMUM_ROLLBACK_CERTIFICATE_SECONDS);
        assert!(preview
            .removed_json_pointers
            .contains(&"/fast_mode/relay/fast_media_v1_enabled".to_owned()));
        assert!(preview
            .removed_json_pointers
            .contains(&"/fast_mode/relay/relay_max_datagram".to_owned()));
        assert_eq!(fs::read(&input).unwrap(), source);
        assert!(!output.exists());
        assert_eq!(
            fs::read(&enrollment_state).unwrap(),
            b"preserve-newer-enrollment-state\n"
        );

        let mut export_options = options();
        export_options.output = Some(output.clone());
        let exported = preview_or_export(export_options).unwrap();
        assert!(exported.ready);
        assert_eq!(exported.output.as_deref(), Some(output.to_str().unwrap()));
        let output_bytes = fs::read(&output).unwrap();
        let validated = crate::starry_config::parse_document(&output_bytes)
            .and_then(crate::starry_config::validate_config)
            .unwrap();
        assert_eq!(validated.config.version, 4);
        assert_eq!(
            validated.config.relay_servers,
            vec!["relay.example:21117".to_owned()]
        );
        assert_eq!(
            validated.config.websocket_signal.relay_health.endpoints[0]
                .telemetry_secret_file
                .as_deref(),
            Some("/var/lib/rustdesk-server-starry/relay-secrets/relay.example/telemetry.secret")
        );
        assert!(validated.config.websocket_signal.relay_health.endpoints[0]
            .fast_media_udp_port
            .is_none());
        assert_eq!(fs::read(&input).unwrap(), source);
        assert_eq!(
            fs::read(&enrollment_state).unwrap(),
            b"preserve-newer-enrollment-state\n"
        );
    }
}

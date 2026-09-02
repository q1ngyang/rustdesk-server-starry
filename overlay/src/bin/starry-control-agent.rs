use hbb_common::ResultType;
use hbbs::config_downgrade::{preview_or_export, DowngradeOptions};
use hbbs::control_agent::collect_fast_media_drain_state;
use hbbs::pairing::{control_pair, read_pairing_code, ControlPairMode, ControlPairOptions};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
};

const HELP: &str = "\
Starry Control Agent\n\n\
Usage:\n  starry-control-agent [serve] [CONFIG]\n  starry-control-agent pair [OPTIONS]\n  starry-control-agent adopt [OPTIONS]\n  starry-control-agent rotate [OPTIONS]\n  starry-control-agent config downgrade --to-schema 4 [--preview | --output PATH] [OPTIONS]\n\n\
Pairing options (the SP1 code is accepted only from stdin or a mode-0600 file):\n  --code-file PATH\n  --broker-ca-file PATH\n  --state-dir PATH\n  --identity-dir PATH\n  --output PATH\n  --shared-dir PATH\n  --managed-config PATH\n  --backup-dir PATH\n  --listen ADDRESS\n  --local-control-address ADDRESS\n\n\
Downgrade options:\n  --agent-config PATH\n  --runtime-state PATH (offline audited override)\n  --certificate PATH (repeatable)\n";

fn main() -> ResultType<()> {
    let mut arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|value| value == "--help" || value == "-h")
    {
        print!("{HELP}");
        return Ok(());
    }
    let command = arguments.first().map(String::as_str);
    match command {
        Some("pair") | Some("adopt") | Some("rotate") => {
            let command = arguments.remove(0);
            let mode = match command.as_str() {
                "pair" => ControlPairMode::Pair,
                "adopt" => ControlPairMode::Adopt,
                "rotate" => ControlPairMode::Rotate,
                _ => unreachable!(),
            };
            run_pairing(mode, parse_options(arguments)?)
        }
        Some("config") => run_config_command(arguments),
        Some("serve") => {
            arguments.remove(0);
            if arguments.len() > 1 {
                return Err(hbb_common::anyhow::anyhow!(
                    "serve accepts at most one CONFIG"
                ));
            }
            run_server(arguments.pop())
        }
        Some(value) if value.starts_with('-') => {
            Err(hbb_common::anyhow::anyhow!("unknown option: {value}"))
        }
        _ => {
            if arguments.len() > 1 {
                return Err(hbb_common::anyhow::anyhow!("expected one CONFIG path"));
            }
            run_server(arguments.pop())
        }
    }
}

fn run_config_command(arguments: Vec<String>) -> ResultType<()> {
    if arguments.get(1).map(String::as_str) != Some("downgrade") {
        return Err(hbb_common::anyhow::anyhow!(
            "expected `config downgrade --to-schema 4`"
        ));
    }
    let mut input = None;
    let mut output = None;
    let mut runtime_state = None;
    let mut agent_config = None;
    let mut certificates = Vec::new();
    let mut preview = false;
    let mut to_schema = None;
    let mut values = arguments.into_iter().skip(2);
    while let Some(argument) = values.next() {
        if argument == "--preview" {
            if preview {
                return Err(hbb_common::anyhow::anyhow!("duplicate --preview"));
            }
            preview = true;
            continue;
        }
        let (name, value) = if let Some((name, value)) = argument.split_once('=') {
            (name.to_owned(), value.to_owned())
        } else {
            let value = values
                .next()
                .ok_or_else(|| hbb_common::anyhow::anyhow!("option {argument} requires a value"))?;
            (argument, value)
        };
        if value.is_empty() {
            return Err(hbb_common::anyhow::anyhow!("empty option: {name}"));
        }
        match name.as_str() {
            "--to-schema" if to_schema.is_none() => to_schema = Some(value),
            "--input" if input.is_none() => input = Some(PathBuf::from(value)),
            "--output" if output.is_none() => output = Some(PathBuf::from(value)),
            "--runtime-state" if runtime_state.is_none() => {
                runtime_state = Some(PathBuf::from(value))
            }
            "--agent-config" if agent_config.is_none() => agent_config = Some(PathBuf::from(value)),
            "--certificate" => certificates.push(PathBuf::from(value)),
            _ => {
                return Err(hbb_common::anyhow::anyhow!(
                    "unknown or duplicate option: {name}"
                ))
            }
        }
    }
    if to_schema.as_deref() != Some("4") {
        return Err(hbb_common::anyhow::anyhow!("--to-schema 4 is required"));
    }
    if preview && output.is_some() {
        return Err(hbb_common::anyhow::anyhow!(
            "--preview and --output are mutually exclusive"
        ));
    }
    let defaults = DowngradePaths::discover()?;
    let runtime_state = runtime_state
        .or_else(|| std::env::var_os("STARRY_FAST_MEDIA_DRAIN_STATE_FILE").map(PathBuf::from));
    let runtime_state_value = if runtime_state.is_none() {
        Some(
            runtime()?
                .block_on(collect_fast_media_drain_state(
                    agent_config.unwrap_or_else(|| defaults.agent_config.clone()),
                ))
                .map_err(hbb_common::anyhow::Error::msg)?,
        )
    } else {
        None
    };
    let report = preview_or_export(DowngradeOptions {
        input: input.unwrap_or(defaults.input),
        output,
        runtime_state,
        runtime_state_value,
        certificates: if certificates.is_empty() {
            defaults.certificates
        } else {
            certificates
        },
    })
    .map_err(hbb_common::anyhow::Error::msg)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

struct DowngradePaths {
    input: PathBuf,
    agent_config: PathBuf,
    certificates: Vec<PathBuf>,
}

impl DowngradePaths {
    fn discover() -> ResultType<Self> {
        if let Some(root) = std::env::var_os("STARRY_PERSIST_ROOT").map(PathBuf::from) {
            require_absolute(&root, "STARRY_PERSIST_ROOT")?;
            let mut certificates = Vec::new();
            append_if_file(
                &mut certificates,
                root.join("control/identity/server-cert.pem"),
            );
            append_relay_certificates(&mut certificates, &root.join("relay-secrets"))?;
            Ok(Self {
                input: root.join("config/config.yaml"),
                agent_config: root.join("control/generated/control-agent.yaml"),
                certificates,
            })
        } else {
            let mut certificates = Vec::new();
            append_if_file(
                &mut certificates,
                PathBuf::from("/etc/rustdesk-server-starry/control-identity/server-cert.pem"),
            );
            append_relay_certificates(
                &mut certificates,
                Path::new("/var/lib/rustdesk-server-starry/relay-secrets"),
            )?;
            Ok(Self {
                input: PathBuf::from("/etc/rustdesk-server-starry/managed/config.yaml"),
                agent_config: PathBuf::from("/etc/rustdesk-server-starry/control-agent.yaml"),
                certificates,
            })
        }
    }
}

fn append_if_file(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if path.is_file() {
        paths.push(path);
    }
}

fn append_relay_certificates(paths: &mut Vec<PathBuf>, root: &Path) -> ResultType<()> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path().join("node-cert.pem");
        append_if_file(paths, path);
    }
    paths.sort();
    paths.dedup();
    Ok(())
}

fn run_server(path: Option<String>) -> ResultType<()> {
    let path = path
        .or_else(|| std::env::var("STARRY_CONTROL_AGENT_CONFIG").ok())
        .unwrap_or_else(|| "starry-control-agent.yaml".to_owned());
    runtime()?
        .block_on(hbbs::control_agent::run(path))
        .map_err(hbb_common::anyhow::Error::msg)
}

fn run_pairing(mode: ControlPairMode, options: BTreeMap<String, String>) -> ResultType<()> {
    let defaults = ControlPaths::discover()?;
    let code_file = optional_path(&options, "code-file");
    let code = read_pairing_code(code_file.as_deref()).map_err(hbb_common::anyhow::Error::msg)?;
    let pairing = ControlPairOptions {
        mode,
        state_dir: path_option(&options, "state-dir", defaults.state_dir),
        identity_dir: path_option(&options, "identity-dir", defaults.identity_dir),
        output: path_option(&options, "output", defaults.output),
        shared_dir: path_option(&options, "shared-dir", defaults.shared_dir),
        managed_config_path: path_option(&options, "managed-config", defaults.managed_config),
        backup_dir: path_option(&options, "backup-dir", defaults.backup_dir),
        listen: address_option(&options, "listen", defaults.listen)?,
        local_control_address: address_option(
            &options,
            "local-control-address",
            defaults.local_control_address,
        )?,
        broker_ca_file: optional_path(&options, "broker-ca-file"),
    };
    let result = runtime()?
        .block_on(control_pair(&code, pairing))
        .map_err(hbb_common::anyhow::Error::msg)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

struct ControlPaths {
    state_dir: PathBuf,
    identity_dir: PathBuf,
    output: PathBuf,
    shared_dir: PathBuf,
    managed_config: PathBuf,
    backup_dir: PathBuf,
    listen: SocketAddr,
    local_control_address: SocketAddr,
}

impl ControlPaths {
    fn discover() -> ResultType<Self> {
        let persist_root = std::env::var_os("STARRY_PERSIST_ROOT").map(PathBuf::from);
        if let Some(root) = persist_root {
            require_absolute(&root, "STARRY_PERSIST_ROOT")?;
            Ok(Self {
                state_dir: root.join("control/state"),
                identity_dir: root.join("control/identity"),
                output: root.join("control/generated/control-agent.yaml"),
                shared_dir: root.join("control/shared"),
                managed_config: root.join("config/config.yaml"),
                backup_dir: root.join("config/history"),
                listen: env_address("STARRY_CONTROL_AGENT_LISTEN", "0.0.0.0:21120")?,
                local_control_address: env_address(
                    "STARRY_LOCAL_CONTROL_ADDRESS",
                    "127.0.0.1:21119",
                )?,
            })
        } else {
            Ok(Self {
                state_dir: PathBuf::from("/var/lib/rustdesk-server-starry/control/state"),
                identity_dir: PathBuf::from("/etc/rustdesk-server-starry/control-identity"),
                output: PathBuf::from("/etc/rustdesk-server-starry/control-agent.yaml"),
                shared_dir: PathBuf::from("/var/lib/rustdesk-server-starry/control/shared"),
                managed_config: PathBuf::from("/etc/rustdesk-server-starry/managed/config.yaml"),
                backup_dir: PathBuf::from("/var/lib/rustdesk-server-starry/config-history"),
                listen: env_address("STARRY_CONTROL_AGENT_LISTEN", "127.0.0.1:21120")?,
                local_control_address: env_address(
                    "STARRY_LOCAL_CONTROL_ADDRESS",
                    "127.0.0.1:21119",
                )?,
            })
        }
    }
}

fn parse_options(arguments: Vec<String>) -> ResultType<BTreeMap<String, String>> {
    let allowed = [
        "code-file",
        "broker-ca-file",
        "state-dir",
        "identity-dir",
        "output",
        "shared-dir",
        "managed-config",
        "backup-dir",
        "listen",
        "local-control-address",
    ];
    let mut options = BTreeMap::new();
    let mut input = arguments.into_iter();
    while let Some(argument) = input.next() {
        let (name, value) = if let Some((name, value)) = argument.split_once('=') {
            (name.to_owned(), value.to_owned())
        } else {
            let value = input
                .next()
                .ok_or_else(|| hbb_common::anyhow::anyhow!("option {argument} requires a value"))?;
            (argument, value)
        };
        let name = name
            .strip_prefix("--")
            .ok_or_else(|| hbb_common::anyhow::anyhow!("unexpected positional argument: {name}"))?;
        if !allowed.contains(&name) || value.is_empty() || options.contains_key(name) {
            return Err(hbb_common::anyhow::anyhow!(
                "unknown, empty, or duplicate option: --{name}"
            ));
        }
        options.insert(name.to_owned(), value);
    }
    Ok(options)
}

fn path_option(options: &BTreeMap<String, String>, name: &str, default: PathBuf) -> PathBuf {
    options.get(name).map(PathBuf::from).unwrap_or(default)
}

fn optional_path(options: &BTreeMap<String, String>, name: &str) -> Option<PathBuf> {
    options.get(name).map(PathBuf::from)
}

fn address_option(
    options: &BTreeMap<String, String>,
    name: &str,
    default: SocketAddr,
) -> ResultType<SocketAddr> {
    match options.get(name) {
        Some(value) => Ok(value.parse()?),
        None => Ok(default),
    }
}

fn env_address(name: &str, default: &str) -> ResultType<SocketAddr> {
    Ok(std::env::var(name)
        .unwrap_or_else(|_| default.to_owned())
        .parse()?)
}

fn require_absolute(path: &Path, name: &str) -> ResultType<()> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(hbb_common::anyhow::anyhow!("{name} must be absolute"))
    }
}

fn runtime() -> ResultType<hbb_common::tokio::runtime::Runtime> {
    Ok(hbb_common::tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?)
}

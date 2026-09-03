use hbb_common::ResultType;
use hbbs::pairing::{read_pairing_code, relay_enroll, RelayEnrollOptions};
use std::{collections::BTreeMap, path::PathBuf};

const HELP: &str = "\
Starry Relay enrollment utility\n\n\
Usage:\n  starry-relayctl enroll [--code-file PATH] [--broker-ca-file PATH] [--data-dir PATH]\n\n\
The SP1 code is accepted only from stdin or a mode-0600 file. This utility never\n\
changes the upstream hbbr command-line interface.\n";

fn main() -> ResultType<()> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|value| value == "--help" || value == "-h")
    {
        print!("{HELP}");
        return Ok(());
    }
    if arguments.first().map(String::as_str) != Some("enroll") {
        return Err(hbb_common::anyhow::anyhow!("expected `enroll`; use --help"));
    }
    let options = parse_options(arguments.into_iter().skip(1).collect())?;
    let data_dir = options
        .get("data-dir")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("RELAY_DATA_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/var/lib/rustdesk-server-starry/relay"));
    if !data_dir.is_absolute() {
        return Err(hbb_common::anyhow::anyhow!(
            "RELAY_DATA_DIR/--data-dir must be absolute"
        ));
    }
    let code_file = options.get("code-file").map(PathBuf::from);
    let code = read_pairing_code(code_file.as_deref()).map_err(hbb_common::anyhow::Error::msg)?;
    let runtime = hbb_common::tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime
        .block_on(relay_enroll(
            &code,
            RelayEnrollOptions {
                data_dir,
                broker_ca_file: options.get("broker-ca-file").map(PathBuf::from),
            },
        ))
        .map_err(hbb_common::anyhow::Error::msg)?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

fn parse_options(arguments: Vec<String>) -> ResultType<BTreeMap<String, String>> {
    let allowed = ["code-file", "broker-ca-file", "data-dir"];
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

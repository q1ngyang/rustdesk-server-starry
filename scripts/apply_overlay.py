#!/usr/bin/env python3
"""Apply the rustdesk-server-starry overlay to a clean upstream checkout."""

from __future__ import annotations

import argparse
import re
import shutil
from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    content = path.read_text(encoding="utf-8")
    count = content.count(old)
    if count != 1:
        raise RuntimeError(
            f"expected exactly one overlay anchor in {path}, found {count}: {old!r}"
        )
    path.write_text(content.replace(old, new, 1), encoding="utf-8")


def replace_between_once(path: Path, start: str, end: str, replacement: str) -> None:
    """Replace one block beginning at start and ending immediately before end."""
    content = path.read_text(encoding="utf-8")
    start_count = content.count(start)
    if start_count != 1:
        raise RuntimeError(
            f"expected one overlay start anchor in {path}, found {start_count}: "
            f"{start!r}"
        )
    start_at = content.index(start)
    try:
        end_at = content.index(end, start_at + len(start))
    except ValueError as err:
        raise RuntimeError(
            f"overlay end anchor not found after unique start in {path}: {end!r}"
        ) from err
    path.write_text(
        content[:start_at] + replacement + content[end_at:], encoding="utf-8"
    )


def proto_block_bounds(content: str, declaration: str) -> tuple[int, int]:
    """Return the byte range for one protobuf block, including its braces."""
    matches = list(
        re.finditer(
            rf"(?m)^[ \t]*{re.escape(declaration)}[ \t]*\{{",
            content,
        )
    )
    if len(matches) != 1:
        raise RuntimeError(f"expected one protobuf declaration: {declaration!r}")
    start = matches[0].start()
    open_at = matches[0].end() - 1
    depth = 0
    for index in range(open_at, len(content)):
        if content[index] == "{":
            depth += 1
        elif content[index] == "}":
            depth -= 1
            if depth == 0:
                return start, index + 1
    raise RuntimeError(f"protobuf declaration has no closing brace: {declaration!r}")


def add_proto_field(path: Path, message: str, field: str, tag: int) -> None:
    content = path.read_text(encoding="utf-8")
    start, end = proto_block_bounds(content, f"message {message}")
    block = content[start:end]
    if field.strip() in block:
        return
    if re.search(rf"=\s*{tag}\s*;", block):
        raise RuntimeError(
            f"protobuf tag {tag} is already occupied in message {message}"
        )
    close_at = end - 1
    insertion = f"  {field.strip()}\n"
    path.write_text(content[:close_at] + insertion + content[close_at:], encoding="utf-8")


def add_proto_oneof_arm(
    path: Path, message: str, oneof: str, arm: str, tag: int
) -> None:
    content = path.read_text(encoding="utf-8")
    message_start, message_end = proto_block_bounds(content, f"message {message}")
    message_block = content[message_start:message_end]
    relative_start, relative_end = proto_block_bounds(message_block, f"oneof {oneof}")
    oneof_block = message_block[relative_start:relative_end]
    if arm.strip() in oneof_block:
        return
    if re.search(rf"=\s*{tag}\s*;", oneof_block):
        raise RuntimeError(
            f"protobuf tag {tag} is already occupied in {message}.{oneof}"
        )
    close_brace_at = message_start + relative_end - 1
    close_at = content.rfind("\n", message_start, close_brace_at) + 1
    insertion = f"    {arm.strip()}\n"
    path.write_text(content[:close_at] + insertion + content[close_at:], encoding="utf-8")


def copy_overlay(repo_root: Path, upstream: Path) -> None:
    shutil.copyfile(repo_root / "Cross.toml", upstream / "Cross.toml")
    shutil.copyfile(repo_root / "PATCH_VERSION", upstream / "PATCH_VERSION")
    shutil.copyfile(repo_root / "overlay/Cargo.lock", upstream / "Cargo.lock")
    for name in (
        "allocation_explain.rs",
        "connection_auth.rs",
        "config_downgrade.rs",
        "control_agent.rs",
        "database.rs",
        "fast_media_relay.rs",
        "fast_relay.rs",
        "geo_relay.rs",
        "local_control.rs",
        "profile_activation.rs",
        "pairing.rs",
        "relay_enrollment.rs",
        "relay_observer.rs",
        "relay_quality.rs",
        "secure_tcp.rs",
        "starry_config.rs",
        "websocket_signal.rs",
    ):
        shutil.copyfile(repo_root / "overlay/src" / name, upstream / "src" / name)
    shutil.copyfile(
        repo_root / "config/config.example.yaml",
        upstream / "src/starry_config.example.yaml",
    )
    shutil.copytree(
        repo_root / "overlay/src/bin",
        upstream / "src/bin",
        dirs_exist_ok=True,
    )
    shutil.copytree(
        repo_root / "overlay/src/control_agent",
        upstream / "src/control_agent",
        dirs_exist_ok=True,
    )
    shutil.copytree(
        repo_root / "overlay/src/geo_relay",
        upstream / "src/geo_relay",
        dirs_exist_ok=True,
    )
    shutil.copytree(
        repo_root / "overlay/src/websocket_signal",
        upstream / "src/websocket_signal",
        dirs_exist_ok=True,
    )
    shutil.copytree(
        repo_root / "overlay/tests",
        upstream / "tests",
        dirs_exist_ok=True,
    )
    shutil.copytree(
        repo_root / "contracts",
        upstream / "contracts",
        dirs_exist_ok=True,
    )


def patch_dependencies(upstream: Path) -> None:
    cargo = upstream / "Cargo.toml"

    def set_dependency(
        name: str,
        desired: str,
        *,
        accepted: tuple[str, ...],
        insert_after: str | None = None,
    ) -> None:
        content = cargo.read_text(encoding="utf-8")
        matches = re.findall(
            rf"(?m)^{re.escape(name)}\s*=.*$",
            content,
        )
        if matches == [desired]:
            return
        if len(matches) == 1 and matches[0] in accepted:
            replace_once(cargo, matches[0] + "\n", desired + "\n")
            return
        if not matches and insert_after is not None:
            replace_once(
                cargo,
                insert_after + "\n",
                insert_after + "\n" + desired + "\n",
            )
            return
        raise RuntimeError(
            f"unexpected {name} dependency declaration(s) in {cargo}: {matches}"
        )

    set_dependency(
        "clap",
        'clap = { version = "3.2.25", default-features = false, features = ["std"] }',
        accepted=('clap = "2"',),
    )
    set_dependency(
        "machine-uid",
        'machine-uid = "0.6"',
        accepted=('machine-uid = "0.2"',),
    )

    content = cargo.read_text(encoding="utf-8")
    database_dependency = (
        'tokio-rusqlite = { version = "0.7", features = ["bundled"] }\n'
    )
    if database_dependency not in content:
        legacy_database_dependencies = (
            'sqlx = { version = "0.6", features = [ "runtime-tokio-rustls", '
            '"sqlite", "macros", "chrono", "json" ] }\n',
            'sqlx = { version = "0.8.6", default-features = false, features = '
            '[ "runtime-tokio-rustls", "sqlite", "macros", "chrono", "json" ] }\n',
            'sqlx = { version = "0.8.6", default-features = false, features = '
            '[ "runtime-tokio-rustls", "sqlite", "chrono", "json" ] }\n',
            'sqlx-core = { version = "=0.8.6", default-features = false, '
            'features = ["_rt-tokio", "_tls-none"] }\n'
            'sqlx-sqlite = { version = "=0.8.6", default-features = false, '
            'features = ["bundled"] }\n',
        )
        source = next(
            (block for block in legacy_database_dependencies if block in content),
            None,
        )
        if source is None:
            raise RuntimeError(
                "upstream database dependency changed; review async SQLite pin"
            )
        replace_once(cargo, source, database_dependency)
    content = cargo.read_text(encoding="utf-8")
    for obsolete_pool in (
        'deadpool = "0.8"\n',
        'deadpool = { version = "0.13", default-features = false, '
        'features = ["managed"] }\n',
    ):
        if obsolete_pool in content:
            replace_once(cargo, obsolete_pool, "")
            content = cargo.read_text(encoding="utf-8")
    set_dependency(
        "uuid",
        'uuid = { version = "1.0", features = ["v4", "v7"] }',
        accepted=('uuid = { version = "1.0", features = ["v4"] }',),
    )
    set_dependency(
        "jsonwebtoken",
        'jsonwebtoken = { version = "9.3.1", default-features = false }',
        accepted=('jsonwebtoken = "8"', 'jsonwebtoken = "9.3.1"'),
    )
    set_dependency(
        "tokio-tungstenite",
        'tokio-tungstenite = { version = "0.28", features = ["rustls-tls-native-roots"] }',
        accepted=(
            'tokio-tungstenite = "0.17"',
            'tokio-tungstenite = { version = "0.17", features = ["rustls-tls-native-roots"] }',
        ),
    )
    set_dependency(
        "tungstenite",
        'tungstenite = "0.28"',
        accepted=('tungstenite = "0.17"',),
    )
    set_dependency(
        "http",
        'http = "1.5"',
        accepted=('http = "0.2"',),
    )
    set_dependency(
        "flexi_logger",
        'flexi_logger = { version = "0.27", features = ["async", "dont_minimize_extra_stacks"] }',
        accepted=(
            'flexi_logger = { version = "0.22", features = ["async", '
            '"use_chrono_for_offset", "dont_minimize_extra_stacks"] }',
        ),
    )

    set_dependency(
        "maxminddb",
        'maxminddb = "0.30"',
        accepted=(),
        insert_after='flate2 = "1.0"',
    )
    set_dependency(
        "serde_yml",
        'serde_yml = { package = "serde_norway", version = "0.9.42" }',
        accepted=('serde_yml = "0.0.13"',),
        insert_after='maxminddb = "0.30"',
    )

    content = cargo.read_text(encoding="utf-8")
    legacy_signature = (
        '# sodiumoxide 0.2.7 requires the pre-2.0 signature trait.\n'
        'signature = "=1.5.0"\n'
    )
    if legacy_signature in content:
        replace_once(
            cargo,
            legacy_signature,
            '# ed25519 1.5.3 fixes its signature dependency to exclude 2.x.\n'
            'ed25519 = "=1.5.3"\n',
        )
    elif not re.search(r"(?m)^ed25519\s*=", content):
        replace_once(
            cargo,
            'serde_yml = { package = "serde_norway", version = "0.9.42" }\n',
            'serde_yml = { package = "serde_norway", version = "0.9.42" }\n'
            '# ed25519 1.5.3 fixes its signature dependency to exclude 2.x.\n'
            'ed25519 = "=1.5.3"\n',
        )

    production_only_dependencies = (
        'tokio-rustls = { version = "0.26", features = ["ring", "tls12"], default-features = false }',
        'tower = { version = "0.4", features = ["util"] }',
        'x509-parser = { version = "0.16.0", features = ["verify"] }',
    )
    content = cargo.read_text(encoding="utf-8")
    dependencies_end = content.find("\n[dev-dependencies]")
    if dependencies_end >= 0:
        for dependency in production_only_dependencies:
            line = dependency + "\n"
            if dependency not in content[:dependencies_end] and line in content[dependencies_end:]:
                replace_once(cargo, line, "")
                content = cargo.read_text(encoding="utf-8")
                dependencies_end = content.find("\n[dev-dependencies]")

    content = cargo.read_text(encoding="utf-8")
    dependencies_end = content.find("\n[dev-dependencies]")
    if dependencies_end >= 0:
        for rcgen_dev in (
            'rcgen = "0.9"\n',
            'rcgen = "0.12.1"\n',
            'rcgen = { version = "0.12.1", features = ["x509-parser"] }\n',
            'rcgen = { version = "0.13.2", features = ["x509-parser"] }\n',
        ):
            position = content.find(rcgen_dev, dependencies_end)
            if position >= 0:
                replace_once(cargo, rcgen_dev, "")
                content = cargo.read_text(encoding="utf-8")
                dependencies_end = content.find("\n[dev-dependencies]")

    dependency_chain = (
        ("sha2", 'sha2 = "0.10"', 'ed25519 = "=1.5.3"', ()),
        ("fs2", 'fs2 = "0.4"', 'sha2 = "0.10"', ()),
        (
            "hyper",
            'hyper = { version = "0.14", features = ["server", "http1", "tcp"] }',
            'fs2 = "0.4"',
            (),
        ),
        ("libc", 'libc = "0.2"', 'hyper = { version = "0.14", features = ["server", "http1", "tcp"] }', ()),
        (
            "rustls",
            'rustls = { version = "0.23.40", default-features = false, features = ["ring", "std", "tls12"] }',
            'libc = "0.2"',
            ('rustls = "0.20"',),
        ),
        (
            "tokio-rustls",
            'tokio-rustls = { version = "0.26", features = ["ring", "tls12"], default-features = false }',
            'rustls = { version = "0.23.40", default-features = false, features = ["ring", "std", "tls12"] }',
            ('tokio-rustls = "0.23"',),
        ),
        (
            "tower",
            'tower = { version = "0.4", features = ["util"] }',
            'tokio-rustls = { version = "0.26", features = ["ring", "tls12"], default-features = false }',
            (),
        ),
        (
            "x509-parser",
            'x509-parser = { version = "0.16.0", features = ["verify"] }',
            'tower = { version = "0.4", features = ["util"] }',
            (
                'x509-parser = "0.14"',
                'x509-parser = "0.15.1"',
                'x509-parser = { version = "0.15.1", features = ["verify"] }',
            ),
        ),
        (
            "url",
            'url = "2.2"',
            'x509-parser = { version = "0.16.0", features = ["verify"] }',
            (),
        ),
        (
            "rcgen",
            'rcgen = { version = "0.13.2", features = ["x509-parser"] }',
            'url = "2.2"',
            ('rcgen = { version = "0.12.1", features = ["x509-parser"] }',),
        ),
    )
    for name, desired, after, accepted in dependency_chain:
        set_dependency(
            name,
            desired,
            accepted=accepted,
            insert_after=after,
        )

    content = cargo.read_text(encoding="utf-8")
    for obsolete in ('rustls-pemfile = "1"\n', 'rustls-pemfile = "2.2"\n'):
        if obsolete in content:
            replace_once(cargo, obsolete, "")
            content = cargo.read_text(encoding="utf-8")

    content = cargo.read_text(encoding="utf-8")
    reqwest_replacements = (
        (
            'reqwest = { git = "https://github.com/rustdesk-org/reqwest", '
            'features = ["blocking", "socks", "json", "native-tls", "gzip"], '
            'default-features=false }',
            'reqwest = { version = "0.12.28", features = ["blocking", "socks", '
            '"json", "native-tls", "gzip"], default-features=false }',
        ),
        (
            'reqwest = { git = "https://github.com/rustdesk-org/reqwest", '
            'features = ["blocking", "socks", "json", "rustls-tls", '
            '"rustls-tls-native-roots", "gzip"], default-features=false }',
            'reqwest = { version = "0.12.28", features = ["blocking", "socks", '
            '"json", "rustls-tls-native-roots", "gzip"], default-features=false }',
        ),
    )
    for legacy, desired in reqwest_replacements:
        content = cargo.read_text(encoding="utf-8")
        if desired not in content:
            replace_once(cargo, legacy, desired)

    content = cargo.read_text(encoding="utf-8")
    final_dev = '[dev-dependencies]\ntempfile = "3.27"\n\n'
    if final_dev not in content:
        legacy_dev = (
            '[dev-dependencies]\nrcgen = "0.9"\n\n',
            '[dev-dependencies]\nrcgen = "0.9"\n'
            'tokio-rustls = "0.23"\n\n',
            '[dev-dependencies]\nrcgen = "0.12.1"\n\n',
            '[dev-dependencies]\nrcgen = "0.13.2"\n\n',
            '[dev-dependencies]\ntempfile = "3.27"\n\n',
        )
        source = next((block for block in legacy_dev if block in content), None)
        if source is not None:
            replace_once(cargo, source, final_dev)
        elif "[dev-dependencies]" not in content:
            replace_once(
                cargo,
                "[build-dependencies]\n",
                final_dev + "[build-dependencies]\n",
            )
        elif 'rcgen = "0.9"\n' in content:
            replace_once(
                cargo,
                'rcgen = "0.9"\n',
                'tempfile = "3.27"\n',
            )
        elif 'rcgen = "0.12.1"\n' in content and 'tempfile = "3.27"\n' not in content:
            replace_once(
                cargo,
                'rcgen = "0.12.1"\n',
                'tempfile = "3.27"\n',
            )
        elif 'rcgen = "0.13.2"\n' in content and 'tempfile = "3.27"\n' not in content:
            replace_once(
                cargo,
                'rcgen = "0.13.2"\n',
                'tempfile = "3.27"\n',
            )
        elif 'tempfile = "3.27"\n' in content:
            pass
        else:
            raise RuntimeError(
                "upstream dev-dependencies changed; review test dependency pin"
            )


def patch_common(upstream: Path) -> None:
    common = upstream / "src/common.rs"
    content = common.read_text(encoding="utf-8")
    if "let arguments: Vec<(String, bool)>" not in content:
        replace_once(
            common,
            '''    let matches = App::new(name)
        .version(crate::version::VERSION)
        .author("Purslane Ltd. <info@rustdesk.com>")
        .about(about)
        .args_from_usage(args)
        .get_matches();
''',
            '''    let app = App::new(name)
        .version(crate::version::VERSION)
        .author("Purslane Ltd. <info@rustdesk.com>")
        .about(about)
        .args_from_usage(args);
    let arguments: Vec<(String, bool)> = app
        .get_arguments()
        .map(|argument| {
            (
                argument.get_id().to_owned(),
                argument.is_takes_value_set(),
            )
        })
        .collect();
    let matches = app.get_matches();
''',
        )
    content = common.read_text(encoding="utf-8")
    if "for (name, takes_value) in arguments" not in content:
        replace_once(
            common,
            '''    for (k, v) in matches.args {
        if let Some(v) = v.vals.first() {
            std::env::set_var(arg_name(k), v.to_string_lossy().to_string());
        }
    }
''',
            '''    for (name, takes_value) in arguments {
        if takes_value {
            if let Some(value) = matches.value_of(&name) {
                std::env::set_var(arg_name(&name), value);
            }
        } else if matches.is_present(&name) {
            std::env::set_var(arg_name(&name), "true");
        }
    }
''',
        )



def patch_relay(upstream: Path) -> None:
    relay = upstream / "src/relay_server.rs"
    content = relay.read_text(encoding="utf-8")
    if "stream.write(res.as_bytes()).await.ok();" in content:
        replace_once(
            relay,
            "stream.write(res.as_bytes()).await.ok();",
            "stream.write_all(res.as_bytes()).await.ok();",
        )
    content = relay.read_text(encoding="utf-8")
    if ".send(tungstenite::Message::Binary(bytes))" not in content:
        replace_once(
            relay,
            ".send(tungstenite::Message::Binary(bytes.to_vec()))",
            ".send(tungstenite::Message::Binary(bytes))",
        )
    content = relay.read_text(encoding="utf-8")
    if '"x-starry-version"' not in content:
        replace_once(
            relay,
            "        let callback = |req: &Request, response: Response| {\n",
            "        let callback = |req: &Request, mut response: Response| {\n",
        )
        replace_once(
            relay,
            "            Ok(response)\n"
            "        };\n"
            "        let ws_stream = tokio_tungstenite::accept_hdr_async(stream, callback).await?;\n",
            "            let version = format!(\n"
            "                \"{}-patch-v{}\",\n"
            "                env!(\"CARGO_PKG_VERSION\"),\n"
            "                include_str!(\"../PATCH_VERSION\").trim(),\n"
            "            );\n"
            "            if let Ok(value) =\n"
            "                tokio_tungstenite::tungstenite::http::HeaderValue::from_bytes(version.as_bytes())\n"
            "            {\n"
            "                response.headers_mut().insert(\"x-starry-version\", value);\n"
            "            }\n"
            "            Ok(response)\n"
            "        };\n"
            "        let ws_stream = tokio_tungstenite::accept_hdr_async(stream, callback).await?;\n",
        )


def patch_relay_quality_hbbr(upstream: Path) -> None:
    relay = upstream / "src/relay_server.rs"
    content = relay.read_text(encoding="utf-8")
    if "const STARRY_RELAY_TELEMETRY_SCHEMA: u32 =" in content:
        return

    if "protobuf::{Message as _, MessageField}," not in content:
        replace_once(
            relay,
            "    protobuf::Message as _,\n",
            "    protobuf::{Message as _, MessageField},\n",
        )

    content = relay.read_text(encoding="utf-8")
    if "static ACTIVE_RELAY_SESSIONS:" not in content:
        replace_once(
            relay,
            "static TOTAL_BANDWIDTH: AtomicUsize = AtomicUsize::new(1024 * 1024 * 1024); // in bit/s\n",
            "static TOTAL_BANDWIDTH: AtomicUsize = AtomicUsize::new(1024 * 1024 * 1024); // in bit/s\n"
            "static ACTIVE_RELAY_SESSIONS: AtomicUsize = AtomicUsize::new(0);\n"
            "static ACTIVE_RELAY_BANDWIDTH_BPS: AtomicUsize = AtomicUsize::new(0);\n"
            "static MAX_RELAY_SESSIONS: AtomicUsize = AtomicUsize::new(10_000);\n",
        )

    content = relay.read_text(encoding="utf-8")
    if '"STARRY_RELAY_MAX_SESSIONS"' not in content:
        replace_once(
            relay,
            "    log::info!(\n"
            "        \"TOTAL_BANDWIDTH: {}Mb/s\",\n"
            "        TOTAL_BANDWIDTH.load(Ordering::SeqCst) as f64 / 1024. / 1024.\n"
            "    );\n"
            "    let tmp = std::env::var(\"SINGLE_BANDWIDTH\")\n",
            "    log::info!(\n"
            "        \"TOTAL_BANDWIDTH: {}Mb/s\",\n"
            "        TOTAL_BANDWIDTH.load(Ordering::SeqCst) as f64 / 1024. / 1024.\n"
            "    );\n"
            "    let tmp = std::env::var(\"STARRY_RELAY_MAX_SESSIONS\")\n"
            "        .map(|value| value.parse::<usize>().unwrap_or(0).min(u32::MAX as usize))\n"
            "        .unwrap_or(0);\n"
            "    if tmp > 0 {\n"
            "        MAX_RELAY_SESSIONS.store(tmp, Ordering::SeqCst);\n"
            "    }\n"
            "    log::info!(\n"
            "        \"STARRY_RELAY_MAX_SESSIONS: {}\",\n"
            "        MAX_RELAY_SESSIONS.load(Ordering::SeqCst)\n"
            "    );\n"
            "    let tmp = std::env::var(\"SINGLE_BANDWIDTH\")\n",
        )

    content = relay.read_text(encoding="utf-8")
    legacy_capacity_parse = (
        "    let tmp = std::env::var(\"STARRY_RELAY_MAX_SESSIONS\")\n"
        "        .map(|value| value.parse::<usize>().unwrap_or(0))\n"
    )
    if legacy_capacity_parse in content:
        replace_once(
            relay,
            legacy_capacity_parse,
            "    let tmp = std::env::var(\"STARRY_RELAY_MAX_SESSIONS\")\n"
            "        .map(|value| value.parse::<usize>().unwrap_or(0).min(u32::MAX as usize))\n",
        )

    content = relay.read_text(encoding="utf-8")
    if '"STARRY_RELAY_CAPACITY_BANDWIDTH_BPS"' not in content:
        replace_once(
            relay,
            "    if tmp > 0. {\n"
            "        TOTAL_BANDWIDTH.store((tmp * 1024. * 1024.) as usize, Ordering::SeqCst);\n"
            "    }\n\n"
            "    log::info!(\n"
            "        \"TOTAL_BANDWIDTH: {}Mb/s\",\n",
            "    if tmp > 0. {\n"
            "        TOTAL_BANDWIDTH.store((tmp * 1024. * 1024.) as usize, Ordering::SeqCst);\n"
            "    }\n"
            "    if let Some(capacity) = std::env::var(\"STARRY_RELAY_CAPACITY_BANDWIDTH_BPS\")\n"
            "        .ok()\n"
            "        .and_then(|value| value.parse::<usize>().ok())\n"
            "        .filter(|value| *value > 0)\n"
            "    {\n"
            "        TOTAL_BANDWIDTH.store(capacity, Ordering::SeqCst);\n"
            "    }\n\n"
            "    log::info!(\n"
            "        \"TOTAL_BANDWIDTH: {}Mb/s\",\n",
        )

    content = relay.read_text(encoding="utf-8")
    if "fn current_relay_load() -> StarryRelayLoad" not in content:
        helpers = '''fn starry_version() -> String {
    format!(
        "{}-patch-v{}",
        env!("CARGO_PKG_VERSION"),
        include_str!("../PATCH_VERSION").trim(),
    )
}

fn load_basis_points(value: usize, capacity: usize) -> u32 {
    if capacity == 0 {
        return 10_000;
    }
    value
        .saturating_mul(10_000)
        .checked_div(capacity)
        .unwrap_or(10_000)
        .min(10_000) as u32
}

fn current_relay_load() -> StarryRelayLoad {
    let active_sessions = ACTIVE_RELAY_SESSIONS.load(Ordering::Relaxed);
    let capacity_sessions = MAX_RELAY_SESSIONS.load(Ordering::Relaxed).max(1);
    let bandwidth_bps = ACTIVE_RELAY_BANDWIDTH_BPS.load(Ordering::Relaxed);
    let capacity_bandwidth_bps = TOTAL_BANDWIDTH.load(Ordering::Relaxed).max(1);
    StarryRelayLoad {
        load_basis_points: load_basis_points(active_sessions, capacity_sessions)
            .max(load_basis_points(bandwidth_bps, capacity_bandwidth_bps)),
        active_sessions: active_sessions.min(u32::MAX as usize) as u32,
        capacity_sessions: capacity_sessions.min(u32::MAX as usize) as u32,
        bandwidth_bps: bandwidth_bps as u64,
        capacity_bandwidth_bps: capacity_bandwidth_bps as u64,
        ..Default::default()
    }
}

fn atomic_saturating_sub(value: &AtomicUsize, amount: usize) {
    let _ = value.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
        Some(current.saturating_sub(amount))
    });
}

struct RelayLoadGuard {
    bandwidth_bps: usize,
}

impl RelayLoadGuard {
    fn new() -> Self {
        ACTIVE_RELAY_SESSIONS.fetch_add(1, Ordering::SeqCst);
        Self { bandwidth_bps: 0 }
    }

    fn update(&mut self, bandwidth_bps: usize) {
        if bandwidth_bps >= self.bandwidth_bps {
            ACTIVE_RELAY_BANDWIDTH_BPS.fetch_add(
                bandwidth_bps - self.bandwidth_bps,
                Ordering::SeqCst,
            );
        } else {
            atomic_saturating_sub(
                &ACTIVE_RELAY_BANDWIDTH_BPS,
                self.bandwidth_bps - bandwidth_bps,
            );
        }
        self.bandwidth_bps = bandwidth_bps;
    }
}

impl Drop for RelayLoadGuard {
    fn drop(&mut self) {
        atomic_saturating_sub(&ACTIVE_RELAY_BANDWIDTH_BPS, self.bandwidth_bps);
        atomic_saturating_sub(&ACTIVE_RELAY_SESSIONS, 1);
    }
}

#[cfg(test)]
mod starry_telemetry_tests {
    use super::*;

    #[test]
    fn probe_rate_limiter_enforces_global_per_ip_and_dimension_bounds() {
        PROBE_PER_IP_PER_MINUTE.store(1, Ordering::SeqCst);
        PROBE_GLOBAL_PER_MINUTE.store(2, Ordering::SeqCst);
        let mut limiter = ProbeRateLimiter::default();
        let first: IpAddr = "192.0.2.1".parse().unwrap();
        let second: IpAddr = "198.51.100.1".parse().unwrap();
        assert!(limiter.allow(first, 60));
        assert!(!limiter.allow(first, 60));
        assert!(limiter.allow(second, 60));
        assert!(!limiter.allow("203.0.113.1".parse().unwrap(), 60));
        assert!(limiter.per_ip.len() <= PROBE_IP_BUCKET_CAPACITY);
        PROBE_PER_IP_PER_MINUTE.store(120, Ordering::SeqCst);
        PROBE_GLOBAL_PER_MINUTE.store(10_000, Ordering::SeqCst);
    }

    #[test]
    fn telemetry_replay_cache_is_bounded_and_rejects_reuse() {
        let mut cache = TelemetryReplayCache::default();
        assert!(cache.insert_fresh(100, "nonce-a"));
        assert!(!cache.insert_fresh(100, "nonce-a"));
        for index in 0..TELEMETRY_REPLAY_CAPACITY + 32 {
            assert!(cache.insert_fresh(100, &format!("nonce-{index}")));
        }
        assert!(cache.order.len() <= TELEMETRY_REPLAY_CAPACITY);
        assert!(cache.entries.len() <= TELEMETRY_REPLAY_CAPACITY);
    }
}

'''
        replace_once(relay, "async fn check_cmd(cmd: &str, limiter: Limiter) -> String {\n", helpers + "async fn check_cmd(cmd: &str, limiter: Limiter) -> String {\n")

    content = relay.read_text(encoding="utf-8")
    if '"x-starry-load-bps"' not in content:
        replace_once(
            relay,
            "            let version = format!(\n"
            "                \"{}-patch-v{}\",\n"
            "                env!(\"CARGO_PKG_VERSION\"),\n"
            "                include_str!(\"../PATCH_VERSION\").trim(),\n"
            "            );\n",
            "            let version = starry_version();\n",
        )
        replace_once(
            relay,
            "                response.headers_mut().insert(\"x-starry-version\", value);\n"
            "            }\n"
            "            Ok(response)\n",
            "                response.headers_mut().insert(\"x-starry-version\", value);\n"
            "            }\n"
            "            let load = current_relay_load();\n"
            "            for (name, value) in [\n"
            "                (\"x-starry-relay-probe-protocol\", \"1\".to_owned()),\n"
            "                (\"x-starry-relay-load-protocol\", \"1\".to_owned()),\n"
            "                (\"x-starry-load-bps\", load.load_basis_points.to_string()),\n"
            "                (\"x-starry-active-sessions\", load.active_sessions.to_string()),\n"
            "                (\"x-starry-capacity-sessions\", load.capacity_sessions.to_string()),\n"
            "                (\"x-starry-bandwidth-bps\", load.bandwidth_bps.to_string()),\n"
            "                (\n"
            "                    \"x-starry-capacity-bandwidth-bps\",\n"
            "                    load.capacity_bandwidth_bps.to_string(),\n"
            "                ),\n"
            "            ] {\n"
            "                if let Ok(value) =\n"
            "                    tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&value)\n"
            "                {\n"
            "                    response.headers_mut().insert(name, value);\n"
            "                }\n"
            "            }\n"
            "            Ok(response)\n",
        )

    content = relay.read_text(encoding="utf-8")
    if '"x-starry-relay-probe-protocol"' not in content:
        replace_once(
            relay,
            '''            for (name, value) in [
                ("x-starry-load-bps", load.load_basis_points.to_string()),
''',
            '''            for (name, value) in [
                ("x-starry-relay-probe-protocol", "1".to_owned()),
                ("x-starry-load-bps", load.load_basis_points.to_string()),
''',
        )
    content = relay.read_text(encoding="utf-8")
    if '"x-starry-relay-load-protocol"' not in content:
        replace_once(
            relay,
            '                ("x-starry-relay-probe-protocol", "1".to_owned()),\n',
            '                ("x-starry-relay-probe-protocol", "1".to_owned()),\n'
            '                ("x-starry-relay-load-protocol", "1".to_owned()),\n',
        )

    content = relay.read_text(encoding="utf-8")
    if "RelayProbeRequest(probe)" not in content:
        replacement = '''async fn make_pair_(stream: impl StreamTrait, addr: SocketAddr, key: &str, limiter: Limiter) {
    let mut stream = stream;
    if let Ok(Some(Ok(bytes))) = timeout(30_000, stream.recv()).await {
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
            match msg_in.union {
                Some(rendezvous_message::Union::RelayProbeRequest(probe)) => {
                    if probe.protocol_version != 1 || probe.nonce.len() != 16 {
                        log::debug!("Rejected malformed Relay quality probe from {}", addr);
                        return;
                    }
                    let mut response = RendezvousMessage::new();
                    response.set_relay_probe_response(RelayProbeResponse {
                        protocol_version: 1,
                        nonce: probe.nonce,
                        load: MessageField::some(current_relay_load()),
                        starry_version: starry_version(),
                        relay_probe_protocol: 1,
                        relay_load_protocol: 1,
                        ..Default::default()
                    });
                    if let Ok(bytes) = response.write_to_bytes() {
                        allow_err!(stream.send_raw(bytes.into()).await);
                    }
                }
                Some(rendezvous_message::Union::RequestRelay(rf)) => {
                    if !key.is_empty() && rf.licence_key != key {
                        log::warn!("Relay authentication failed from {} - invalid key", addr);
                        return;
                    }
                    if !rf.uuid.is_empty() {
                        let mut peer = PEERS.lock().await.remove(&rf.uuid);
                        if let Some(peer) = peer.as_mut() {
                            log::info!("Relayrequest {} from {} got paired", rf.uuid, addr);
                            let id = format!("{}:{}", addr.ip(), addr.port());
                            USAGE.write().await.insert(id.clone(), Default::default());
                            if !stream.is_ws() && !peer.is_ws() {
                                peer.set_raw();
                                stream.set_raw();
                                log::info!("Both are raw");
                            }
                            let mut load_guard = RelayLoadGuard::new();
                            if let Err(err) = relay(
                                addr,
                                &mut stream,
                                peer,
                                limiter,
                                id.clone(),
                                &mut load_guard,
                            )
                            .await
                            {
                                log::info!("Relay of {} closed: {}", addr, err);
                            } else {
                                log::info!("Relay of {} closed", addr);
                            }
                            USAGE.write().await.remove(&id);
                        } else {
                            log::info!("New relay request {} from {}", rf.uuid, addr);
                            PEERS.lock().await.insert(rf.uuid.clone(), Box::new(stream));
                            sleep(30.).await;
                            PEERS.lock().await.remove(&rf.uuid);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

'''
        replace_between_once(
            relay,
            "async fn make_pair_(stream: impl StreamTrait, addr: SocketAddr, key: &str, limiter: Limiter) {\n",
            "async fn relay(\n",
            replacement,
        )

    content = relay.read_text(encoding="utf-8")
    if "relay_probe_protocol: 1," not in content:
        replace_once(
            relay,
            '''                        load: MessageField::some(current_relay_load()),
                        starry_version: starry_version(),
                        ..Default::default()
''',
            '''                        load: MessageField::some(current_relay_load()),
                        starry_version: starry_version(),
                        relay_probe_protocol: 1,
                        ..Default::default()
''',
        )
    content = relay.read_text(encoding="utf-8")
    if "relay_load_protocol: 1," not in content:
        replace_once(
            relay,
            "                        relay_probe_protocol: 1,\n",
            "                        relay_probe_protocol: 1,\n"
            "                        relay_load_protocol: 1,\n",
        )

    content = relay.read_text(encoding="utf-8")
    if "load_guard: &mut RelayLoadGuard," not in content:
        replace_once(
            relay,
            "    total_limiter: Limiter,\n"
            "    id: String,\n"
            ") -> ResultType<()> {\n",
            "    total_limiter: Limiter,\n"
            "    id: String,\n"
            "    load_guard: &mut RelayLoadGuard,\n"
            ") -> ResultType<()> {\n",
        )
        replace_once(
            relay,
            "            let speed = total_s / n;\n"
            "            if speed > highest_s {\n",
            "            let speed = total_s / n;\n"
            "            load_guard.update(speed.saturating_mul(1_000));\n"
            "            if speed > highest_s {\n",
        )

    patch_hbbr_telemetry_observability_v1(relay)


def patch_hbbr_telemetry_observability_v1(relay: Path) -> None:
    """Separate public probes from authenticated, bounded HBBR telemetry."""
    content = relay.read_text(encoding="utf-8")
    if "const STARRY_RELAY_TELEMETRY_SCHEMA: u32 =" in content:
        return

    replace_once(
        relay,
        "    protobuf::{Message as _, MessageField},\n",
        "    protobuf::Message as _,\n",
    )
    replace_once(
        relay,
        "use sodiumoxide::crypto::sign;\n",
        "use sha2::{Digest, Sha256};\nuse sodiumoxide::crypto::{auth, sign};\n",
    )
    replace_once(
        relay,
        '''use std::{
    collections::{HashMap, HashSet},
    io::prelude::*,
    io::Error,
    net::SocketAddr,
    sync::atomic::{AtomicUsize, Ordering},
};
''',
        '''use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::prelude::*,
    io::Error,
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
''',
    )
    replace_once(
        relay,
        "    static ref PEERS: Mutex<HashMap<String, Box<dyn StreamTrait>>> = Default::default();\n",
        "    static ref PEERS: Mutex<HashMap<String, PendingPeer>> = Default::default();\n",
    )
    replace_once(
        relay,
        "    static ref BLOCKLIST: RwLock<HashSet<String>> = Default::default();\n",
        "    static ref BLOCKLIST: RwLock<HashSet<String>> = Default::default();\n"
        "    static ref PROCESS_STARTED: Instant = Instant::now();\n"
        "    static ref PROCESS_INSTANCE_ID: String = uuid::Uuid::now_v7().to_string();\n"
        "    static ref TELEMETRY_AUTH_KEY: Option<auth::Key> = load_telemetry_auth_key();\n"
        "    static ref TELEMETRY_REPLAY_CACHE: StdMutex<TelemetryReplayCache> = Default::default();\n"
        "    static ref PROBE_RATE_LIMITER: StdMutex<ProbeRateLimiter> = Default::default();\n",
    )
    replace_once(
        relay,
        '''static ACTIVE_RELAY_SESSIONS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_RELAY_BANDWIDTH_BPS: AtomicUsize = AtomicUsize::new(0);
static MAX_RELAY_SESSIONS: AtomicUsize = AtomicUsize::new(10_000);
''',
        '''static ACTIVE_RELAY_SESSIONS: AtomicUsize = AtomicUsize::new(0);
static PENDING_RELAY_PAIRS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_RELAY_BANDWIDTH_BPS: AtomicUsize = AtomicUsize::new(0);
static MAX_RELAY_SESSIONS: AtomicUsize = AtomicUsize::new(10_000);
static ADMISSION_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static PROBE_MALFORMED: AtomicU64 = AtomicU64::new(0);
static PROBE_UNSUPPORTED: AtomicU64 = AtomicU64::new(0);
static PROBE_RATE_LIMITED: AtomicU64 = AtomicU64::new(0);
static PROBE_SUCCESSFUL: AtomicU64 = AtomicU64::new(0);
static TELEMETRY_AUTH_FAILURES: AtomicU64 = AtomicU64::new(0);
static TELEMETRY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static PENDING_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static PROBE_PER_IP_PER_MINUTE: AtomicUsize = AtomicUsize::new(120);
static PROBE_GLOBAL_PER_MINUTE: AtomicUsize = AtomicUsize::new(10_000);
const STARRY_RELAY_TELEMETRY_SCHEMA: u32 = crate::fast_media_relay::TELEMETRY_SCHEMA_VERSION;
const TELEMETRY_AUTH_CLOCK_SKEW_SECONDS: u64 = 30;
const TELEMETRY_REPLAY_CAPACITY: usize = 4_096;
const PROBE_IP_BUCKET_CAPACITY: usize = 4_096;
const BANDWIDTH_EMA_ALPHA_BASIS_POINTS: u32 = 2_500;
''',
    )

    content = relay.read_text(encoding="utf-8")
    start_call = '''pub async fn start(port: &str, key: &str) -> ResultType<()> {
    let key = get_server_sk(key);
'''
    replace_once(
        relay,
        start_call,
        '''pub async fn start(port: &str, key: &str) -> ResultType<()> {
    if sodiumoxide::init().is_err() {
        bail!("cannot initialize telemetry authentication");
    }
    let _ = &*TELEMETRY_AUTH_KEY;
    let key = get_server_sk(key);
    if let Err(error) = crate::relay_enrollment::preflight(&key) {
        bail!("Relay enrollment preflight failed: {error}");
    }
    crate::fast_media_relay::spawn(&key);
''',
    )

    replace_once(
        relay,
        '''    log::info!(
        "STARRY_RELAY_MAX_SESSIONS: {}",
        MAX_RELAY_SESSIONS.load(Ordering::SeqCst)
    );
    let tmp = std::env::var("SINGLE_BANDWIDTH")
''',
        '''    log::info!(
        "STARRY_RELAY_MAX_SESSIONS: {} (enforced admission limit)",
        MAX_RELAY_SESSIONS.load(Ordering::SeqCst)
    );
    set_bounded_rate_from_env(
        "STARRY_RELAY_PROBE_PER_IP_PER_MINUTE",
        &PROBE_PER_IP_PER_MINUTE,
    );
    set_bounded_rate_from_env(
        "STARRY_RELAY_PROBE_GLOBAL_PER_MINUTE",
        &PROBE_GLOBAL_PER_MINUTE,
    );
    log::info!(
        "Relay probe limits: per_ip_per_minute={}, global_per_minute={}",
        PROBE_PER_IP_PER_MINUTE.load(Ordering::SeqCst),
        PROBE_GLOBAL_PER_MINUTE.load(Ordering::SeqCst),
    );
    let tmp = std::env::var("SINGLE_BANDWIDTH")
''',
    )

    helpers = r'''fn starry_version() -> String {
    format!(
        "{}-patch-v{}",
        env!("CARGO_PKG_VERSION"),
        include_str!("../PATCH_VERSION").trim(),
    )
}

struct PendingPeer {
    stream: Box<dyn StreamTrait>,
    generation: u64,
}

#[derive(Default)]
struct TelemetryReplayCache {
    entries: HashSet<String>,
    order: VecDeque<(u64, String)>,
}

impl TelemetryReplayCache {
    fn insert_fresh(&mut self, now: u64, nonce: &str) -> bool {
        while self
            .order
            .front()
            .map(|(seen_at, _)| now.saturating_sub(*seen_at) > TELEMETRY_AUTH_CLOCK_SKEW_SECONDS)
            .unwrap_or(false)
        {
            if let Some((_, expired)) = self.order.pop_front() {
                self.entries.remove(&expired);
            }
        }
        if self.entries.contains(nonce) {
            return false;
        }
        while self.order.len() >= TELEMETRY_REPLAY_CAPACITY {
            if let Some((_, oldest)) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(nonce.to_owned());
        self.order.push_back((now, nonce.to_owned()));
        true
    }
}

#[derive(Default)]
struct ProbeRateLimiter {
    window: u64,
    global: usize,
    per_ip: HashMap<IpAddr, usize>,
}

impl ProbeRateLimiter {
    fn allow(&mut self, source: IpAddr, now: u64) -> bool {
        let window = now / 60;
        if self.window != window {
            self.window = window;
            self.global = 0;
            self.per_ip.clear();
        }
        let global_limit = PROBE_GLOBAL_PER_MINUTE.load(Ordering::Relaxed).max(1);
        let per_ip_limit = PROBE_PER_IP_PER_MINUTE.load(Ordering::Relaxed).max(1);
        if self.global >= global_limit {
            return false;
        }
        if !self.per_ip.contains_key(&source) && self.per_ip.len() >= PROBE_IP_BUCKET_CAPACITY {
            return false;
        }
        let count = self.per_ip.entry(source).or_default();
        if *count >= per_ip_limit {
            return false;
        }
        self.global = self.global.saturating_add(1);
        *count = count.saturating_add(1);
        true
    }
}

fn set_bounded_rate_from_env(name: &str, target: &AtomicUsize) {
    let configured = std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=1_000_000).contains(value));
    if let Some(configured) = configured {
        target.store(configured, Ordering::SeqCst);
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn load_telemetry_auth_key() -> Option<auth::Key> {
    let path = std::env::var("STARRY_RELAY_TELEMETRY_SECRET_FILE")
        .ok()
        .filter(|path| !path.is_empty())?;
    let mut secret = match std::fs::read(&path) {
        Ok(secret) => secret,
        Err(error) => {
            log::error!("Cannot read STARRY_RELAY_TELEMETRY_SECRET_FILE: {error}");
            return None;
        }
    };
    while matches!(secret.last(), Some(b'\n' | b'\r')) {
        secret.pop();
    }
    if !(32..=1_024).contains(&secret.len()) {
        log::error!("STARRY_RELAY_TELEMETRY_SECRET_FILE must contain 32..1024 bytes");
        secret.fill(0);
        return None;
    }
    let digest = Sha256::digest(&secret);
    secret.fill(0);
    auth::Key::from_slice(&digest)
}

fn hex_encode(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_decode_tag(value: &str) -> Option<auth::Tag> {
    if value.len() != auth::TAGBYTES * 2 {
        return None;
    }
    let mut decoded = vec![0_u8; auth::TAGBYTES];
    for (index, output) in decoded.iter_mut().enumerate() {
        let high = hex_nibble(value.as_bytes()[index * 2])?;
        let low = hex_nibble(value.as_bytes()[index * 2 + 1])?;
        *output = (high << 4) | low;
    }
    auth::Tag::from_slice(&decoded)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn bounded_request_header<'a>(request: &'a http::Request<()>, name: &str, max: usize) -> Option<&'a str> {
    let value = request.headers().get(name)?.to_str().ok()?;
    (!value.is_empty() && value.len() <= max).then_some(value)
}

fn authenticate_telemetry_request(request: &http::Request<()>) -> Option<String> {
    let key = TELEMETRY_AUTH_KEY.as_ref()?;
    let now = unix_seconds();
    let timestamp = bounded_request_header(request, "x-starry-telemetry-timestamp", 20)?;
    let timestamp_value = timestamp.parse::<u64>().ok()?;
    if now.abs_diff(timestamp_value) > TELEMETRY_AUTH_CLOCK_SKEW_SECONDS {
        return None;
    }
    let nonce = bounded_request_header(request, "x-starry-telemetry-nonce", 32)?;
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let signature = bounded_request_header(request, "x-starry-telemetry-auth", 64)?;
    let tag = hex_decode_tag(signature)?;
    let canonical = format!(
        "starry-telemetry-request-v1\n{timestamp}\n{nonce}\n/ws/telemetry"
    );
    if !auth::verify(&tag, canonical.as_bytes(), key) {
        return None;
    }
    let mut replay = TELEMETRY_REPLAY_CACHE.lock().ok()?;
    replay.insert_fresh(now, nonce).then(|| nonce.to_owned())
}

fn load_basis_points(value: usize, capacity: usize) -> u32 {
    if capacity == 0 {
        return 10_000;
    }
    value
        .saturating_mul(10_000)
        .checked_div(capacity)
        .unwrap_or(10_000)
        .min(10_000) as u32
}

fn is_draining() -> bool {
    let statically_draining = std::env::var("STARRY_RELAY_DRAINING")
        .ok()
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);
    statically_draining
        || std::env::var("STARRY_RELAY_DRAINING_FILE")
            .ok()
            .filter(|path| !path.is_empty())
            .map(|path| Path::new(&path).is_file())
            .unwrap_or(false)
}

fn current_relay_load() -> StarryRelayLoad {
    let active_sessions = ACTIVE_RELAY_SESSIONS.load(Ordering::Relaxed);
    let capacity_sessions = MAX_RELAY_SESSIONS.load(Ordering::Relaxed).max(1);
    let bandwidth_bps = ACTIVE_RELAY_BANDWIDTH_BPS.load(Ordering::Relaxed);
    let capacity_bandwidth_bps = TOTAL_BANDWIDTH.load(Ordering::Relaxed).max(1);
    StarryRelayLoad {
        load_basis_points: load_basis_points(active_sessions, capacity_sessions)
            .max(load_basis_points(bandwidth_bps, capacity_bandwidth_bps)),
        active_sessions: active_sessions.min(u32::MAX as usize) as u32,
        capacity_sessions: capacity_sessions.min(u32::MAX as usize) as u32,
        bandwidth_bps: bandwidth_bps as u64,
        capacity_bandwidth_bps: capacity_bandwidth_bps as u64,
        ..Default::default()
    }
}

fn telemetry_payload() -> String {
    let load = current_relay_load();
    let fast_media = crate::fast_media_relay::runtime_snapshot();
    let draining = is_draining();
    let admission_open = !draining && load.active_sessions < load.capacity_sessions;
    let sequence = TELEMETRY_SEQUENCE.fetch_add(1, Ordering::SeqCst).saturating_add(1);
    let fast_media_payload = serde_json::json!({
        "protocol": crate::fast_media_relay::PROTOCOL_VERSION,
        "enabled": fast_media.enabled,
        "healthy": fast_media.healthy,
        "udp_port": fast_media.udp_port.unwrap_or_default(),
        "active_allocations": fast_media.active_allocations,
        "active_streams": fast_media.active_streams,
        "hello_accepted": fast_media.hello_accepted,
        "cookie_rejected": fast_media.cookie_rejected,
        "bind_succeeded": fast_media.bind_succeeded,
        "bind_rejected": fast_media.bind_rejected,
        "grant_rejected": fast_media.grant_rejected,
        "role_mismatch": fast_media.role_mismatch,
        "session_mismatch": fast_media.session_mismatch,
        "allocation_mismatch": fast_media.allocation_mismatch,
        "rebinds": fast_media.rebinds,
        "forwarded_packets": fast_media.forwarded_packets,
        "forwarded_bytes": fast_media.forwarded_bytes,
        "dropped_packets": fast_media.dropped_packets,
        "rate_limited": fast_media.rate_limited,
        "replay_rejected": fast_media.replay_rejected,
        "expired_allocations": fast_media.expired_allocations,
        "listener_failures": fast_media.listener_failures,
    });
    let payload = serde_json::json!({
        "telemetry_schema": STARRY_RELAY_TELEMETRY_SCHEMA,
        "process_instance_id": PROCESS_INSTANCE_ID.as_str(),
        "sequence": sequence,
        "observed_at_unix_ms": unix_millis(),
        "uptime_seconds": PROCESS_STARTED.elapsed().as_secs(),
        "version": starry_version(),
        "relay_probe_protocol": 1,
        "relay_load_protocol": 1,
        "fast_media": fast_media_payload,
        "load_basis_points": load.load_basis_points,
        "active_sessions": load.active_sessions,
        "pending_pairs": PENDING_RELAY_PAIRS.load(Ordering::Relaxed).min(u32::MAX as usize) as u32,
        "capacity_sessions": load.capacity_sessions,
        "bandwidth_bps": load.bandwidth_bps,
        "bandwidth_ema_alpha_basis_points": BANDWIDTH_EMA_ALPHA_BASIS_POINTS,
        "capacity_bandwidth_bps": load.capacity_bandwidth_bps,
        "draining": draining,
        "admission_open": admission_open,
        "admission_rejections": ADMISSION_REJECTIONS.load(Ordering::Relaxed),
        "probe_malformed": PROBE_MALFORMED.load(Ordering::Relaxed),
        "probe_unsupported": PROBE_UNSUPPORTED.load(Ordering::Relaxed),
        "probe_rate_limited": PROBE_RATE_LIMITED.load(Ordering::Relaxed),
        "probe_successful": PROBE_SUCCESSFUL.load(Ordering::Relaxed),
        "telemetry_auth_failures": TELEMETRY_AUTH_FAILURES.load(Ordering::Relaxed),
    });
    base64::encode_config(
        serde_json::to_vec(&payload).unwrap_or_default(),
        base64::URL_SAFE_NO_PAD,
    )
}

fn add_public_capability_headers(response: &mut http::Response<()>) {
    for (name, value) in [
        ("x-starry-version", starry_version()),
        ("x-starry-relay-probe-protocol", "1".to_owned()),
        ("x-starry-relay-load-protocol", "1".to_owned()),
    ] {
        if let Ok(value) = http::HeaderValue::from_str(&value) {
            response.headers_mut().insert(name, value);
        }
    }
}

fn add_authenticated_telemetry_headers(
    response: &mut http::Response<()>,
    request_nonce: &str,
) -> bool {
    let Some(key) = TELEMETRY_AUTH_KEY.as_ref() else {
        return false;
    };
    let payload = telemetry_payload();
    let canonical = format!(
        "starry-telemetry-response-v1\n{request_nonce}\n{payload}"
    );
    let signature = hex_encode(auth::authenticate(canonical.as_bytes(), key).as_ref());
    for (name, value) in [
        ("x-starry-telemetry", payload),
        ("x-starry-telemetry-auth", signature),
    ] {
        let Ok(value) = http::HeaderValue::from_str(&value) else {
            return false;
        };
        response.headers_mut().insert(name, value);
    }
    true
}

fn increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

fn atomic_saturating_sub(value: &AtomicUsize, amount: usize) {
    let _ = value.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
        Some(current.saturating_sub(amount))
    });
}

fn admission_available() -> bool {
    !is_draining()
        && ACTIVE_RELAY_SESSIONS.load(Ordering::SeqCst)
            < MAX_RELAY_SESSIONS.load(Ordering::SeqCst).max(1)
}

struct RelayLoadGuard {
    bandwidth_bps: usize,
}

impl RelayLoadGuard {
    fn try_new() -> Option<Self> {
        if is_draining() {
            return None;
        }
        let capacity = MAX_RELAY_SESSIONS.load(Ordering::SeqCst).max(1);
        ACTIVE_RELAY_SESSIONS
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |active| {
                (active < capacity).then_some(active.saturating_add(1))
            })
            .ok()?;
        Some(Self { bandwidth_bps: 0 })
    }

    fn update(&mut self, sample_bps: usize) {
        let bandwidth_bps = if self.bandwidth_bps == 0 {
            sample_bps
        } else {
            self.bandwidth_bps
                .saturating_mul(3)
                .saturating_add(sample_bps)
                / 4
        };
        if bandwidth_bps >= self.bandwidth_bps {
            ACTIVE_RELAY_BANDWIDTH_BPS.fetch_add(
                bandwidth_bps - self.bandwidth_bps,
                Ordering::SeqCst,
            );
        } else {
            atomic_saturating_sub(
                &ACTIVE_RELAY_BANDWIDTH_BPS,
                self.bandwidth_bps - bandwidth_bps,
            );
        }
        self.bandwidth_bps = bandwidth_bps;
    }
}

impl Drop for RelayLoadGuard {
    fn drop(&mut self) {
        atomic_saturating_sub(&ACTIVE_RELAY_BANDWIDTH_BPS, self.bandwidth_bps);
        atomic_saturating_sub(&ACTIVE_RELAY_SESSIONS, 1);
    }
}

#[cfg(test)]
mod starry_telemetry_tests {
    use super::*;

    #[test]
    fn probe_rate_limiter_enforces_global_per_ip_and_dimension_bounds() {
        PROBE_PER_IP_PER_MINUTE.store(1, Ordering::SeqCst);
        PROBE_GLOBAL_PER_MINUTE.store(2, Ordering::SeqCst);
        let mut limiter = ProbeRateLimiter::default();
        let first: IpAddr = "192.0.2.1".parse().unwrap();
        let second: IpAddr = "198.51.100.1".parse().unwrap();
        assert!(limiter.allow(first, 60));
        assert!(!limiter.allow(first, 60));
        assert!(limiter.allow(second, 60));
        assert!(!limiter.allow("203.0.113.1".parse().unwrap(), 60));
        assert!(limiter.per_ip.len() <= PROBE_IP_BUCKET_CAPACITY);
        PROBE_PER_IP_PER_MINUTE.store(120, Ordering::SeqCst);
        PROBE_GLOBAL_PER_MINUTE.store(10_000, Ordering::SeqCst);
    }

    #[test]
    fn telemetry_replay_cache_is_bounded_and_rejects_reuse() {
        let mut cache = TelemetryReplayCache::default();
        assert!(cache.insert_fresh(100, "nonce-a"));
        assert!(!cache.insert_fresh(100, "nonce-a"));
        for index in 0..TELEMETRY_REPLAY_CAPACITY + 32 {
            assert!(cache.insert_fresh(100, &format!("nonce-{index}")));
        }
        assert!(cache.order.len() <= TELEMETRY_REPLAY_CAPACITY);
        assert!(cache.entries.len() <= TELEMETRY_REPLAY_CAPACITY);
    }
}

'''
    replace_between_once(
        relay,
        "fn starry_version() -> String {\n",
        "async fn check_cmd(cmd: &str, limiter: Limiter) -> String {\n",
        helpers,
    )

    pairing = r'''async fn make_pair(
    stream: TcpStream,
    mut addr: SocketAddr,
    key: &str,
    limiter: Limiter,
    ws: bool,
) -> ResultType<()> {
    let probe_source_ip = addr.ip();
    if ws {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        let telemetry_connection = Arc::new(AtomicBool::new(false));
        let telemetry_flag = telemetry_connection.clone();
        let callback = move |req: &Request, mut response: Response| {
            if req.uri().path() == "/ws/telemetry" {
                if req.uri().query().is_some() {
                    increment(&TELEMETRY_AUTH_FAILURES);
                    return Err(http::Response::builder()
                        .status(http::StatusCode::UNAUTHORIZED)
                        .body(Some("Unauthorized".to_owned()))
                        .unwrap());
                }
                let Some(request_nonce) = authenticate_telemetry_request(req) else {
                    increment(&TELEMETRY_AUTH_FAILURES);
                    return Err(http::Response::builder()
                        .status(http::StatusCode::UNAUTHORIZED)
                        .body(Some("Unauthorized".to_owned()))
                        .unwrap());
                };
                add_public_capability_headers(&mut response);
                if !add_authenticated_telemetry_headers(&mut response, &request_nonce) {
                    return Err(http::Response::builder()
                        .status(http::StatusCode::SERVICE_UNAVAILABLE)
                        .body(Some("Unavailable".to_owned()))
                        .unwrap());
                }
                telemetry_flag.store(true, Ordering::SeqCst);
                return Ok(response);
            }
            let headers = req.headers();
            let real_ip = headers
                .get("X-Real-IP")
                .or_else(|| headers.get("X-Forwarded-For"))
                .and_then(|header_value| header_value.to_str().ok());
            if let Some(ip) = real_ip {
                if ip.contains('.') {
                    addr = format!("{ip}:0").parse().unwrap_or(addr);
                } else {
                    addr = format!("[{ip}]:0").parse().unwrap_or(addr);
                }
            }
            add_public_capability_headers(&mut response);
            Ok(response)
        };
        let mut ws_stream = tokio_tungstenite::accept_hdr_async(stream, callback).await?;
        if telemetry_connection.load(Ordering::SeqCst) {
            allow_err!(ws_stream.close(None).await);
            return Ok(());
        }
        make_pair_(ws_stream, addr, probe_source_ip, key, limiter).await;
    } else {
        make_pair_(
            FramedStream::from(stream, addr),
            addr,
            probe_source_ip,
            key,
            limiter,
        )
        .await;
    }
    Ok(())
}

async fn make_pair_(
    stream: impl StreamTrait,
    addr: SocketAddr,
    probe_source_ip: IpAddr,
    key: &str,
    limiter: Limiter,
) {
    let mut stream = stream;
    if let Ok(Some(Ok(bytes))) = timeout(30_000, stream.recv()).await {
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
            match msg_in.union {
                Some(rendezvous_message::Union::RelayProbeRequest(probe)) => {
                    let allowed = PROBE_RATE_LIMITER
                        .lock()
                        .map(|mut limiter| limiter.allow(probe_source_ip, unix_seconds()))
                        .unwrap_or(false);
                    if !allowed {
                        increment(&PROBE_RATE_LIMITED);
                        return;
                    }
                    if probe.nonce.len() != 16 {
                        increment(&PROBE_MALFORMED);
                        return;
                    }
                    if probe.protocol_version != 1 {
                        increment(&PROBE_UNSUPPORTED);
                        return;
                    }
                    let mut response = RendezvousMessage::new();
                    response.set_relay_probe_response(RelayProbeResponse {
                        protocol_version: 1,
                        nonce: probe.nonce,
                        starry_version: starry_version(),
                        relay_probe_protocol: 1,
                        relay_load_protocol: 1,
                        ..Default::default()
                    });
                    if let Ok(bytes) = response.write_to_bytes() {
                        if stream.send_raw(bytes.into()).await.is_ok() {
                            increment(&PROBE_SUCCESSFUL);
                        }
                    }
                }
                Some(rendezvous_message::Union::RequestRelay(rf)) => {
                    if !key.is_empty() && rf.licence_key != key {
                        log::warn!("Relay authentication failed from {} - invalid key", addr);
                        return;
                    }
                    if rf.uuid.is_empty() {
                        return;
                    }
                    let pending = PEERS.lock().await.remove(&rf.uuid);
                    if let Some(mut pending) = pending {
                        atomic_saturating_sub(&PENDING_RELAY_PAIRS, 1);
                        let Some(mut load_guard) = RelayLoadGuard::try_new() else {
                            increment(&ADMISSION_REJECTIONS);
                            return;
                        };
                        let peer = &mut pending.stream;
                        log::info!("Relayrequest {} from {} got paired", rf.uuid, addr);
                        let id = format!("{}:{}", addr.ip(), addr.port());
                        USAGE.write().await.insert(id.clone(), Default::default());
                        if !stream.is_ws() && !peer.is_ws() {
                            peer.set_raw();
                            stream.set_raw();
                            log::info!("Both are raw");
                        }
                        if let Err(err) = relay(
                            addr,
                            &mut stream,
                            peer,
                            limiter,
                            id.clone(),
                            &mut load_guard,
                        )
                        .await
                        {
                            log::info!("Relay of {} closed: {}", addr, err);
                        } else {
                            log::info!("Relay of {} closed", addr);
                        }
                        USAGE.write().await.remove(&id);
                    } else {
                        if !admission_available()
                            || PENDING_RELAY_PAIRS.load(Ordering::SeqCst)
                                >= MAX_RELAY_SESSIONS.load(Ordering::SeqCst).max(1)
                        {
                            increment(&ADMISSION_REJECTIONS);
                            return;
                        }
                        log::info!("New relay request {} from {}", rf.uuid, addr);
                        let generation = PENDING_SEQUENCE
                            .fetch_add(1, Ordering::SeqCst)
                            .saturating_add(1);
                        let replaced = PEERS.lock().await.insert(
                            rf.uuid.clone(),
                            PendingPeer {
                                stream: Box::new(stream),
                                generation,
                            },
                        );
                        if replaced.is_none() {
                            PENDING_RELAY_PAIRS.fetch_add(1, Ordering::SeqCst);
                        }
                        sleep(30.).await;
                        let mut peers = PEERS.lock().await;
                        let still_current = peers
                            .get(&rf.uuid)
                            .map(|peer| peer.generation == generation)
                            .unwrap_or(false);
                        if still_current {
                            peers.remove(&rf.uuid);
                            atomic_saturating_sub(&PENDING_RELAY_PAIRS, 1);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

'''
    replace_between_once(
        relay,
        "async fn make_pair(\n",
        "async fn relay(\n",
        pairing,
    )


def patch_hbb_common(upstream: Path) -> None:
    common_root = upstream / "libs/hbb_common"
    manifest = common_root / "Cargo.toml"
    lib = common_root / "src/lib.rs"
    proxy = common_root / "src/proxy.rs"
    if not all(path.is_file() for path in (manifest, lib, proxy)):
        raise RuntimeError("hbb_common submodule is not initialized")

    content = manifest.read_text(encoding="utf-8")
    if 'dlopen = "0.1"\n' in content:
        replace_once(manifest, 'dlopen = "0.1"\n', "")
    content = manifest.read_text(encoding="utf-8")
    if 'machine-uid = "0.6"' not in content:
        replace_once(
            manifest,
            'machine-uid = { git = "https://github.com/rustdesk-org/machine-uid" }',
            'machine-uid = "0.6"',
        )
    content = manifest.read_text(encoding="utf-8")
    if 'rustls-platform-verifier = "0.7"' not in content:
        replace_once(
            manifest,
            'rustls-platform-verifier = "0.3.1"',
            'rustls-platform-verifier = "0.7"',
        )

    content = lib.read_text(encoding="utf-8")
    dlopen_export = '''#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use dlopen;
'''
    if dlopen_export in content:
        replace_once(lib, dlopen_export, "")

    content = proxy.read_text(encoding="utf-8")
    if "ClientConfig::with_platform_verifier()" not in content:
        replace_once(
            proxy,
            "        let verifier = rustls_platform_verifier::tls_config();\n",
            "        use rustls_platform_verifier::ConfigVerifierExt;\n"
            "        let verifier = tokio_rustls::rustls::ClientConfig::with_platform_verifier()\n"
            "            .map_err(|err| ProxyError::AddressResolutionFailed(err.to_string()))?;\n",
        )

    patch_relay_quality_proto(
        common_root / "protos/rendezvous.proto",
        upstream / "contracts/relay-quality/v1/rendezvous-extension.proto",
    )
    patch_fast_relay_proto(
        common_root / "protos/rendezvous.proto",
        upstream / "contracts/fast-relay/v1/rendezvous-extension.proto",
    )
    patch_profile_activation_proto(common_root / "protos/rendezvous.proto")


def patch_profile_activation_proto(proto: Path) -> None:
    # Canonicalize the unreleased WIP message names before applying the v1
    # contract. The wire tags are unchanged, so a repeatedly overlaid tree is
    # byte-for-byte equivalent to a clean application.
    content = proto.read_text(encoding="utf-8")
    content = content.replace("DeactivateProfileResponse", "DeactivatePeerResponse")
    content = content.replace("DeactivateProfile", "DeactivatePeer")
    content = content.replace("deactivate_profile_response", "deactivate_peer_response")
    content = content.replace("deactivate_profile", "deactivate_peer")
    proto.write_text(content, encoding="utf-8")

    fields = (
        ("RegisterPeer", "uint64 route_generation = 60;", 60),
        ("RegisterPeer", "uint64 activation_epoch = 61;", 61),
        ("RegisterPeer", "bytes route_lease = 62;", 62),
        ("RegisterPeer", "bytes activation_id = 63;", 63),
        ("RegisterPk", "uint64 activation_epoch = 61;", 61),
        ("RegisterPk", "bytes activation_id = 63;", 63),
        ("RegisterPkResponse", "uint64 route_generation = 60;", 60),
        ("RegisterPkResponse", "uint64 activation_epoch = 61;", 61),
        ("RegisterPkResponse", "bytes route_lease = 62;", 62),
        ("RegisterPkResponse", "bytes activation_id = 63;", 63),
    )
    for message, field, tag in fields:
        add_proto_field(proto, message, field, tag)

    for message, ordered_fields in (
        (
            "RegisterPeer",
            (
                "uint64 route_generation = 60;",
                "uint64 activation_epoch = 61;",
                "bytes route_lease = 62;",
                "bytes activation_id = 63;",
            ),
        ),
        (
            "RegisterPk",
            (
                "uint64 activation_epoch = 61;",
                "bytes activation_id = 63;",
            ),
        ),
        (
            "RegisterPkResponse",
            (
                "uint64 route_generation = 60;",
                "uint64 activation_epoch = 61;",
                "bytes route_lease = 62;",
                "bytes activation_id = 63;",
            ),
        ),
    ):
        content = proto.read_text(encoding="utf-8")
        start, end = proto_block_bounds(content, f"message {message}")
        block = content[start:end]
        for field in ordered_fields:
            block = block.replace(f"  {field}\n", "")
        block = block[:-1] + "".join(f"  {field}\n" for field in ordered_fields) + "}"
        proto.write_text(content[:start] + block + content[end:], encoding="utf-8")

    content = proto.read_text(encoding="utf-8")
    if "message DeactivatePeer {" not in content:
        definitions = '''message DeactivatePeer {
  string id = 1;
  bytes network_identity_uuid = 2;
  uint64 activation_epoch = 3;
  bytes activation_id = 4;
  bytes route_lease = 5;
  uint64 route_generation = 6;
}

message DeactivatePeerResponse {
  uint64 activation_epoch = 1;
  bytes activation_id = 2;
  bool deactivated = 3;
  uint64 route_generation = 4;
}

'''
        replace_once(
            proto,
            "message RendezvousMessage {\n",
            definitions + "message RendezvousMessage {\n",
        )
    content = proto.read_text(encoding="utf-8")
    start, end = proto_block_bounds(content, "message DeactivatePeer")
    block = content[start:end].replace(
        "bytes uuid = 2;", "bytes network_identity_uuid = 2;"
    )
    proto.write_text(content[:start] + block + content[end:], encoding="utf-8")
    add_proto_field(proto, "DeactivatePeer", "uint64 route_generation = 6;", 6)
    add_proto_field(
        proto,
        "DeactivatePeerResponse",
        "uint64 route_generation = 4;",
        4,
    )
    add_proto_oneof_arm(
        proto,
        "RendezvousMessage",
        "union",
        "DeactivatePeer deactivate_peer = 62;",
        62,
    )
    add_proto_oneof_arm(
        proto,
        "RendezvousMessage",
        "union",
        "DeactivatePeerResponse deactivate_peer_response = 63;",
        63,
    )


def patch_relay_quality_proto(proto: Path, contract: Path) -> None:
    contract_text = contract.read_text(encoding="utf-8")
    marker = "message StarryRelayLoad {"
    if marker not in contract_text:
        raise RuntimeError(f"Relay quality contract has no definitions: {contract}")
    definitions = contract_text[contract_text.index(marker) :].rstrip() + "\n\n"
    content = proto.read_text(encoding="utf-8")
    if "message RelayQualityOffer" not in content:
        replace_once(proto, "message RendezvousMessage {\n", definitions + "message RendezvousMessage {\n")
    content = proto.read_text(encoding="utf-8")
    if "message RelayQualityCancel" not in content:
        cancel_definition = contract_text[contract_text.index("message RelayQualityCancel {") :].rstrip() + "\n\n"
        replace_once(
            proto,
            "message RendezvousMessage {\n",
            cancel_definition + "message RendezvousMessage {\n",
        )

    # Incremental v1 additions must also be applied to trees that already
    # contain the original Relay Quality message definitions.
    add_proto_field(
        proto,
        "RelayProbeResponse",
        "uint32 relay_probe_protocol = 5;",
        5,
    )
    add_proto_field(
        proto,
        "RelayProbeResponse",
        "uint32 relay_load_protocol = 6;",
        6,
    )
    add_proto_field(
        proto,
        "RelayQualityOffer",
        "uint32 probe_timeout_ms = 9;",
        9,
    )
    for field, tag in (
        ("uint32 strategy = 10;", 10),
        ("uint32 stage = 11;", 11),
        ("bytes stage_token = 12;", 12),
        ("uint64 stage_deadline_unix_ms = 13;", 13),
        ("uint64 total_deadline_unix_ms = 14;", 14),
        ("string primary_relay = 15;", 15),
        ("uint32 p2p_probe_grace_ms = 16;", 16),
    ):
        add_proto_field(proto, "RelayQualityOffer", field, tag)
    for field, tag in (
        ("uint32 stage = 4;", 4),
        ("bytes stage_token = 5;", 5),
        ("uint32 endpoint_role = 6;", 6),
    ):
        add_proto_field(proto, "RelayProbeReport", field, tag)
    for field, tag in (
        ("uint32 reason_code = 7;", 7),
        ("uint32 stage = 8;", 8),
        ("bool partial = 9;", 9),
    ):
        add_proto_field(proto, "RelayQualityDecision", field, tag)

    fields = (
        ("PunchHoleRequest", "uint32 relay_quality_protocol = 100;", 100),
        ("PunchHole", "RelayQualityOffer relay_quality_offer = 100;", 100),
        ("FetchLocalAddr", "RelayQualityOffer relay_quality_offer = 100;", 100),
        ("PunchHoleSent", "RelayProbeReport relay_quality_report = 100;", 100),
        ("LocalAddr", "RelayProbeReport relay_quality_report = 100;", 100),
        ("PunchHoleResponse", "RelayQualityOffer relay_quality_offer = 100;", 100),
        ("PunchHoleResponse", "RelayProbeReport relay_quality_peer_report = 101;", 101),
        ("PunchHoleResponse", "RelayQualityDecision relay_quality_decision = 102;", 102),
        ("RequestRelay", "RelayProbeReport relay_quality_report = 100;", 100),
        ("RequestRelay", "RelayQualityDecision relay_quality_decision = 101;", 101),
        ("RequestRelay", "bytes relay_quality_allocation_id = 102;", 102),
        ("RelayResponse", "RelayProbeReport relay_quality_report = 100;", 100),
        ("RelayResponse", "RelayQualityDecision relay_quality_decision = 101;", 101),
    )
    for message, field, tag in fields:
        add_proto_field(proto, message, field, tag)
    add_proto_oneof_arm(
        proto,
        "RendezvousMessage",
        "union",
        "RelayProbeRequest relay_probe_request = 100;",
        100,
    )
    add_proto_oneof_arm(
        proto,
        "RendezvousMessage",
        "union",
        "RelayProbeResponse relay_probe_response = 101;",
        101,
    )
    add_proto_oneof_arm(
        proto,
        "RendezvousMessage",
        "union",
        "RelayProbeReport relay_quality_stage_report = 102;",
        102,
    )
    add_proto_oneof_arm(
        proto,
        "RendezvousMessage",
        "union",
        "RelayQualityOffer relay_quality_stage_offer = 103;",
        103,
    )
    add_proto_oneof_arm(
        proto,
        "RendezvousMessage",
        "union",
        "RelayQualityDecision relay_quality_stage_decision = 104;",
        104,
    )
    add_proto_oneof_arm(
        proto,
        "RendezvousMessage",
        "union",
        "RelayQualityCancel relay_quality_cancel = 105;",
        105,
    )


def patch_fast_relay_proto(proto: Path, contract: Path) -> None:
    contract_text = contract.read_text(encoding="utf-8")
    marker = "message FastRelayAuthorization {"
    if marker not in contract_text:
        raise RuntimeError(f"Fast Relay contract has no definitions: {contract}")
    definition = contract_text[contract_text.index(marker) :]
    definition = definition[: definition.index("// Official message bindings")].rstrip() + "\n\n"
    content = proto.read_text(encoding="utf-8")
    if marker not in content:
        replace_once(proto, "message RendezvousMessage {\n", definition + "message RendezvousMessage {\n")
    add_proto_field(
        proto,
        "RequestRelay",
        "bytes fast_relay_authorization = 64;",
        64,
    )
    add_proto_field(
        proto,
        "RelayResponse",
        "bytes fast_relay_authorization = 64;",
        64,
    )


def patch_modules(upstream: Path) -> None:
    lib = upstream / "src/lib.rs"
    lib_text = lib.read_text(encoding="utf-8")
    if "mod starry_config;" not in lib_text:
        replace_once(
            lib,
            "mod rendezvous_server;\n",
            "mod allocation_explain;\n"
            "mod connection_auth;\n"
            "pub mod control_agent;\n"
            "mod geo_relay;\n"
            "mod local_control;\n"
            "mod profile_activation;\n"
            "mod relay_observer;\n"
            "mod rendezvous_server;\n"
            "mod secure_tcp;\n"
            "mod starry_config;\n"
            "mod websocket_signal;\n",
        )
    elif "mod websocket_signal;" not in lib_text:
        replace_once(
            lib,
            "mod starry_config;\n",
            "mod starry_config;\nmod websocket_signal;\n",
        )
    lib_text = lib.read_text(encoding="utf-8")
    if "mod allocation_explain;" not in lib_text:
        replace_once(
            lib,
            "mod geo_relay;\n",
            "mod allocation_explain;\nmod geo_relay;\n",
        )
    lib_text = lib.read_text(encoding="utf-8")
    if "mod connection_auth;" not in lib_text:
        replace_once(
            lib,
            "mod allocation_explain;\n",
            "mod allocation_explain;\nmod connection_auth;\n",
        )
    lib_text = lib.read_text(encoding="utf-8")
    if "mod profile_activation;" not in lib_text:
        if "mod local_control;" in lib_text:
            replace_once(
                lib,
                "mod local_control;\n",
                "mod local_control;\nmod profile_activation;\n",
            )
        else:
            replace_once(
                lib,
                "mod geo_relay;\n",
                "mod geo_relay;\nmod profile_activation;\n",
            )
    lib_text = lib.read_text(encoding="utf-8")
    if "pub mod control_agent;" not in lib_text:
        replace_once(
            lib,
            "mod connection_auth;\n",
            "mod connection_auth;\npub mod control_agent;\n",
        )
    lib_text = lib.read_text(encoding="utf-8")
    if "pub mod config_downgrade;" not in lib_text:
        replace_once(
            lib,
            "pub mod control_agent;\n",
            "pub mod config_downgrade;\npub mod control_agent;\n",
        )
    lib_text = lib.read_text(encoding="utf-8")
    if "pub mod pairing;" not in lib_text:
        replace_once(
            lib,
            "pub mod control_agent;\n",
            "pub mod control_agent;\npub mod pairing;\n",
        )
    lib_text = lib.read_text(encoding="utf-8")
    if "mod local_control;" not in lib_text:
        replace_once(
            lib,
            "mod geo_relay;\n",
            "mod geo_relay;\nmod local_control;\nmod relay_observer;\n",
        )
    lib_text = lib.read_text(encoding="utf-8")
    if "mod relay_quality;" not in lib_text:
        replace_once(
            lib,
            "mod relay_observer;\n",
            "mod relay_observer;\nmod relay_quality;\n",
        )

    lib_text = lib.read_text(encoding="utf-8")
    if "mod fast_relay;" not in lib_text:
        replace_once(
            lib,
            "mod geo_relay;\n",
            "mod fast_relay;\nmod geo_relay;\n",
        )
    hbbr = upstream / "src/hbbr.rs"
    hbbr_text = hbbr.read_text(encoding="utf-8")
    if "mod fast_media_relay;" not in hbbr_text:
        replace_once(
            hbbr,
            "mod common;\n",
            "mod common;\nmod fast_media_relay;\n",
        )
    hbbr_text = hbbr.read_text(encoding="utf-8")
    if "mod relay_enrollment;" not in hbbr_text:
        replace_once(
            hbbr,
            "mod relay_server;\n",
            "mod relay_enrollment;\nmod relay_server;\n",
        )


def patch_cli(upstream: Path) -> None:
    main = upstream / "src/main.rs"
    main_text = main.read_text(encoding="utf-8")
    if "--starry-config" not in main_text:
        replace_once(
            main,
            "        -r, --relay-servers=[HOST] 'Sets the default relay servers, separated by comma'\n",
            "        -r, --relay-servers=[HOST] 'Sets the default relay servers, separated by comma'\n"
            "        , --starry-config=[FILE(default=starry/config.yaml)] 'Sets the external rustdesk-server-starry config file'\n",
        )
    main_text = main.read_text(encoding="utf-8")
    if "--must-login" not in main_text:
        replace_once(
            main,
            "        , --starry-config=[FILE(default=starry/config.yaml)] 'Sets the external rustdesk-server-starry config file'\n",
            "        , --starry-config=[FILE(default=starry/config.yaml)] 'Sets the external rustdesk-server-starry config file'\n"
            "        , --must-login 'Requires configured JWT authentication and enforces it as a startup floor'\n",
        )


def patch_rendezvous(upstream: Path) -> None:
    rendezvous = upstream / "src/rendezvous_server.rs"
    content = rendezvous.read_text(encoding="utf-8")
    full_starry_import = (
        "use crate::{allocation_explain, connection_auth, fast_relay, geo_relay, local_control, profile_activation, relay_observer, "
        "relay_quality, secure_tcp, starry_config, websocket_signal};"
    )
    if full_starry_import not in content:
        legacy_starry_imports = (
            "use crate::{allocation_explain, connection_auth, fast_relay, geo_relay, local_control, relay_observer, relay_quality, secure_tcp, starry_config, websocket_signal};\n",
            "use crate::{allocation_explain, connection_auth, geo_relay, local_control, relay_observer, relay_quality, secure_tcp, starry_config, websocket_signal};\n",
            "use crate::{allocation_explain, connection_auth, geo_relay, local_control, relay_observer, secure_tcp, starry_config, websocket_signal};\n",
            "use crate::{allocation_explain, geo_relay, local_control, relay_observer, secure_tcp, starry_config, websocket_signal};\n",
            "use crate::{allocation_explain, geo_relay, secure_tcp, starry_config, websocket_signal};\n",
            "use crate::{geo_relay, secure_tcp, starry_config, websocket_signal};\n",
            "use crate::{geo_relay, secure_tcp, starry_config};\n",
        )
        existing_import = next(
            (candidate for candidate in legacy_starry_imports if candidate in content),
            None,
        )
        if existing_import is not None:
            replace_once(rendezvous, existing_import, full_starry_import + "\n")
        else:
            replace_once(
                rendezvous,
                "use crate::common::*;\nuse crate::peer::*;\n",
                "use crate::common::*;\n"
                + full_starry_import
                + "\nuse crate::peer::*;\n",
            )

    content = rendezvous.read_text(encoding="utf-8")
    if "sync::{mpsc, oneshot, Mutex}" not in content:
        replace_once(
            rendezvous,
            "        sync::{mpsc, Mutex},\n",
            "        sync::{mpsc, oneshot, Mutex},\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "struct RelayApplyAck" not in content:
        replace_once(
            rendezvous,
            '''#[derive(Clone, Debug)]
enum Data {
    Msg(Box<RendezvousMessage>, SocketAddr),
    RelayServers0(String),
    RelayServers(RelayServers),
}
''',
            '''#[derive(Debug)]
enum Data {
    Msg(Box<RendezvousMessage>, SocketAddr),
    RelayServers0(String),
    RelayServersApply {
        relay_servers: String,
        generation: u64,
        ack: oneshot::Sender<Result<RelayApplyAck, String>>,
    },
    RelayServers(RelayServers),
}

#[derive(Debug)]
struct RelayApplyAck {
    previous_relay_servers: String,
    subsystem_ack: starry_config::SubsystemAck,
}
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    old_relay_data_arm = (
        "                        Data::RelayServers0(rs) => { self.parse_relay_servers(&rs); }\n"
        "                        Data::RelayServers(rs) => { self.relay_servers = Arc::new(rs); }\n"
    )
    new_relay_data_arm = '''                        Data::RelayServers0(rs) => {
                            self.parse_relay_servers(&rs);
                            relay_observer::update_configured(
                                self.relay_servers0.as_ref(),
                                starry_config::runtime_state().generation,
                            );
                        }
                        Data::RelayServersApply { relay_servers, generation, ack } => {
                            let previous_relay_servers = self.relay_servers0.join(",");
                            self.parse_relay_servers(&relay_servers);
                            relay_observer::update_configured(
                                self.relay_servers0.as_ref(),
                                generation,
                            );
                            let detail = format!(
                                "applied generation {generation} with {} configured Relay(s)",
                                self.relay_servers0.len()
                            );
                            let _ = ack.send(Ok(RelayApplyAck {
                                previous_relay_servers,
                                subsystem_ack: starry_config::SubsystemAck {
                                    subsystem: "relay_pool".to_owned(),
                                    accepted: true,
                                    detail,
                                },
                            }));
                        }
                        Data::RelayServers(rs) => {
                            relay_observer::update_native_online(&rs);
                            self.relay_servers = Arc::new(rs);
                        }
'''
    if "Data::RelayServersApply { relay_servers, generation, ack }" not in content:
        replace_once(rendezvous, old_relay_data_arm, new_relay_data_arm)

    content = rendezvous.read_text(encoding="utf-8")
    legacy_relay_servers0_arm = (
        "                        Data::RelayServers0(rs) => { self.parse_relay_servers(&rs); }\n"
    )
    if legacy_relay_servers0_arm in content:
        replace_once(
            rendezvous,
            legacy_relay_servers0_arm,
            '''                        Data::RelayServers0(rs) => {
                            self.parse_relay_servers(&rs);
                            relay_observer::update_configured(
                                self.relay_servers0.as_ref(),
                                starry_config::runtime_state().generation,
                            );
                        }
''',
        )
    content = rendezvous.read_text(encoding="utf-8")
    if "relay_observer::update_configured(\n                                self.relay_servers0.as_ref(),\n                                generation," not in content:
        replace_once(
            rendezvous,
            "                            self.parse_relay_servers(&relay_servers);\n",
            "                            self.parse_relay_servers(&relay_servers);\n"
            "                            relay_observer::update_configured(\n"
            "                                self.relay_servers0.as_ref(),\n"
            "                                generation,\n"
            "                            );\n",
        )
    content = rendezvous.read_text(encoding="utf-8")
    legacy_native_arm = (
        "                        Data::RelayServers(rs) => { self.relay_servers = Arc::new(rs); }\n"
    )
    if legacy_native_arm in content:
        replace_once(
            rendezvous,
            legacy_native_arm,
            '''                        Data::RelayServers(rs) => {
                            relay_observer::update_native_online(&rs);
                            self.relay_servers = Arc::new(rs);
                        }
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "stream::{SplitSink, StreamExt}," in content:
        replace_once(
            rendezvous,
            "        stream::{SplitSink, StreamExt},\n",
            "        stream::StreamExt,\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "        sink::SinkExt,\n" in content:
        replace_once(rendezvous, "        sink::SinkExt,\n", "")

    content = rendezvous.read_text(encoding="utf-8")
    if "TcpStream(secure_tcp::TcpWriteTransport)" not in content:
        replace_once(
            rendezvous,
            "const REG_TIMEOUT: i64 = 30_000;\n"
            "type TcpStreamSink = SplitSink<Framed<TcpStream, BytesCodec>, Bytes>;\n"
            "type WsSink = SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, tungstenite::Message>;\n"
            "enum Sink {\n"
            "    TcpStream(TcpStreamSink),\n",
            "const REG_TIMEOUT: i64 = 30_000;\n"
            "enum Sink {\n"
            "    TcpStream(secure_tcp::TcpWriteTransport),\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "Ws(websocket_signal::WsWriteTransport)" not in content:
        replace_once(
            rendezvous,
            "    Ws(WsSink),\n",
            "    Ws(websocket_signal::WsWriteTransport),\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    ws_sink_alias = (
        "type WsSink = SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, "
        "tungstenite::Message>;\n"
    )
    if ws_sink_alias in content:
        replace_once(rendezvous, ws_sink_alias, "")

    content = rendezvous.read_text(encoding="utf-8")
    if "Starry config startup:" not in content:
        replace_once(
            rendezvous,
            '        rs.parse_relay_servers(&get_arg("relay-servers"));\n',
            '        let upstream_relay_servers = get_arg("relay-servers");\n'
            '        let config_outcome = starry_config::initialize(&get_arg("starry-config"));\n'
            '        log::info!("Starry config startup: {}", config_outcome.message);\n'
            '        let relay_servers = config_outcome\n'
            '            .relay_servers\n'
            '            .as_deref()\n'
            '            .unwrap_or(&upstream_relay_servers);\n'
            '        rs.parse_relay_servers(relay_servers);\n'
            '        let relay_generation = config_outcome\n'
            '            .activation_ack\n'
            '            .as_ref()\n'
            '            .map(|ack| ack.generation)\n'
            '            .unwrap_or_default();\n'
            '        relay_observer::update_configured(rs.relay_servers0.as_ref(), relay_generation);\n'
            '        let geo_startup = geo_relay::reload();\n'
            '        log::info!("Geo relay startup: {}", geo_startup);\n'
            '        let mmdb_startup = geo_relay::start_mmdb_updater();\n'
            '        log::info!("{}", mmdb_startup);\n',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "relay_observer::update_configured(rs.relay_servers0.as_ref()" not in content:
        replace_once(
            rendezvous,
            '        rs.parse_relay_servers(relay_servers);\n',
            '        rs.parse_relay_servers(relay_servers);\n'
            '        let relay_generation = config_outcome\n'
            '            .activation_ack\n'
            '            .as_ref()\n'
            '            .map(|ack| ack.generation)\n'
            '            .unwrap_or_default();\n'
            '        relay_observer::update_configured(rs.relay_servers0.as_ref(), relay_generation);\n',
        )

    content = rendezvous.read_text(encoding="utf-8")
    startup_activation = '''        let upstream_relay_servers = get_arg("relay-servers");
        let config_outcome = starry_config::initialize(&get_arg("starry-config"));
        log::info!("Starry config startup: {}", config_outcome.message);
        if let Some(error) = config_outcome.error.as_deref() {
            bail!("Starry config startup rejected: {error}");
        }
        if config_outcome.accepted {
            let Some(config) = starry_config::snapshot() else {
                bail!("accepted Starry startup config is unavailable");
            };
            let Some(generation) = config_outcome
                .activation_ack
                .as_ref()
                .map(|ack| ack.generation)
            else {
                bail!("Starry startup activation acknowledgement is missing");
            };
            let prepared_geo = match geo_relay::prepare(&config) {
                Ok(prepared) => prepared,
                Err(err) => bail!("Geo startup preparation failed: {err}"),
            };
            let prepared_websocket = match websocket_signal::prepare(
                &config.websocket_signal,
                config.relay_quality.enabled,
            ) {
                Ok(prepared) => prepared,
                Err(err) => bail!("WebSocket startup preparation failed: {err}"),
            };
            let prepared_auth = match connection_auth::prepare(
                &config.connection_auth,
                connection_auth::must_login_floor(),
            )
            .await
            {
                Ok(prepared) => prepared,
                Err(err) => bail!("Connection authentication startup preparation failed: {err}"),
            };
            let relay_servers = config_outcome
                .relay_servers
                .as_deref()
                .unwrap_or(&upstream_relay_servers);
            rs.parse_relay_servers(relay_servers);
            relay_observer::update_configured(rs.relay_servers0.as_ref(), generation);
            let relay_ack = starry_config::SubsystemAck {
                subsystem: "relay_pool".to_owned(),
                accepted: true,
                detail: format!(
                    "applied generation {generation} with {} configured Relay(s)",
                    rs.relay_servers0.len()
                ),
            };
            let geo_ack = match geo_relay::activate_prepared(prepared_geo) {
                Ok(ack) => ack,
                Err(err) => bail!("Geo startup activation failed: {err}"),
            };
            let websocket_ack = match websocket_signal::activate_prepared(prepared_websocket) {
                Ok(ack) => ack,
                Err(err) => bail!("WebSocket startup activation failed: {err}"),
            };
            let secure_tcp_ack = starry_config::SubsystemAck {
                subsystem: "secure_tcp".to_owned(),
                accepted: true,
                detail: "configuration visible to new TCP connections".to_owned(),
            };
            let auth_ack = connection_auth::activate(prepared_auth);
            let ack = match starry_config::acknowledge_active(
                generation,
                vec![relay_ack, geo_ack, websocket_ack, secure_tcp_ack, auth_ack],
            ) {
                Ok(ack) => ack,
                Err(diagnostics) => bail!(
                    "Starry startup acknowledgement failed: {}",
                    diagnostics.summary()
                ),
            };
            log::info!(
                "Starry startup activation acknowledged: generation={}, digest={}",
                ack.generation,
                ack.effective_digest
            );
        } else {
            let disabled_auth = match connection_auth::prepare(
                &starry_config::ConnectionAuthConfig::default(),
                connection_auth::must_login_floor(),
            )
            .await
            {
                Ok(prepared) => prepared,
                Err(err) => bail!("Connection authentication startup preparation failed: {err}"),
            };
            connection_auth::activate(disabled_auth);
            rs.parse_relay_servers(&upstream_relay_servers);
            relay_observer::update_configured(rs.relay_servers0.as_ref(), 0);
            log::info!("Geo relay startup: {}", geo_relay::reload());
            log::info!(
                "WebSocket Signal startup: {}",
                websocket_signal::reconfigure()
            );
        }
        let mmdb_startup = geo_relay::start_mmdb_updater();
        log::info!("{}", mmdb_startup);
'''
    if "Starry startup activation acknowledged:" not in content:
        replace_between_once(
            rendezvous,
            '        let upstream_relay_servers = get_arg("relay-servers");\n',
            '        let mut listener = create_tcp_listener(port).await?;\n',
            startup_activation,
        )
    elif "let prepared_auth = match connection_auth::prepare(" not in content:
        replace_between_once(
            rendezvous,
            '        let upstream_relay_servers = get_arg("relay-servers");\n',
            '        let mut listener = create_tcp_listener(port).await?;\n',
            startup_activation,
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "WebSocket Signal startup:" not in content:
        replace_once(
            rendezvous,
            '        log::info!("{}", mmdb_startup);\n',
            '        log::info!("{}", mmdb_startup);\n'
            '        let websocket_startup = websocket_signal::reconfigure();\n'
            '        log::info!("WebSocket Signal startup: {}", websocket_startup);\n',
        )

    content = rendezvous.read_text(encoding="utf-8")
    unsafe_geo_startup = (
        '        log::info!("Geo relay startup: {}", geo_relay::reload());\n'
        '        log::info!("{}", geo_relay::start_mmdb_updater());\n'
    )
    if unsafe_geo_startup in content:
        replace_once(
            rendezvous,
            unsafe_geo_startup,
            '        let geo_startup = geo_relay::reload();\n'
            '        log::info!("Geo relay startup: {}", geo_startup);\n'
            '        let mmdb_startup = geo_relay::start_mmdb_updater();\n'
            '        log::info!("{}", mmdb_startup);\n',
        )

    content = rendezvous.read_text(encoding="utf-8")
    unsafe_websocket_startup = (
        '        log::info!(\n'
        '            "WebSocket Signal startup: {}",\n'
        '            websocket_signal::reconfigure()\n'
        '        );\n'
    )
    if unsafe_websocket_startup in content:
        replace_once(
            rendezvous,
            unsafe_websocket_startup,
            '        let websocket_startup = websocket_signal::reconfigure();\n'
            '        log::info!("WebSocket Signal startup: {}", websocket_startup);\n',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "async fn process_register_pk(" not in content:
        udp_register_start = (
            "                Some(rendezvous_message::Union::RegisterPk(rk)) => {\n"
        )

    content = rendezvous.read_text(encoding="utf-8")
    result_import = (
        "        register_pk_response::Result::{TOO_FREQUENT, UUID_MISMATCH},\n"
    )
    if result_import in content and "async fn process_register_pk(" in content:
        replace_once(rendezvous, result_import, "")

    content = rendezvous.read_text(encoding="utf-8")
    old_register_peer = '''                    if !rp.id.is_empty() {
                        log::trace!("New peer registered: {:?} {:?}", &rp.id, &addr);
                        self.update_addr(rp.id, addr, socket).await?;
                        if self.inner.serial > rp.serial {
'''
    new_register_peer = '''                    if !rp.id.is_empty() {
                        log::trace!("New peer registered from {:?}", &addr);
                        let peer_id = rp.id;
                        if self.update_addr(peer_id.clone(), addr, socket).await? {
                            websocket_signal::native_registration(&peer_id).await;
                        }
                        if self.inner.serial > rp.serial {
'''
    if (
        new_register_peer not in content
        and "profile_activation::begin_renewal(" not in content
    ):
        replace_once(rendezvous, old_register_peer, new_register_peer)

    content = rendezvous.read_text(encoding="utf-8")
    old_update_signature = '''    async fn update_addr(
        &mut self,
        id: String,
        socket_addr: SocketAddr,
        socket: &mut FramedSocket,
    ) -> ResultType<()> {
'''
    new_update_signature = '''    async fn update_addr(
        &mut self,
        id: String,
        socket_addr: SocketAddr,
        socket: &mut FramedSocket,
    ) -> ResultType<bool> {
'''
    routed_update_signature = '''    async fn update_addr(
        &mut self,
        id: String,
        socket_addr: SocketAddr,
        route_generation: u64,
        socket: &mut FramedSocket,
    ) -> ResultType<bool> {
'''
    if new_update_signature not in content and routed_update_signature not in content:
        replace_once(rendezvous, old_update_signature, new_update_signature)
        replace_once(
            rendezvous,
            '''        msg_out.set_register_peer_response(RegisterPeerResponse {
            request_pk,
            ..Default::default()
        });
        socket.send(&msg_out, socket_addr).await
    }
''',
            '''        msg_out.set_register_peer_response(RegisterPeerResponse {
            request_pk,
            ..Default::default()
        });
        socket.send(&msg_out, socket_addr).await?;
        Ok(!request_pk)
    }
''',
        )
        udp_register_end = (
            "                Some(rendezvous_message::Union::PunchHoleRequest(ph)) => {\n"
        )
        replace_between_once(
            rendezvous,
            udp_register_start,
            udp_register_end,
            '''                Some(rendezvous_message::Union::RegisterPk(rk)) => {
                    if let Some((result, id)) =
                        self.process_register_pk(rk, addr, Some(addr)).await
                    {
                        if result == register_pk_response::Result::OK {
                            websocket_signal::native_registration(&id).await;
                        }
                        send_rk_res(socket, addr, result).await?;
                    }
                }
''',
        )
        replace_once(
            rendezvous,
            '''    #[inline]
    async fn handle_tcp(
''',
            '''    async fn process_register_pk(
        &mut self,
        rk: RegisterPk,
        effective_addr: SocketAddr,
        native_addr: Option<SocketAddr>,
    ) -> Option<(register_pk_response::Result, String)> {
        if rk.uuid.is_empty() || rk.pk.is_empty() {
            return None;
        }
        let fast_identity_verified = self.is_verified_fast_registration(&rk).await;
        let id = rk.id;
        let ip = effective_addr.ip().to_string();
        if id.len() < 6 {
            return Some((register_pk_response::Result::UUID_MISMATCH, id));
        }
        if !self.check_ip_blocker(&ip, &id).await {
            return Some((register_pk_response::Result::TOO_FREQUENT, id));
        }
        let peer = self.pm.get_or(&id).await;
        let (changed, ip_changed, current_addr) = {
            let peer = peer.read().await;
            let current_addr = peer.socket_addr;
            if peer.uuid.is_empty() {
                (true, false, current_addr)
            } else {
                if peer.uuid == rk.uuid {
                    if peer.info.ip != ip && peer.pk != rk.pk {
                        log::warn!("Peer identity mismatch after simultaneous IP and key change");
                        return Some((register_pk_response::Result::UUID_MISMATCH, id));
                    }
                } else {
                    log::warn!("Peer UUID mismatch during registration");
                    return Some((register_pk_response::Result::UUID_MISMATCH, id));
                }
                let ip_changed = peer.info.ip != ip;
                (
                    peer.uuid != rk.uuid || peer.pk != rk.pk || ip_changed,
                    ip_changed,
                    current_addr,
                )
            }
        };
        let mut req_pk = peer.read().await.reg_pk;
        if req_pk.1.elapsed().as_secs() > 6 {
            req_pk.0 = 0;
        } else if req_pk.0 > 2 {
            return Some((register_pk_response::Result::TOO_FREQUENT, id));
        }
        req_pk.0 += 1;
        req_pk.1 = Instant::now();
        peer.write().await.reg_pk = req_pk;
        if ip_changed {
            let mut lock = IP_CHANGES.lock().await;
            if let Some((tm, ips)) = lock.get_mut(&id) {
                if tm.elapsed().as_secs() > IP_CHANGE_DUR {
                    *tm = Instant::now();
                    ips.clear();
                    ips.insert(ip.clone(), 1);
                } else if let Some(value) = ips.get_mut(&ip) {
                    *value += 1;
                } else {
                    ips.insert(ip.clone(), 1);
                }
            } else {
                lock.insert(
                    id.clone(),
                    (Instant::now(), HashMap::from([(ip.clone(), 1)])),
                );
            }
        }
        let result = if changed {
            self.pm
                .update_pk(
                    id.clone(),
                    peer,
                    native_addr.unwrap_or(current_addr),
                    rk.uuid,
                    rk.pk,
                    ip,
                )
                .await
        } else {
            let mut peer = peer.write().await;
            peer.last_reg_time = Instant::now();
            if let Some(addr) = native_addr {
                peer.socket_addr = addr;
            }
            register_pk_response::Result::OK
        };
        Some((result, id))
    }

    async fn touch_registered_peer(&self, peer_id: &str) {
        if let Some(peer) = self.pm.get_in_memory(peer_id).await {
            peer.write().await.last_reg_time = Instant::now();
        }
    }

    #[inline]
    async fn handle_tcp(
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    old_handle_signature = '''        sink: &mut Option<Sink>,
        addr: SocketAddr,
        key: &str,
        ws: bool,
'''
    new_handle_signature = '''        sink: &mut Option<Sink>,
        route_addr: SocketAddr,
        effective_addr: SocketAddr,
        key: &str,
        ws: bool,
'''
    if new_handle_signature not in content:
        replace_once(rendezvous, old_handle_signature, new_handle_signature)

    content = rendezvous.read_text(encoding="utf-8")
    handle_tcp_start = "    async fn handle_tcp(\n"
    handle_tcp_end = "    #[inline]\n    async fn update_addr(\n"
    if handle_tcp_start in content and handle_tcp_end in content:
        start_at = content.index(handle_tcp_start)
        end_at = content.index(handle_tcp_end, start_at)
        block = content[start_at:end_at]
        if "let addr = route_addr;" not in block:
            block = block.replace(
                "    ) -> bool {\n        if let Ok(msg_in)",
                "    ) -> bool {\n        let addr = route_addr;\n        if let Ok(msg_in)",
                1,
            )
            rendezvous.write_text(
                content[:start_at] + block + content[end_at:], encoding="utf-8"
            )

    content = rendezvous.read_text(encoding="utf-8")
    native_relay_method = '''    fn get_relay_server(&self, _pa: IpAddr, _pb: IpAddr) -> String {
        if self.relay_servers.is_empty() {
            return "".to_owned();
        } else if self.relay_servers.len() == 1 {
            return self.relay_servers[0].clone();
        }
        let i = ROTATION_RELAY_SERVER.fetch_add(1, Ordering::SeqCst) % self.relay_servers.len();
        self.relay_servers[i].clone()
    }
'''
    runtime_relay_helpers = '''    async fn apply_relay_servers_runtime(
        &self,
        relay_servers: String,
        generation: u64,
    ) -> Result<RelayApplyAck, String> {
        let (ack, response) = oneshot::channel();
        self.tx
            .send(Data::RelayServersApply {
                relay_servers,
                generation,
                ack,
            })
            .map_err(|_| "Relay pool apply channel is closed".to_owned())?;
        let response = timeout(5_000, response)
            .await
            .map_err(|_| "Relay pool apply acknowledgement timed out".to_owned())?
            .map_err(|_| "Relay pool apply acknowledgement channel is closed".to_owned())?;
        response
    }

    async fn restore_runtime_subsystems(
        &self,
        previous: Option<&starry_config::StarryConfig>,
        previous_relay_servers: String,
        generation: u64,
    ) -> String {
        let mut failures = Vec::new();
        if let Err(err) = self
            .apply_relay_servers_runtime(previous_relay_servers, generation)
            .await
        {
            failures.push(format!("Relay rollback failed: {err}"));
        }
        if let Some(config) = previous {
            match geo_relay::prepare(config).and_then(geo_relay::activate_prepared) {
                Ok(_) => {}
                Err(err) => failures.push(format!("Geo rollback failed: {err}")),
            }
            match websocket_signal::prepare(
                &config.websocket_signal,
                config.relay_quality.enabled,
            )
                .and_then(websocket_signal::activate_prepared)
            {
                Ok(_) => {}
                Err(err) => failures.push(format!("WebSocket rollback failed: {err}")),
            }
        } else {
            let geo = geo_relay::reload();
            if geo.contains("failed") || geo.contains("rejected") {
                failures.push(geo);
            }
            let websocket = websocket_signal::reconfigure();
            if websocket.contains("failed") || websocket.contains("rejected") {
                failures.push(websocket);
            }
        }
        let previous_auth = previous.map(|config| &config.connection_auth);
        let auth = connection_auth::restore(
            previous_auth,
            connection_auth::must_login_floor(),
        )
        .await;
        if auth.contains("failed") {
            failures.push(auth);
        }
        if failures.is_empty() {
            "runtime subsystems restored".to_owned()
        } else {
            failures.join("; ")
        }
    }

'''
    content = rendezvous.read_text(encoding="utf-8")
    if "async fn apply_relay_servers_runtime(" not in content:
        replace_once(
            rendezvous,
            "    fn parse_relay_servers(&mut self, relay_servers: &str) {\n",
            runtime_relay_helpers
            + "    fn parse_relay_servers(&mut self, relay_servers: &str) {\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    restore_start = "    async fn restore_runtime_subsystems(\n"
    restore_end = "    fn parse_relay_servers(&mut self, relay_servers: &str) {\n"
    if restore_start in content and restore_end in content:
        start_at = content.index(restore_start)
        end_at = content.index(restore_end, start_at)
        block = content[start_at:end_at]
        if "let previous_auth = previous.map(" not in block:
            block = block.replace(
                "        if failures.is_empty() {\n",
                "        let previous_auth = previous.map(|config| &config.connection_auth);\n"
                "        let auth = connection_auth::restore(\n"
                "            previous_auth,\n"
                "            connection_auth::must_login_floor(),\n"
                "        )\n"
                "        .await;\n"
                "        if auth.contains(\"failed\") {\n"
                "            failures.push(auth);\n"
                "        }\n"
                "        if failures.is_empty() {\n",
                1,
            )
            rendezvous.write_text(
                content[:start_at] + block + content[end_at:], encoding="utf-8"
            )

    content = rendezvous.read_text(encoding="utf-8")
    old_geo_method = '''    fn get_relay_server(&self, pa: IpAddr, pb: IpAddr) -> String {
        if self.relay_servers.is_empty() {
            return "".to_owned();
        }
        if let Some(relay) = geo_relay::select_relay(pa, pb, self.relay_servers.as_ref()) {
            return relay;
        }
        if self.relay_servers.len() == 1 {
            return self.relay_servers[0].clone();
        }
        let i = ROTATION_RELAY_SERVER.fetch_add(1, Ordering::SeqCst) % self.relay_servers.len();
        self.relay_servers[i].clone()
    }
'''
    legacy_transport_relay_method = '''    fn get_relay_server(
        &self,
        pa: IpAddr,
        pb: IpAddr,
        requirement: websocket_signal::RelayRequirement,
    ) -> String {
        let eligible = websocket_signal::eligible_relays(
            self.relay_servers0.as_ref(),
            self.relay_servers.as_ref(),
            requirement,
        );
        if eligible.is_empty() {
            if requirement != websocket_signal::RelayRequirement::NativeOnly {
                log::warn!("No eligible WebSocket Relay for requirement {:?}", requirement);
            }
            return "".to_owned();
        }
        if let Some(relay) = geo_relay::select_relay(pa, pb, &eligible, requirement) {
            return relay;
        }
        if eligible.len() == 1 {
            return eligible[0].clone();
        }
        let i = ROTATION_RELAY_SERVER.fetch_add(1, Ordering::SeqCst) % eligible.len();
        eligible[i].clone()
    }
'''
    transport_relay_method = '''    fn get_relay_server(
        &self,
        pa: IpAddr,
        pb: IpAddr,
        requirement: websocket_signal::RelayRequirement,
    ) -> String {
        let snapshot = relay_observer::snapshot();
        if !snapshot.is_consistent() {
            log::warn!("Relay selection paused while runtime generations converge");
            return "".to_owned();
        }
        let eligible = snapshot.eligible_relays(requirement);
        if eligible.is_empty() {
            if requirement != websocket_signal::RelayRequirement::NativeOnly {
                log::warn!("No eligible WebSocket Relay for requirement {:?}", requirement);
            }
            return "".to_owned();
        }
        if let Some(selection) = snapshot.select_geo(pa, pb, &eligible, requirement) {
            return selection.relay;
        }
        if eligible.len() == 1 {
            return eligible[0].clone();
        }
        let i = ROTATION_RELAY_SERVER.fetch_add(1, Ordering::SeqCst) % eligible.len();
        eligible[i].clone()
    }
'''
    if transport_relay_method not in content:
        if legacy_transport_relay_method in content:
            replace_once(rendezvous, legacy_transport_relay_method, transport_relay_method)
        elif old_geo_method in content:
            replace_once(rendezvous, old_geo_method, transport_relay_method)
        else:
            replace_once(rendezvous, native_relay_method, transport_relay_method)

    content = rendezvous.read_text(encoding="utf-8")
    legacy_explain_relay_method = '''
    fn explain_relay_server(
        &self,
        pa: IpAddr,
        pb: IpAddr,
        requirement: websocket_signal::RelayRequirement,
    ) -> allocation_explain::AllocationTrace {
        let configured = self.relay_servers0.as_ref();
        let eligible = websocket_signal::eligible_relays(
            configured,
            self.relay_servers.as_ref(),
            requirement,
        );
        let matched_rule = geo_relay::select_relay_explained(pa, pb, &eligible, requirement)
            .map(|selection| allocation_explain::MatchedRule {
                name: selection.rule_name,
                index: selection.rule_index,
                direction: selection.direction.to_owned(),
                relay_id: selection.relay,
            });
        let runtime = starry_config::runtime_state();
        allocation_explain::explain_relay_selection(
            configured,
            &eligible,
            matched_rule,
            ROTATION_RELAY_SERVER.load(Ordering::SeqCst),
            runtime.generation,
            websocket_signal::health_snapshot_id(),
        )
    }
'''
    explain_relay_method = '''
    fn explain_relay_server(
        &self,
        pa: IpAddr,
        pb: IpAddr,
        requirement: websocket_signal::RelayRequirement,
    ) -> Result<allocation_explain::AllocationTrace, String> {
        let snapshot = relay_observer::snapshot();
        if !snapshot.is_consistent() {
            return Err("Relay pool and active configuration generations are not synchronized".to_owned());
        }
        let configured = snapshot.configured_relays();
        let eligible = snapshot.eligible_relays(requirement);
        let matched_rule = snapshot
            .select_geo(pa, pb, &eligible, requirement)
            .map(|selection| allocation_explain::MatchedRule {
                name: selection.rule_name,
                index: selection.rule_index,
                direction: selection.direction.to_owned(),
                relay_id: selection.relay,
            });
        let exclusion_reasons = snapshot.exclusion_reasons(requirement);
        Ok(allocation_explain::explain_relay_selection(
            &configured,
            &eligible,
            matched_rule,
            ROTATION_RELAY_SERVER.load(Ordering::SeqCst),
            snapshot.config_generation,
            snapshot.health_snapshot_id,
            &exclusion_reasons,
        ))
    }
'''
    if "fn explain_relay_server(" not in content:
        replace_once(
            rendezvous,
            transport_relay_method,
            transport_relay_method + explain_relay_method,
        )
    elif legacy_explain_relay_method in content:
        replace_once(rendezvous, legacy_explain_relay_method, explain_relay_method)

    content = rendezvous.read_text(encoding="utf-8")
    reload_runtime_method = '''
    async fn reload_starry_runtime(&self) -> Result<starry_config::ActivationAck, String> {
        let candidate = starry_config::load_candidate()
            .map_err(|diagnostics| diagnostics.summary())?;
        let plan = starry_config::plan_activation(&candidate)
            .map_err(|diagnostics| diagnostics.summary())?;
        let previous = starry_config::snapshot();
        let prepared_geo = geo_relay::prepare(&candidate.config)?;
        let prepared_websocket = websocket_signal::prepare(
            &candidate.config.websocket_signal,
            candidate.config.relay_quality.enabled,
        )?;
        let prepared_auth = connection_auth::prepare(
            &candidate.config.connection_auth,
            connection_auth::must_login_floor(),
        )
        .await?;
        let relay_servers = if candidate.config.relay_servers.is_empty() {
            get_arg("relay-servers")
        } else {
            candidate.config.relay_servers.join(",")
        };
        let candidate_generation = plan.base_generation.saturating_add(1);
        let relay_apply = self
            .apply_relay_servers_runtime(relay_servers, candidate_generation)
            .await?;
        let previous_relay_servers = relay_apply.previous_relay_servers.clone();
        let relay_ack = relay_apply.subsystem_ack;

        let geo_ack = match geo_relay::activate_prepared(prepared_geo) {
            Ok(ack) => ack,
            Err(err) => {
                let recovery = self
                    .restore_runtime_subsystems(
                        previous.as_deref(),
                        previous_relay_servers,
                        plan.base_generation,
                    )
                    .await;
                return Err(format!(
                    "Geo apply acknowledgement failed: {err}; recovery={recovery}"
                ));
            }
        };
        let websocket_ack = match websocket_signal::activate_prepared(prepared_websocket) {
            Ok(ack) => ack,
            Err(err) => {
                let recovery = self
                    .restore_runtime_subsystems(
                        previous.as_deref(),
                        previous_relay_servers,
                        plan.base_generation,
                    )
                    .await;
                return Err(format!(
                    "WebSocket apply acknowledgement failed: {err}; recovery={recovery}"
                ));
            }
        };
        let secure_tcp_ack = starry_config::SubsystemAck {
            subsystem: "secure_tcp".to_owned(),
            accepted: true,
            detail: "configuration visible to new TCP connections".to_owned(),
        };
        let auth_ack = connection_auth::activate(prepared_auth);
        match starry_config::activate_if_base_generation(
            candidate,
            vec![relay_ack, geo_ack, websocket_ack, secure_tcp_ack, auth_ack],
            plan.base_generation,
        ) {
            Ok(ack) => Ok(ack),
            Err(diagnostics) => {
                let recovery = self
                    .restore_runtime_subsystems(
                        previous.as_deref(),
                        previous_relay_servers,
                        plan.base_generation,
                    )
                    .await;
                Err(format!(
                    "configuration activation failed: {}; recovery={recovery}",
                    diagnostics.summary()
                ))
            }
        }
    }
'''
    if "async fn reload_starry_runtime(" not in content:
        replace_once(
            rendezvous,
            "    async fn check_cmd(&self, cmd: &str) -> String {\n",
            reload_runtime_method + "\n    async fn check_cmd(&self, cmd: &str) -> String {\n",
        )
    elif "let prepared_auth = connection_auth::prepare(" not in content:
        replace_between_once(
            rendezvous,
            "    async fn reload_starry_runtime(&self) -> Result<starry_config::ActivationAck, String> {\n",
            "    async fn handle_local_control_request(\n",
            reload_runtime_method + "\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    local_control_dispatch = '''    async fn handle_local_control_request(
        &self,
        request: local_control::Request,
    ) -> local_control::Response {
        let request_id = request.request_id.clone();
        let empty_params = request
            .params
            .as_object()
            .map(|params| params.is_empty())
            .unwrap_or(false);
        match request.method.as_str() {
            "capabilities" if empty_params => local_control::Response::success(
                request_id,
                serde_json::json!({
                    "protocol": {"name": "starry-local-control", "version": 1},
                    "methods": [
                        "capabilities",
                        "status",
                        "relays",
                        "peer.verify",
                        "allocation.simulate",
                        "config.runtime_state",
                        "runtime.reload"
                    ],
                    "limits": {"max_frame_bytes": local_control::MAX_FRAME_BYTES}
                }),
            ),
            "status" if empty_params => {
                let config = starry_config::runtime_state();
                let relay = relay_observer::snapshot();
                let auth = connection_auth::status();
                let auth_ready = matches!(auth.verifier_state, "disabled" | "ready");
                local_control::Response::success(
                    request_id,
                    serde_json::json!({
                        "ready": relay.is_consistent() && auth_ready,
                        "config": config,
                        "auth": auth
                    }),
                )
            }
            "relays" if empty_params => {
                let snapshot = relay_observer::snapshot();
                if !snapshot.is_consistent() {
                    local_control::Response::error(
                        request_id,
                        "STARRY_NOT_READY",
                        "Relay pool and active configuration generations are not synchronized",
                        true,
                    )
                } else {
                    match serde_json::to_value(snapshot) {
                        Ok(value) => local_control::Response::success(request_id, value),
                        Err(_) => local_control::Response::error(
                            request_id,
                            "LOCAL_CONTROL_PROTOCOL_ERROR",
                            "cannot serialize Relay runtime snapshot",
                            false,
                        ),
                    }
                }
            }
            "peer.verify" => {
                let peer_id = request.params.get("id").and_then(serde_json::Value::as_str);
                let uuid = request.params.get("uuid").and_then(serde_json::Value::as_str);
                let activation_epoch = request
                    .params
                    .get("activation_epoch")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default();
                let activation_id = request
                    .params
                    .get("activation_id")
                    .and_then(serde_json::Value::as_str);
                let route_leases = request
                    .params
                    .get("route_leases")
                    .and_then(serde_json::Value::as_array);
                let registered = match (peer_id, uuid, activation_id, route_leases) {
                    (Some(peer_id), Some(uuid), Some(activation_id), Some(route_leases))
                        if !peer_id.is_empty()
                            && peer_id.len() <= 128
                            && activation_epoch > 0
                            && !route_leases.is_empty()
                            && route_leases.len() <= 16 =>
                    {
                        let uuid = base64::decode(uuid)
                            .ok()
                            .filter(|value| value.len() == 16)
                            .map(Bytes::from);
                        let activation_id = base64::decode(activation_id)
                            .ok()
                            .filter(|value| value.len() == 16)
                            .map(Bytes::from);
                        let route_leases = route_leases
                            .iter()
                            .map(|value| {
                                value
                                    .as_str()
                                    .and_then(|value| base64::decode(value).ok())
                                    .filter(|value| value.len() == 32)
                                    .map(Bytes::from)
                            })
                            .collect::<Option<Vec<_>>>();
                        match (uuid, activation_id, route_leases) {
                            (Some(uuid), Some(activation_id), Some(route_leases)) => {
                                profile_activation::verify_active(
                                    peer_id,
                                    &uuid,
                                    activation_epoch,
                                    &activation_id,
                                    &route_leases,
                                )
                                .await
                            }
                            _ => false,
                        }
                    }
                    _ => false,
                };
                local_control::Response::success(
                    request_id,
                    serde_json::json!({"registered": registered}),
                )
            }
            "allocation.simulate" => match relay_observer::simulate(
                request.params,
                ROTATION_RELAY_SERVER.load(Ordering::SeqCst),
            ) {
                Ok(trace) => match serde_json::to_value(trace) {
                    Ok(value) => local_control::Response::success(request_id, value),
                    Err(_) => local_control::Response::error(
                        request_id,
                        "LOCAL_CONTROL_PROTOCOL_ERROR",
                        "cannot serialize allocation trace",
                        false,
                    ),
                },
                Err(err) => local_control::Response::error(
                    request_id,
                    err.code,
                    err.detail,
                    err.retryable,
                ),
            },
            "config.runtime_state" if empty_params => {
                match serde_json::to_value(starry_config::runtime_state()) {
                    Ok(value) => local_control::Response::success(request_id, value),
                    Err(_) => local_control::Response::error(
                        request_id,
                        "LOCAL_CONTROL_PROTOCOL_ERROR",
                        "cannot serialize runtime configuration state",
                        false,
                    ),
                }
            }
            "runtime.reload" if empty_params => match self.reload_starry_runtime().await {
                Ok(ack) => match serde_json::to_value(ack) {
                    Ok(value) => local_control::Response::success(request_id, value),
                    Err(_) => local_control::Response::error(
                        request_id,
                        "LOCAL_CONTROL_PROTOCOL_ERROR",
                        "cannot serialize runtime activation acknowledgement",
                        false,
                    ),
                },
                Err(err) => local_control::Response::error(
                    request_id,
                    "CONFIG_INVALID",
                    err,
                    false,
                ),
            },
            "capabilities" | "status" | "relays" | "config.runtime_state"
            | "runtime.reload" => local_control::Response::error(
                request_id,
                "REQUEST_INVALID",
                "this local control method does not accept parameters",
                false,
            ),
            _ => local_control::Response::error(
                request_id,
                "REQUEST_INVALID",
                "unknown local control method",
                false,
            ),
        }
    }

'''
    if "async fn handle_local_control_request(" not in content:
        replace_once(
            rendezvous,
            "    async fn check_cmd(&self, cmd: &str) -> String {\n",
            local_control_dispatch + "    async fn check_cmd(&self, cmd: &str) -> String {\n",
        )
    elif '"peer.verify" =>' not in content or "let auth = connection_auth::status();" not in content:
        replace_between_once(
            rendezvous,
            "    async fn handle_local_control_request(\n",
            "    async fn check_cmd(&self, cmd: &str) -> String {\n",
            local_control_dispatch,
        )

    content = rendezvous.read_text(encoding="utf-8")
    if 'Some("reload-starry-config" | "rsc") =>' not in content:
        replace_once(
            rendezvous,
            '            Some("relay-servers" | "rs") => {\n',
            '            Some("reload-starry-config" | "rsc") => {\n'
            '                let outcome = starry_config::reload();\n'
            '                if outcome.accepted {\n'
            '                    let relay_servers = outcome\n'
            '                        .relay_servers\n'
            '                        .clone()\n'
            '                        .unwrap_or_else(|| get_arg("relay-servers"));\n'
            '                    self.tx.send(Data::RelayServers0(relay_servers)).ok();\n'
            '                }\n'
            '                res = format!("{}; {}", outcome.message, geo_relay::reload());\n'
            '            }\n'
            '            Some("reload-geo" | "rg") => {\n'
            '                res = geo_relay::reload();\n'
            '            }\n'
            '            Some("relay-servers" | "rs") => {\n',
        )

    content = rendezvous.read_text(encoding="utf-8")
    unsafe_reload_send = '''                let relay_servers = outcome
                    .relay_servers
                    .unwrap_or_else(|| get_arg("relay-servers"));
                self.tx.send(Data::RelayServers0(relay_servers)).ok();
'''
    guarded_reload_send = '''                if outcome.accepted {
                    let relay_servers = outcome
                        .relay_servers
                        .clone()
                        .unwrap_or_else(|| get_arg("relay-servers"));
                    self.tx.send(Data::RelayServers0(relay_servers)).ok();
                }
'''
    if unsafe_reload_send in content:
        replace_once(rendezvous, unsafe_reload_send, guarded_reload_send)

    content = rendezvous.read_text(encoding="utf-8")
    old_reload_result = '                res = format!("{}; {}", outcome.message, geo_relay::reload());\n'
    legacy_websocket_reload_result = '''                res = format!(
                    "{}; {}; {}",
                    outcome.message,
                    geo_relay::reload(),
                    websocket_signal::reconfigure()
                );
'''
    new_reload_result = '''                res = if outcome.accepted {
                    format!(
                        "{}; {}; {}",
                        outcome.message,
                        geo_relay::reload(),
                        websocket_signal::reconfigure()
                    )
                } else {
                    outcome.message
    };
'''
    if legacy_websocket_reload_result in content:
        replace_once(rendezvous, legacy_websocket_reload_result, new_reload_result)
    elif old_reload_result in content:
        replace_once(rendezvous, old_reload_result, new_reload_result)

    content = rendezvous.read_text(encoding="utf-8")
    transactional_reload = '''            Some("reload-starry-config" | "rsc") => {
                res = match self.reload_starry_runtime().await {
                    Ok(ack) => serde_json::to_string_pretty(&ack)
                        .unwrap_or_else(|_| "runtime activation acknowledgement serialization failed".to_owned()),
                    Err(err) => format!("runtime reload rejected: {err}"),
                };
            }
'''
    if "res = match self.reload_starry_runtime().await" not in content:
        replace_between_once(
            rendezvous,
            '            Some("reload-starry-config" | "rsc") => {\n',
            '            Some("reload-geo" | "rg") => {\n',
            transactional_reload,
        )

    content = rendezvous.read_text(encoding="utf-8")
    if '"reload-starry-config(rsc)",' not in content:
        replace_once(
            rendezvous,
            '                    "{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n",\n'
            '                    "relay-servers(rs) <separated by ,>",\n'
            '                    "reload-geo(rg)",\n',
            '                    "{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n",\n'
            '                    "relay-servers(rs) <separated by ,>",\n'
            '                    "reload-starry-config(rsc)",\n'
            '                    "reload-geo(rg)",\n',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if '"websocket-status(ws)",' not in content:
        replace_once(
            rendezvous,
            '                    "{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n",\n'
            '                    "relay-servers(rs) <separated by ,>",\n'
            '                    "reload-starry-config(rsc)",\n'
            '                    "reload-geo(rg)",\n',
            '                    "{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n{}\\n",\n'
            '                    "relay-servers(rs) <separated by ,>",\n'
            '                    "reload-starry-config(rsc)",\n'
            '                    "reload-geo(rg)",\n'
            '                    "websocket-status(ws)",\n',
        )
        replace_once(
            rendezvous,
            '                    "test-geo(tg) <ip1> <ip2>"\n',
            '                    "test-geo(tg) <ip1> <ip2> [native|wss|mixed]"\n',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if 'Some("websocket-status" | "ws") =>' not in content:
        replace_once(
            rendezvous,
            '            Some("relay-servers" | "rs") => {\n',
            '            Some("websocket-status" | "ws") => {\n'
            '                res = websocket_signal::status(self.relay_servers.as_ref()).await;\n'
            '            }\n'
            '            Some("relay-servers" | "rs") => {\n',
        )

    content = rendezvous.read_text(encoding="utf-8")
    old_test_geo = '''            Some("test-geo" | "tg") => {
                if let Some(rs) = fds.next() {
                    if let Ok(a) = rs.parse::<IpAddr>() {
                        if let Some(rs) = fds.next() {
                            if let Ok(b) = rs.parse::<IpAddr>() {
                                res = format!("{:?}", self.get_relay_server(a, b));
                            }
                        } else {
                            res = format!("{:?}", self.get_relay_server(a, a));
                        }
                    }
                }
            }
'''
    legacy_transport_test_geo = '''            Some("test-geo" | "tg") => {
                if let Some(first) = fds.next() {
                    if let Ok(a) = first.parse::<IpAddr>() {
                        let second = fds.next().and_then(|value| value.parse::<IpAddr>().ok());
                        let b = second.unwrap_or(a);
                        match websocket_signal::RelayRequirement::parse(fds.next()) {
                            Some(requirement) => {
                                res = format!("{:?}", self.get_relay_server(a, b, requirement));
                            }
                            None => res = "invalid transport; expected native, wss, or mixed".to_owned(),
                        }
                    }
                }
            }
'''
    legacy_explain_test_geo = '''            Some("test-geo" | "tg") => {
                if let Some(first) = fds.next() {
                    if let Ok(a) = first.parse::<IpAddr>() {
                        let second = fds.next().and_then(|value| value.parse::<IpAddr>().ok());
                        let b = second.unwrap_or(a);
                        match websocket_signal::RelayRequirement::parse(fds.next()) {
                            Some(requirement) => {
                                let trace = self.explain_relay_server(a, b, requirement);
                                res = serde_json::to_string_pretty(&trace)
                                    .unwrap_or_else(|_| "allocation trace serialization failed".to_owned());
                            }
                            None => res = "invalid transport; expected native, wss, or mixed".to_owned(),
                        }
                    }
                }
            }
'''
    new_test_geo = '''            Some("test-geo" | "tg") => {
                if let Some(first) = fds.next() {
                    if let Ok(a) = first.parse::<IpAddr>() {
                        let second = fds.next().and_then(|value| value.parse::<IpAddr>().ok());
                        let b = second.unwrap_or(a);
                        match websocket_signal::RelayRequirement::parse(fds.next()) {
                            Some(requirement) => {
                                res = match self.explain_relay_server(a, b, requirement) {
                                    Ok(trace) => serde_json::to_string_pretty(&trace)
                                        .unwrap_or_else(|_| "allocation trace serialization failed".to_owned()),
                                    Err(err) => format!("allocation simulation unavailable: {err}"),
                                };
                            }
                            None => res = "invalid transport; expected native, wss, or mixed".to_owned(),
                        }
                    }
                }
            }
'''
    if new_test_geo not in content:
        if legacy_explain_test_geo in content:
            replace_once(rendezvous, legacy_explain_test_geo, new_test_geo)
        elif legacy_transport_test_geo in content:
            replace_once(rendezvous, legacy_transport_test_geo, new_test_geo)
        else:
            replace_once(rendezvous, old_test_geo, new_test_geo)

    content = rendezvous.read_text(encoding="utf-8")
    if "secure_tcp::negotiate" not in content:
        replace_once(
            rendezvous,
            '''        } else {
            let (a, mut b) = Framed::new(stream, BytesCodec::new()).split();
            sink = Some(Sink::TcpStream(a));
            while let Ok(Some(Ok(bytes))) = timeout(30_000, b.next()).await {
                if !self.handle_tcp(&bytes, &mut sink, addr, key, ws).await {
                    break;
                }
            }
        }
''',
            '''        } else {
            let secure_config = starry_config::snapshot()
                .map(|config| config.secure_tcp.clone())
                .unwrap_or_default();
            let negotiated = secure_tcp::negotiate(
                stream,
                self.inner.sk.as_ref(),
                &secure_config,
            )
            .await?;
            let secure_tcp::NegotiatedTcp {
                sink: tcp_sink,
                stream: mut tcp_stream,
                first_plaintext,
                secured,
            } = negotiated;
            if secured {
                log::debug!("Secure TCP connection established for {addr}");
            }
            sink = Some(Sink::TcpStream(tcp_sink));
            let mut keep_reading = true;
            if let Some(bytes) = first_plaintext {
                keep_reading = self.handle_tcp(&bytes, &mut sink, addr, key, ws).await;
            }
            while keep_reading {
                match timeout(secure_config.idle_timeout_ms, tcp_stream.next()).await {
                    Ok(Some(Ok(bytes))) => {
                        keep_reading = self.handle_tcp(&bytes, &mut sink, addr, key, ws).await;
                    }
                    _ => break,
                }
            }
        }
            ''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "async fn handle_websocket_signal(" not in content:
        replace_once(
            rendezvous,
            '''    #[inline]
    async fn handle_listener_inner(
''',
            '''    async fn handle_websocket_signal(
        &mut self,
        stream: TcpStream,
        route_addr: SocketAddr,
        key: &str,
        config: starry_config::WebSocketSignalConfig,
    ) -> ResultType<()> {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

        let effective_slot = Arc::new(std::sync::Mutex::new(None));
        let callback_slot = effective_slot.clone();
        let callback_config = config.clone();
        let callback = move |request: &Request, response: Response| {
            match websocket_signal::inspect_upgrade(
                request.uri(),
                request.headers(),
                route_addr,
                &callback_config,
            ) {
                Ok(effective_addr) => {
                    if let Ok(mut slot) = callback_slot.lock() {
                        *slot = Some(effective_addr);
                    }
                    Ok(response)
                }
                Err(reason) => {
                    log::warn!("Rejected WebSocket Signal upgrade: {reason}");
                    Err(http::Response::builder()
                        .status(http::StatusCode::FORBIDDEN)
                        .body(Some("Forbidden".to_owned()))
                        .expect("static WebSocket rejection response"))
                }
            }
        };
        let parser_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .read_buffer_size(config.max_frame_bytes.clamp(4_096, 65_536))
            .max_message_size(Some(config.max_frame_bytes))
            .max_frame_size(Some(config.max_frame_bytes));
        let websocket = tokio_tungstenite::accept_hdr_async_with_config(
            stream,
            callback,
            Some(parser_config),
        )
        .await?;
        let effective_addr = effective_slot
            .lock()
            .ok()
            .and_then(|slot| *slot)
            .unwrap_or(route_addr);
        let (sink, stream) = websocket.split();
        let connection_id = websocket_signal::next_connection_id();
        let (writer, receiver) =
            websocket_signal::transport(connection_id, config.outbound_queue_capacity);
        let writer_task = writer.clone();
        tokio::spawn(async move {
            websocket_signal::writer_loop(sink, receiver, writer_task).await;
        });
        websocket_signal::register_connection(
            route_addr,
            effective_addr,
            connection_id,
            writer.clone(),
        )
        .await;

        let session = self
            .drive_websocket_signal(
                stream,
                writer.clone(),
                route_addr,
                effective_addr,
                key,
                &config,
            )
            .await;
        if let Some(session) = session.as_ref() {
            websocket_signal::remove_session(session, "reader closed").await;
        }
        self.tcp_punch.lock().await.remove(&try_into_v4(route_addr));
        websocket_signal::remove_connection(route_addr, connection_id).await;
        writer.close();
        Ok(())
    }

    async fn drive_websocket_signal(
        &mut self,
        mut stream: hbb_common::futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<TcpStream>,
        >,
        writer: websocket_signal::WsWriteTransport,
        route_addr: SocketAddr,
        effective_addr: SocketAddr,
        key: &str,
        config: &starry_config::WebSocketSignalConfig,
    ) -> Option<websocket_signal::SessionToken> {
        let registration_started = Instant::now();
        loop {
            let elapsed_ms = u64::try_from(registration_started.elapsed().as_millis())
                .unwrap_or(u64::MAX);
            let remaining_ms = config.registration_timeout_ms.saturating_sub(elapsed_ms);
            if remaining_ms == 0 {
                log::debug!("WebSocket Signal absolute registration deadline elapsed");
                return None;
            }
            let first = match timeout(remaining_ms, stream.next()).await {
                Ok(Some(Ok(message))) => message,
                Ok(Some(Err(err))) => {
                    log::debug!("WebSocket Signal read failed before registration: {err}");
                    return None;
                }
                Ok(None) => return None,
                Err(_) => {
                    log::debug!("WebSocket Signal registration timeout");
                    return None;
                }
            };
            let bytes = match first {
                tungstenite::Message::Binary(bytes) => bytes,
                tungstenite::Message::Ping(bytes) => {
                    if writer.send_pong(bytes).is_err() {
                        return None;
                    }
                    continue;
                }
                tungstenite::Message::Close(_) => return None,
                _ => {
                    log::debug!("WebSocket Signal requires binary protocol frames");
                    return None;
                }
            };
            if bytes.is_empty() {
                continue;
            }
            if bytes.len() > config.max_frame_bytes {
                log::warn!("Rejected oversized WebSocket Signal frame: {} bytes", bytes.len());
                return None;
            }
            let parsed = match RendezvousMessage::parse_from_bytes(&bytes) {
                Ok(message) => message,
                Err(err) => {
                    log::debug!("Rejected malformed WebSocket Signal protobuf: {err}");
                    return None;
                }
            };
            match parsed.union {
                Some(rendezvous_message::Union::RegisterPeer(_)) => {
                    let mut response = RendezvousMessage::new();
                    response.set_register_peer_response(RegisterPeerResponse {
                        request_pk: true,
                        ..Default::default()
                    });
                    if let Ok(bytes) = response.write_to_bytes() {
                        if writer.send_binary(bytes).is_err() {
                            return None;
                        }
                    }
                    continue;
                }
                Some(rendezvous_message::Union::RegisterPk(rk)) => {
                    if !websocket_signal::relay_ready() {
                        let mut response = RendezvousMessage::new();
                        response.set_register_pk_response(RegisterPkResponse {
                            result: register_pk_response::Result::SERVER_ERROR.into(),
                            ..Default::default()
                        });
                        if let Ok(bytes) = response.write_to_bytes() {
                            let _ = writer.send_binary(bytes);
                        }
                        log::warn!("WebSocket registration rejected: no healthy WSS Relay");
                        return None;
                    }
                    if !websocket_signal::allow_registration(
                        effective_addr.ip(),
                        config.registration_rate_per_minute,
                    )
                    .await
                    {
                        let mut response = RendezvousMessage::new();
                        response.set_register_pk_response(RegisterPkResponse {
                            result: register_pk_response::Result::TOO_FREQUENT.into(),
                            ..Default::default()
                        });
                        if let Ok(bytes) = response.write_to_bytes() {
                            let _ = writer.send_binary(bytes);
                        }
                        return None;
                    }
                    if !websocket_signal::capacity_available(
                        &rk.id,
                        effective_addr.ip(),
                        config,
                    )
                    .await
                    {
                        let mut response = RendezvousMessage::new();
                        response.set_register_pk_response(RegisterPkResponse {
                            result: register_pk_response::Result::TOO_FREQUENT.into(),
                            ..Default::default()
                        });
                        if let Ok(bytes) = response.write_to_bytes() {
                            let _ = writer.send_binary(bytes);
                        }
                        return None;
                    }
                    let Some((mut result, peer_id)) =
                        self.process_register_pk(rk, effective_addr, None).await
                    else {
                        return None;
                    };
                    let mut token = None;
                    if result == register_pk_response::Result::OK {
                        match websocket_signal::bind(
                            peer_id,
                            writer.clone(),
                            effective_addr.ip(),
                            route_addr,
                            config,
                        )
                        .await
                        {
                            Ok(bound) => token = Some(bound),
                            Err(err) => {
                                log::warn!("WebSocket registration capacity rejection: {err}");
                                result = register_pk_response::Result::TOO_FREQUENT;
                            }
                        }
                    }
                    let keep_alive = (config.idle_timeout_ms / 1_500)
                        .clamp(1, i32::MAX as u64) as i32;
                    let mut response = RendezvousMessage::new();
                    response.set_register_pk_response(RegisterPkResponse {
                        result: result.into(),
                        keep_alive,
                        ..Default::default()
                    });
                    let sent = response
                        .write_to_bytes()
                        .ok()
                        .and_then(|bytes| writer.send_binary(bytes).ok())
                        .is_some();
                    if result != register_pk_response::Result::OK || !sent {
                        if let Some(token) = token.as_ref() {
                            websocket_signal::remove_session(token, "registration response failed")
                                .await;
                        }
                        return None;
                    }
                    let token = token.expect("successful WebSocket registration has a route");
                    self.touch_registered_peer(&token.peer_id).await;
                    return self
                        .run_registered_websocket(
                            stream,
                            writer,
                            route_addr,
                            effective_addr,
                            key,
                            config,
                            token,
                        )
                        .await;
                }
                Some(_) => {
                    let mut sink = Some(Sink::Ws(writer.clone()));
                    let mut keep_reading = self.handle_tcp(
                        &bytes,
                        &mut sink,
                        route_addr,
                        effective_addr,
                        key,
                        true,
                    )
                    .await;
                    while keep_reading {
                        match timeout(config.idle_timeout_ms, stream.next()).await {
                            Ok(Some(Ok(tungstenite::Message::Binary(bytes)))) => {
                                if bytes.len() > config.max_frame_bytes {
                                    break;
                                }
                                if bytes.is_empty() {
                                    continue;
                                }
                                let mut sink = Some(Sink::Ws(writer.clone()));
                                keep_reading = self
                                    .handle_tcp(
                                        &bytes,
                                        &mut sink,
                                        route_addr,
                                        effective_addr,
                                        key,
                                        true,
                                    )
                                    .await;
                            }
                            Ok(Some(Ok(tungstenite::Message::Ping(bytes)))) => {
                                if writer.send_pong(bytes).is_err() {
                                    break;
                                }
                            }
                            Ok(Some(Ok(tungstenite::Message::Close(_))))
                            | Ok(None)
                            | Err(_) => break,
                            Ok(Some(Ok(_))) | Ok(Some(Err(_))) => break,
                        }
                    }
                    return None;
                }
                None => return None,
            }
        }
    }

    async fn run_registered_websocket(
        &mut self,
        mut stream: hbb_common::futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<TcpStream>,
        >,
        writer: websocket_signal::WsWriteTransport,
        route_addr: SocketAddr,
        effective_addr: SocketAddr,
        key: &str,
        config: &starry_config::WebSocketSignalConfig,
        token: websocket_signal::SessionToken,
    ) -> Option<websocket_signal::SessionToken> {
        let mut heartbeat = interval(Duration::from_millis(config.keepalive_interval_ms));
        heartbeat.tick().await;
        loop {
            tokio::select! {
                incoming = stream.next() => {
                    match incoming {
                        Some(Ok(tungstenite::Message::Binary(bytes))) => {
                            if bytes.len() > config.max_frame_bytes {
                                log::warn!("Closing WebSocket Signal session after oversized frame");
                                break;
                            }
                            token.touch();
                            self.touch_registered_peer(&token.peer_id).await;
                            if bytes.is_empty() {
                                continue;
                            }
                            let mut sink = Some(Sink::Ws(writer.clone()));
                            self.handle_tcp(
                                &bytes,
                                &mut sink,
                                route_addr,
                                effective_addr,
                                key,
                                true,
                            ).await;
                        }
                        Some(Ok(tungstenite::Message::Ping(bytes))) => {
                            if writer.send_pong(bytes).is_err() {
                                break;
                            }
                        }
                        Some(Ok(tungstenite::Message::Pong(_))) => token.touch(),
                        Some(Ok(tungstenite::Message::Close(_))) | None => break,
                        Some(Ok(_)) => break,
                        Some(Err(err)) => {
                            log::debug!("WebSocket Signal reader closed: {err}");
                            break;
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    if writer.is_closed() {
                        break;
                    }
                    if token.idle_for() >= Duration::from_millis(config.idle_timeout_ms) {
                        websocket_signal::remove_session(&token, "idle timeout").await;
                        break;
                    }
                    if writer.send_binary(Vec::new()).is_err() {
                        break;
                    }
                }
            }
        }
        Some(token)
    }

    #[inline]
    async fn handle_listener_inner(
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "return self.handle_websocket_signal(stream, addr, key, ws_config).await;" not in content:
        replace_once(
            rendezvous,
            '''        let mut sink;
        if ws {
            use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
''',
            '''        let mut sink;
        if ws {
            let ws_config = websocket_signal::config();
            if ws_config.enabled {
                return self.handle_websocket_signal(stream, addr, key, ws_config).await;
            }
            use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    legacy_raw_sink = '''            let ws_stream = tokio_tungstenite::accept_hdr_async(stream, callback).await?;
            let (a, mut b) = ws_stream.split();
            sink = Some(Sink::Ws(a));
'''
    legacy_actor_sink = '''            let ws_stream = tokio_tungstenite::accept_hdr_async(stream, callback).await?;
            let (a, mut b) = ws_stream.split();
            let connection_id = websocket_signal::next_connection_id();
            let (writer, receiver) = websocket_signal::transport(
                connection_id,
                ws_config.outbound_queue_capacity,
            );
            let writer_task = writer.clone();
            tokio::spawn(async move {
                websocket_signal::writer_loop(a, receiver, writer_task).await;
            });
            sink = Some(Sink::Ws(writer));
'''
    if legacy_raw_sink in content:
        replace_once(rendezvous, legacy_raw_sink, legacy_actor_sink)

    content = rendezvous.read_text(encoding="utf-8")
    old_tcp_call = "self.handle_tcp(&bytes, &mut sink, addr, key, ws)"
    new_tcp_call = "self.handle_tcp(&bytes, &mut sink, addr, addr, key, ws)"
    if old_tcp_call in content:
        count = content.count(old_tcp_call)
        if count != 3:
            raise RuntimeError(
                f"expected three upstream handle_tcp calls, found {count}"
            )
        rendezvous.write_text(
            content.replace(old_tcp_call, new_tcp_call), encoding="utf-8"
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "handle_tcp_punch_hole_request(route_addr, effective_addr" not in content:
        replace_once(
            rendezvous,
            "self.handle_tcp_punch_hole_request(addr, ph, key, ws).await",
            "self.handle_tcp_punch_hole_request(route_addr, effective_addr, ph, key, ws).await",
        )

    content = rendezvous.read_text(encoding="utf-8")
    old_request_relay = '''                    if let Some(peer) = self.pm.get_in_memory(&rf.id).await {
                        let mut msg_out = RendezvousMessage::new();
                        rf.socket_addr = AddrMangle::encode(addr).into();
                        msg_out.set_request_relay(rf);
                        let peer_addr = peer.read().await.socket_addr;
                        self.tx.send(Data::Msg(msg_out.into(), peer_addr)).ok();
                    }
                    return true;
'''
    new_request_relay = '''                    let peer_id = rf.id.clone();
                    let websocket_target = websocket_signal::route(&peer_id).await.is_some();
                    let mut msg_out = RendezvousMessage::new();
                    rf.socket_addr = AddrMangle::encode(route_addr).into();
                    msg_out.set_request_relay(rf);
                    if websocket_target {
                        if !websocket_signal::send_to_peer(&peer_id, &msg_out).await {
                            log::warn!("Failed to deliver RequestRelay to current WebSocket route");
                        }
                    } else if let Some(peer) = self.pm.get_in_memory(&peer_id).await {
                        let peer_addr = peer.read().await.socket_addr;
                        self.tx.send(Data::Msg(msg_out.into(), peer_addr)).ok();
                    }
                    return true;
'''
    if (
        new_request_relay not in content
        and "let quality_selection = if let Some(target_ip)" not in content
        and "let quality_resolution = if let Some((" not in content
    ):
        replace_once(rendezvous, old_request_relay, new_request_relay)

    content = rendezvous.read_text(encoding="utf-8")
    old_relay_selection = '''                        if self.is_lan(addr_b) {
                            // https://github.com/rustdesk/rustdesk-server/issues/24
                            rr.relay_server = self.inner.local_ip.clone();
                        } else if rr.relay_server == self.inner.local_ip {
                            rr.relay_server = self.get_relay_server(addr.ip(), addr_b.ip());
                        }
'''
    new_relay_selection = '''                        let controller_effective = websocket_signal::connection_effective(addr_b)
                            .await
                            .unwrap_or(addr_b);
                        let controller_ws = websocket_signal::is_websocket_route(addr_b).await;
                        let requirement = match (ws, controller_ws) {
                            (true, true) => websocket_signal::RelayRequirement::WebSocketOnly,
                            (true, false) | (false, true) => {
                                websocket_signal::RelayRequirement::Mixed
                            }
                            (false, false) => websocket_signal::RelayRequirement::NativeOnly,
                        };
                        if requirement == websocket_signal::RelayRequirement::NativeOnly
                            && self.is_lan(controller_effective)
                        {
                            // https://github.com/rustdesk/rustdesk-server/issues/24
                            rr.relay_server = self.inner.local_ip.clone();
                        } else if rr.relay_server == self.inner.local_ip {
                            rr.relay_server = self.get_relay_server(
                                effective_addr.ip(),
                                controller_effective.ip(),
                                requirement,
                            );
                        }
'''
    if new_relay_selection not in content:
        replace_once(rendezvous, old_relay_selection, new_relay_selection)

    content = rendezvous.read_text(encoding="utf-8")
    if "let _ = ws.send_binary(bytes);" not in content:
        replace_once(
            rendezvous,
            '''                    Sink::Ws(ws) => {
                        allow_err!(ws.send(tungstenite::Message::Binary(bytes)).await);
                    }
''',
            '''                    Sink::Ws(ws) => {
                        let _ = ws.send_binary(bytes);
                    }
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "let target_websocket = websocket_signal::route(&id).await;" not in content:
        replace_between_once(
            rendezvous,
            '''    #[inline]
    async fn handle_punch_hole_request(
''',
            '''    #[inline]
    async fn handle_online_request(
''',
            '''    #[inline]
    async fn handle_punch_hole_request(
        &mut self,
        route_addr: SocketAddr,
        effective_addr: SocketAddr,
        ph: PunchHoleRequest,
        key: &str,
        ws: bool,
    ) -> ResultType<(RendezvousMessage, Option<SocketAddr>)> {
        let mut ph = ph;
        if !key.is_empty() && ph.licence_key != key {
            log::warn!("Authentication failed for punch-hole request: invalid key");
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::LICENSE_MISMATCH.into(),
                ..Default::default()
            });
            return Ok((msg_out, None));
        }
        let id = ph.id;
        let target_websocket = websocket_signal::route(&id).await;
        if let Some(peer) = self.pm.get(&id).await {
            let (native_peer_addr, native_elapsed) = {
                let peer = peer.read().await;
                (
                    peer.socket_addr,
                    peer.last_reg_time.elapsed().as_millis() as i64,
                )
            };
            let (elapsed, peer_route_addr, peer_effective_addr, target_ws) =
                if let Some(route) = target_websocket {
                    (
                        route.idle_millis as i64,
                        route.route_addr,
                        SocketAddr::new(route.effective_ip, 0),
                        true,
                    )
                } else {
                    (native_elapsed, native_peer_addr, native_peer_addr, false)
                };
            if elapsed >= REG_TIMEOUT {
                let mut msg_out = RendezvousMessage::new();
                msg_out.set_punch_hole_response(PunchHoleResponse {
                    failure: punch_hole_response::Failure::OFFLINE.into(),
                    ..Default::default()
                });
                return Ok((msg_out, None));
            }

            {
                let from_ip = try_into_v4(effective_addr).ip().to_string();
                let to_ip = try_into_v4(peer_effective_addr).ip().to_string();
                let to_id_clone = id.clone();
                let mut lock = PUNCH_REQS.lock().await;
                let mut duplicate = false;
                for entry in lock.iter().rev().take(30) {
                    if entry.from_ip == from_ip && entry.to_id == to_id_clone {
                        if entry.tm.elapsed().as_secs() < PUNCH_REQ_DEDUPE_SEC {
                            duplicate = true;
                        }
                        break;
                    }
                }
                if !duplicate {
                    lock.push(PunchReqEntry {
                        tm: Instant::now(),
                        from_ip,
                        to_ip,
                        to_id: to_id_clone,
                    });
                }
            }

            let requirement = match (ws, target_ws) {
                (true, true) => websocket_signal::RelayRequirement::WebSocketOnly,
                (true, false) | (false, true) => websocket_signal::RelayRequirement::Mixed,
                (false, false) => websocket_signal::RelayRequirement::NativeOnly,
            };
            let mut msg_out = RendezvousMessage::new();
            let peer_is_lan = self.is_lan(peer_effective_addr);
            let is_lan = self.is_lan(effective_addr);
            let mut relay_server = self.get_relay_server(
                effective_addr.ip(),
                peer_effective_addr.ip(),
                requirement,
            );
            let force_websocket_relay =
                requirement != websocket_signal::RelayRequirement::NativeOnly;
            if force_websocket_relay && relay_server.is_empty() {
                msg_out.set_punch_hole_response(PunchHoleResponse {
                    failure: punch_hole_response::Failure::OFFLINE.into(),
                    other_failure: "no eligible WebSocket relay".to_owned(),
                    ..Default::default()
                });
                return Ok((msg_out, None));
            }
            if ALWAYS_USE_RELAY.load(Ordering::SeqCst)
                || force_websocket_relay
                || (peer_is_lan ^ is_lan)
            {
                if !force_websocket_relay && peer_is_lan {
                    // https://github.com/rustdesk/rustdesk-server/issues/24
                    relay_server = self.inner.local_ip.clone()
                }
                ph.nat_type = NatType::SYMMETRIC.into();
            }
            let same_intranet = !ws
                && !target_ws
                && (peer_is_lan && is_lan || {
                    match (peer_effective_addr, effective_addr) {
                        (SocketAddr::V4(a), SocketAddr::V4(b)) => a.ip() == b.ip(),
                        (SocketAddr::V6(a), SocketAddr::V6(b)) => a.ip() == b.ip(),
                        _ => false,
                    }
                });
            let socket_addr = AddrMangle::encode(route_addr).into();
            if same_intranet {
                log::debug!("Dispatching FetchLocalAddr over native signalling");
                msg_out.set_fetch_local_addr(FetchLocalAddr {
                    socket_addr,
                    relay_server,
                    ..Default::default()
                });
            } else {
                log::debug!("Dispatching PunchHole with Relay requirement {:?}", requirement);
                msg_out.set_punch_hole(PunchHole {
                    socket_addr,
                    nat_type: ph.nat_type,
                    relay_server,
                    ..Default::default()
                });
            }
            Ok((msg_out, Some(native_peer_addr)))
        } else {
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::ID_NOT_EXIST.into(),
                ..Default::default()
            });
            Ok((msg_out, None))
        }
    }

''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "let websocket_target = websocket_signal::route(&target_id).await.is_some();" not in content:
        replace_between_once(
            rendezvous,
            '''    #[inline]
    async fn handle_tcp_punch_hole_request(
''',
            '''    #[inline]
    async fn handle_udp_punch_hole_request(
''',
            '''    #[inline]
    async fn handle_tcp_punch_hole_request(
        &mut self,
        route_addr: SocketAddr,
        effective_addr: SocketAddr,
        ph: PunchHoleRequest,
        key: &str,
        ws: bool,
    ) -> ResultType<()> {
        let target_id = ph.id.clone();
        let websocket_target = websocket_signal::route(&target_id).await.is_some();
        let (msg, to_addr) = self
            .handle_punch_hole_request(route_addr, effective_addr, ph, key, ws)
            .await?;
        if let Some(addr) = to_addr {
            if websocket_target {
                if !websocket_signal::send_to_peer(&target_id, &msg).await {
                    let mut failure = RendezvousMessage::new();
                    failure.set_punch_hole_response(PunchHoleResponse {
                        failure: punch_hole_response::Failure::OFFLINE.into(),
                        other_failure: "WebSocket route closed during dispatch".to_owned(),
                        ..Default::default()
                    });
                    self.send_to_tcp_sync(failure, route_addr).await?;
                }
            } else {
                self.tx.send(Data::Msg(msg.into(), addr))?;
            }
        } else {
            self.send_to_tcp_sync(msg, route_addr).await?;
        }
        Ok(())
    }

''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    old_udp_punch = "self.handle_punch_hole_request(addr, ph, key, false).await?"
    if old_udp_punch in content:
        replace_once(
            rendezvous,
            old_udp_punch,
            "self.handle_punch_hole_request(addr, addr, ph, key, false).await?",
        )

    # UDP PunchHoleRequest is an explicit unsupported terminal branch. Remove
    # the now-unreachable allocation helper as a compile-time guard against a
    # future call site accidentally turning UDP into an authentication bypass.
    content = rendezvous.read_text(encoding="utf-8")
    udp_punch_helper = '''    #[inline]
    async fn handle_udp_punch_hole_request(
'''
    if udp_punch_helper in content:
        replace_between_once(
            rendezvous,
            udp_punch_helper,
            "    async fn check_ip_blocker",
            "",
        )

    content = rendezvous.read_text(encoding="utf-8")
    udp_key_parameter = '''    async fn handle_udp(
        &mut self,
        bytes: &BytesMut,
        addr: SocketAddr,
        socket: &mut FramedSocket,
        key: &str,
    ) -> ResultType<()> {
'''
    if udp_key_parameter in content:
        replace_once(
            rendezvous,
            udp_key_parameter,
            udp_key_parameter.replace("        key: &str,", "        _key: &str,"),
        )
    content = rendezvous.read_text(encoding="utf-8")
    udp_unsupported_arm = '''                Some(rendezvous_message::Union::PunchHoleRequest(ph)) => {
                    // UDP PunchHoleRequest is intentionally unsupported.
'''
    if udp_unsupported_arm in content:
        replace_once(
            rendezvous,
            udp_unsupported_arm,
            udp_unsupported_arm.replace("PunchHoleRequest(ph)", "PunchHoleRequest(_ph)"),
        )

    content = rendezvous.read_text(encoding="utf-8")
    legacy_loopback_control = '''    async fn handle_listener2(&self, stream: TcpStream, addr: SocketAddr) {
        let mut rs = self.clone();
        let ip = try_into_v4(addr).ip();
        if ip.is_loopback() {
            tokio::spawn(async move {
                let mut stream = stream;
                let mut buffer = [0; 1024];
                if let Ok(Ok(n)) = timeout(1000, stream.read(&mut buffer[..])).await {
                    if let Ok(data) = std::str::from_utf8(&buffer[..n]) {
                        let res = rs.check_cmd(data).await;
                        stream.write(res.as_bytes()).await.ok();
                    }
                }
            });
            return;
        }
'''
    structured_loopback_control = '''    async fn handle_listener2(&self, stream: TcpStream, addr: SocketAddr) {
        let mut rs = self.clone();
        let ip = try_into_v4(addr).ip();
        if ip.is_loopback() {
            tokio::spawn(async move {
                let mut stream = stream;
                match local_control::read_request(&mut stream).await {
                    Ok(local_control::IncomingRequest::Framed(request)) => {
                        let response = match local_control::authenticate(&request) {
                            Ok(()) => rs.handle_local_control_request(request).await,
                            Err(err) => local_control::Response::error(
                                request.request_id.clone(),
                                err.code,
                                err.detail,
                                false,
                            ),
                        };
                        if let Err(err) = local_control::write_response(&mut stream, &response).await {
                            log::debug!("Structured local control response failed: {}", err.detail);
                        }
                    }
                    Err(err) if err.framed => {
                        let response = local_control::Response::error(
                            String::new(),
                            err.code,
                            err.detail,
                            false,
                        );
                        let _ = local_control::write_response(&mut stream, &response).await;
                    }
                    Err(err) => {
                        log::debug!("Rejected unframed local control request: {}", err.detail);
                    }
                }
            });
            return;
        }
'''
    if "local_control::read_request(&mut stream)" not in content:
        replace_once(rendezvous, legacy_loopback_control, structured_loopback_control)

    content = rendezvous.read_text(encoding="utf-8")
    legacy_dispatch_arm = '''                    Ok(local_control::IncomingRequest::Legacy(command)) => {
                        let response = rs.check_cmd(&command).await;
                        if let Err(err) = local_control::write_legacy_response(
                            &mut stream,
                            response.as_bytes(),
                        )
                        .await
                        {
                            log::debug!("Legacy local control response failed: {}", err.detail);
                        }
                    }
'''
    if legacy_dispatch_arm in content:
        replace_once(rendezvous, legacy_dispatch_arm, "")
        content = rendezvous.read_text(encoding="utf-8")
    unauthenticated_framed_arm = '''                    Ok(local_control::IncomingRequest::Framed(request)) => {
                        let response = rs.handle_local_control_request(request).await;
                        if let Err(err) = local_control::write_response(&mut stream, &response).await {
                            log::debug!("Structured local control response failed: {}", err.detail);
                        }
                    }
'''
    authenticated_framed_arm = '''                    Ok(local_control::IncomingRequest::Framed(request)) => {
                        let response = match local_control::authenticate(&request) {
                            Ok(()) => rs.handle_local_control_request(request).await,
                            Err(err) => local_control::Response::error(
                                request.request_id.clone(),
                                err.code,
                                err.detail,
                                false,
                            ),
                        };
                        if let Err(err) = local_control::write_response(&mut stream, &response).await {
                            log::debug!("Structured local control response failed: {}", err.detail);
                        }
                    }
'''
    if unauthenticated_framed_arm in content:
        replace_once(
            rendezvous,
            unauthenticated_framed_arm,
            authenticated_framed_arm,
        )
        content = rendezvous.read_text(encoding="utf-8")
    content = content.replace(
        "Rejected legacy local control request",
        "Rejected unframed local control request",
    )
    rendezvous.write_text(content, encoding="utf-8")

    content = rendezvous.read_text(encoding="utf-8")
    if "local_control::read_request(&mut stream)" in content:
        async_io_import = "        io::{AsyncReadExt, AsyncWriteExt},\n"
        if async_io_import in content:
            replace_once(rendezvous, async_io_import, "")

    content = rendezvous.read_text(encoding="utf-8")
    if "secure_tcp::negotiate" in content:
        if "    bytes_codec::BytesCodec,\n" in content:
            replace_once(rendezvous, "    bytes_codec::BytesCodec,\n", "")
        content = rendezvous.read_text(encoding="utf-8")
        if "    tokio_util::codec::Framed,\n" in content:
            replace_once(rendezvous, "    tokio_util::codec::Framed,\n", "")

    # process_register_pk replaces every bare TOO_FREQUENT/UUID_MISMATCH use
    # with the qualified protobuf path. Perform this cleanup after all of the
    # registration rewrites so a clean checkout reaches the same result on
    # its first overlay application as on subsequent applications.
    content = rendezvous.read_text(encoding="utf-8")
    result_import = (
        "        register_pk_response::Result::{TOO_FREQUENT, UUID_MISMATCH},\n"
    )
    if result_import in content and "async fn process_register_pk(" in content:
        replace_once(rendezvous, result_import, "")

    content = rendezvous.read_text(encoding="utf-8")
    rotation_test = '''

#[cfg(test)]
mod starry_allocation_tests {
    use super::*;

    #[test]
    fn allocation_simulation_does_not_advance_production_rotation() {
        let original = ROTATION_RELAY_SERVER.swap(23, Ordering::SeqCst);
        let configured = vec!["relay-a".to_owned(), "relay-b".to_owned()];
        let exclusion_reasons = std::collections::HashMap::new();
        let trace = allocation_explain::explain_relay_selection(
            &configured,
            &configured,
            None,
            ROTATION_RELAY_SERVER.load(Ordering::SeqCst),
            7,
            "health-11".to_owned(),
            &exclusion_reasons,
        );
        assert_eq!(trace.selection.kind, "rotation_prediction");
        assert_eq!(ROTATION_RELAY_SERVER.load(Ordering::SeqCst), 23);
        ROTATION_RELAY_SERVER.store(original, Ordering::SeqCst);
    }
}
'''
    legacy_rotation_test_call = '''        let configured = vec!["relay-a".to_owned(), "relay-b".to_owned()];
        let trace = allocation_explain::explain_relay_selection(
            &configured,
            &configured,
            None,
            ROTATION_RELAY_SERVER.load(Ordering::SeqCst),
            7,
            11,
        );
'''
    updated_rotation_test_call = '''        let configured = vec!["relay-a".to_owned(), "relay-b".to_owned()];
        let exclusion_reasons = std::collections::HashMap::new();
        let trace = allocation_explain::explain_relay_selection(
            &configured,
            &configured,
            None,
            ROTATION_RELAY_SERVER.load(Ordering::SeqCst),
            7,
            "health-11".to_owned(),
            &exclusion_reasons,
        );
'''
    if legacy_rotation_test_call in content:
        replace_once(rendezvous, legacy_rotation_test_call, updated_rotation_test_call)
        content = rendezvous.read_text(encoding="utf-8")

    # Authentication is deliberately injected after all transport-routing
    # rewrites so both request kinds share one final, drift-checked entry point.
    content = rendezvous.read_text(encoding="utf-8")
    legacy_auth_signature = '''    async fn handle_tcp(
        &mut self,
        bytes: &[u8],
        sink: &mut Option<Sink>,
        route_addr: SocketAddr,
        effective_addr: SocketAddr,
        key: &str,
        ws: bool,
    ) -> bool {
'''
    authenticated_signature = '''    async fn handle_tcp(
        &mut self,
        bytes: &[u8],
        sink: &mut Option<Sink>,
        route_addr: SocketAddr,
        effective_addr: SocketAddr,
        key: &str,
        ws: bool,
        signal_transport: connection_auth::SignalTransport,
    ) -> bool {
'''
    if authenticated_signature not in content:
        replace_once(rendezvous, legacy_auth_signature, authenticated_signature)

    content = rendezvous.read_text(encoding="utf-8")
    old_authenticated_arms = '''                Some(rendezvous_message::Union::PunchHoleRequest(ph)) => {
                    // there maybe several attempt, so sink can be none
                    if let Some(sink) = sink.take() {
                        self.tcp_punch.lock().await.insert(try_into_v4(addr), sink);
                    }
                    allow_err!(self.handle_tcp_punch_hole_request(route_addr, effective_addr, ph, key, ws).await);
                    return true;
                }
                Some(rendezvous_message::Union::RequestRelay(mut rf)) => {
                    // there maybe several attempt, so sink can be none
                    if let Some(sink) = sink.take() {
                        self.tcp_punch.lock().await.insert(try_into_v4(addr), sink);
                    }
                    let peer_id = rf.id.clone();
                    let websocket_target = websocket_signal::route(&peer_id).await.is_some();
                    let mut msg_out = RendezvousMessage::new();
                    rf.socket_addr = AddrMangle::encode(route_addr).into();
                    msg_out.set_request_relay(rf);
                    if websocket_target {
                        if !websocket_signal::send_to_peer(&peer_id, &msg_out).await {
                            log::warn!("Failed to deliver RequestRelay to current WebSocket route");
                        }
                    } else if let Some(peer) = self.pm.get_in_memory(&peer_id).await {
                        let peer_addr = peer.read().await.socket_addr;
                        self.tx.send(Data::Msg(msg_out.into(), peer_addr)).ok();
                    }
                    return true;
                }
'''
    new_authenticated_arms = '''                Some(rendezvous_message::Union::PunchHoleRequest(ph)) => {
                    let decision = connection_auth::authorize_connection_attempt(
                        &ph.token,
                        connection_auth::ConnectionAttemptKind::PunchHole,
                        signal_transport,
                        effective_addr.ip(),
                    )
                    .await;
                    if decision.verdict == "would_deny" {
                        log::warn!(
                            "Connection authentication audit: kind=punch_hole transport={:?} reason={}",
                            signal_transport,
                            decision.reason
                        );
                    }
                    if !decision.proceed {
                        log::warn!(
                            "Connection authentication denied: kind=punch_hole transport={:?} reason={}",
                            signal_transport,
                            decision.reason
                        );
                        let mut denied = RendezvousMessage::new();
                        denied.set_punch_hole_response(PunchHoleResponse {
                            failure: punch_hole_response::Failure::OFFLINE.into(),
                            other_failure: "connection authorization failed".to_owned(),
                            ..Default::default()
                        });
                        Self::send_to_sink(sink, denied).await;
                        return false;
                    }
                    // There may be several attempts, so sink can be none.
                    if let Some(sink) = sink.take() {
                        self.tcp_punch.lock().await.insert(try_into_v4(addr), sink);
                    }
                    allow_err!(self.handle_tcp_punch_hole_request(route_addr, effective_addr, ph, key, ws).await);
                    return true;
                }
                Some(rendezvous_message::Union::RequestRelay(mut rf)) => {
                    let decision = connection_auth::authorize_connection_attempt(
                        &rf.token,
                        connection_auth::ConnectionAttemptKind::RequestRelay,
                        signal_transport,
                        effective_addr.ip(),
                    )
                    .await;
                    if decision.verdict == "would_deny" {
                        log::warn!(
                            "Connection authentication audit: kind=request_relay transport={:?} reason={}",
                            signal_transport,
                            decision.reason
                        );
                    }
                    if !decision.proceed {
                        log::warn!(
                            "Connection authentication denied: kind=request_relay transport={:?} reason={}",
                            signal_transport,
                            decision.reason
                        );
                        let mut denied = RendezvousMessage::new();
                        denied.set_relay_response(RelayResponse {
                            refuse_reason: "connection authorization failed".to_owned(),
                            ..Default::default()
                        });
                        Self::send_to_sink(sink, denied).await;
                        return false;
                    }
                    // There may be several attempts, so sink can be none.
                    if let Some(sink) = sink.take() {
                        self.tcp_punch.lock().await.insert(try_into_v4(addr), sink);
                    }
                    let peer_id = rf.id.clone();
                    let websocket_target = websocket_signal::route(&peer_id).await.is_some();
                    let mut msg_out = RendezvousMessage::new();
                    rf.socket_addr = AddrMangle::encode(route_addr).into();
                    msg_out.set_request_relay(rf);
                    if websocket_target {
                        if !websocket_signal::send_to_peer(&peer_id, &msg_out).await {
                            log::warn!("Failed to deliver RequestRelay to current WebSocket route");
                        }
                    } else if let Some(peer) = self.pm.get_in_memory(&peer_id).await {
                        let peer_addr = peer.read().await.socket_addr;
                        self.tx.send(Data::Msg(msg_out.into(), peer_addr)).ok();
                    }
                    return true;
                }
'''
    if "Connection authentication denied: kind=punch_hole" not in content:
        replace_once(rendezvous, old_authenticated_arms, new_authenticated_arms)

    content = rendezvous.read_text(encoding="utf-8")
    if "let signal_transport;" not in content:
        replace_once(
            rendezvous,
            "        let mut sink;\n        if ws {\n",
            "        let mut sink;\n"
            "        let signal_transport;\n"
            "        if ws {\n"
            "            signal_transport = connection_auth::SignalTransport::WebSocket;\n",
        )
        replace_once(
            rendezvous,
            "            } = negotiated;\n            if secured {\n",
            "            } = negotiated;\n"
            "            signal_transport = if secured {\n"
            "                connection_auth::SignalTransport::SecureTcp\n"
            "            } else {\n"
            "                connection_auth::SignalTransport::Tcp\n"
            "            };\n"
            "            if secured {\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "true,\n                        connection_auth::SignalTransport::WebSocket," not in content:
        websocket_call_pattern = re.compile(
            r"(?m)^(?P<indent>[ \t]+)key,\n(?P=indent)true,\n(?P<close>[ \t]+)\)\n"
        )
        content, count = websocket_call_pattern.subn(
            lambda match: (
                f"{match.group('indent')}key,\n"
                f"{match.group('indent')}true,\n"
                f"{match.group('indent')}connection_auth::SignalTransport::WebSocket,\n"
                f"{match.group('close')})\n"
            ),
            content,
        )
        if count != 2:
            raise RuntimeError(
                f"expected two unregistered WebSocket authorization call sites, found {count}"
            )
        rendezvous.write_text(content, encoding="utf-8")

    content = rendezvous.read_text(encoding="utf-8")
    compact_call = "self.handle_tcp(&bytes, &mut sink, addr, addr, key, ws).await"
    compact_call_with_transport = (
        "self.handle_tcp(&bytes, &mut sink, addr, addr, key, ws, signal_transport).await"
    )
    if compact_call_with_transport not in content:
        count = content.count(compact_call)
        if count != 3:
            raise RuntimeError(
                f"expected three listener authorization call sites, found {count}"
            )
        rendezvous.write_text(
            content.replace(compact_call, compact_call_with_transport), encoding="utf-8"
        )

    content = rendezvous.read_text(encoding="utf-8")
    legacy_registered_websocket_call = '''                            self.handle_tcp(
                                &bytes,
                                &mut sink,
                                route_addr,
                                effective_addr,
                                key,
                                true,
                            ).await;
'''
    registered_websocket_call = '''                            self.handle_tcp(
                                &bytes,
                                &mut sink,
                                route_addr,
                                effective_addr,
                                key,
                                true,
                                connection_auth::SignalTransport::WebSocket,
                            ).await;
'''
    closing_registered_websocket_call = '''                            if !self.handle_tcp(
                                &bytes,
                                &mut sink,
                                route_addr,
                                effective_addr,
                                key,
                                true,
                                connection_auth::SignalTransport::WebSocket,
                            ).await {
                                break;
                            }
'''
    if legacy_registered_websocket_call in content:
        replace_once(
            rendezvous,
            legacy_registered_websocket_call,
            registered_websocket_call,
        )
        content = rendezvous.read_text(encoding="utf-8")
    if closing_registered_websocket_call in content:
        replace_once(
            rendezvous,
            closing_registered_websocket_call,
            registered_websocket_call,
        )
        content = rendezvous.read_text(encoding="utf-8")

    registered_websocket_parse_anchor = '''                            if bytes.is_empty() {
                                continue;
                            }
                            let mut sink = Some(Sink::Ws(writer.clone()));
'''
    registered_websocket_parse_guard = '''                            if bytes.is_empty() {
                                continue;
                            }
                            if RendezvousMessage::parse_from_bytes(&bytes).is_err() {
                                log::debug!("Closing registered WebSocket Signal session after malformed protobuf");
                                break;
                            }
                            let mut sink = Some(Sink::Ws(writer.clone()));
'''
    if (
        registered_websocket_parse_guard not in content
        and "let parsed = match RendezvousMessage::parse_from_bytes(&bytes)" not in content
    ):
        replace_once(
            rendezvous,
            registered_websocket_parse_anchor,
            registered_websocket_parse_guard,
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "allocation_simulation_does_not_advance_production_rotation" not in content:
        rendezvous.write_text(content + rotation_test, encoding="utf-8")

    content = rendezvous.read_text(encoding="utf-8")
    legacy_pong = "writer.send_pong(bytes)"
    current_pong = "writer.send_pong(bytes.to_vec())"
    legacy_pong_count = content.count(legacy_pong)
    current_pong_count = content.count(current_pong)
    if legacy_pong_count == 3 and current_pong_count == 0:
        rendezvous.write_text(
            content.replace(legacy_pong, current_pong),
            encoding="utf-8",
        )
    elif legacy_pong_count != 0 or current_pong_count != 3:
        raise RuntimeError(
            "unexpected WebSocket Pong conversion anchors in rendezvous_server.rs: "
            f"legacy={legacy_pong_count}, current={current_pong_count}"
        )

    content = rendezvous.read_text(encoding="utf-8")
    legacy_connection_registration = (
        "        websocket_signal::register_connection(route_addr, effective_addr, connection_id).await;\n"
    )
    bounded_connection_registration = '''        websocket_signal::register_connection(
            route_addr,
            effective_addr,
            connection_id,
            writer.clone(),
        )
        .await;
'''
    if legacy_connection_registration in content:
        replace_once(
            rendezvous,
            legacy_connection_registration,
            bounded_connection_registration,
        )

    content = rendezvous.read_text(encoding="utf-8")
    unbounded_signal_accept = (
        "        let websocket = tokio_tungstenite::accept_hdr_async(stream, callback).await?;\n"
    )
    bounded_signal_accept = '''        let parser_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .read_buffer_size(config.max_frame_bytes.clamp(4_096, 65_536))
            .max_message_size(Some(config.max_frame_bytes))
            .max_frame_size(Some(config.max_frame_bytes));
        let websocket = tokio_tungstenite::accept_hdr_async_with_config(
            stream,
            callback,
            Some(parser_config),
        )
        .await?;
'''
    if unbounded_signal_accept in content:
        replace_once(rendezvous, unbounded_signal_accept, bounded_signal_accept)

    content = rendezvous.read_text(encoding="utf-8")
    sliding_registration_timeout = '''    ) -> Option<websocket_signal::SessionToken> {
        loop {
            let first = match timeout(config.registration_timeout_ms, stream.next()).await {
'''
    absolute_registration_timeout = '''    ) -> Option<websocket_signal::SessionToken> {
        let registration_started = Instant::now();
        loop {
            let elapsed_ms = u64::try_from(registration_started.elapsed().as_millis())
                .unwrap_or(u64::MAX);
            let remaining_ms = config.registration_timeout_ms.saturating_sub(elapsed_ms);
            if remaining_ms == 0 {
                log::debug!("WebSocket Signal absolute registration deadline elapsed");
                return None;
            }
            let first = match timeout(remaining_ms, stream.next()).await {
'''
    if sliding_registration_timeout in content:
        replace_once(
            rendezvous,
            sliding_registration_timeout,
            absolute_registration_timeout,
        )

    # A disabled Starry WebSocket listener must close its admission path.  It
    # must not fall through to the upstream legacy handler with weaker origin,
    # proxy, lifetime, and frame controls.
    content = rendezvous.read_text(encoding="utf-8")
    disabled_fallback_start = '''        if ws {
            signal_transport = connection_auth::SignalTransport::WebSocket;
            let ws_config = websocket_signal::config();
            if ws_config.enabled {
                return self.handle_websocket_signal(stream, addr, key, ws_config).await;
            }
'''
    disabled_fallback_end = '''        } else {
            let secure_config = starry_config::snapshot()
'''
    disabled_closed = '''        if ws {
            let ws_config = websocket_signal::config();
            if ws_config.enabled {
                return self.handle_websocket_signal(stream, addr, key, ws_config).await;
            }
            log::debug!("WebSocket Signal listener is disabled; closing admission");
            return Ok(());
'''
    if disabled_fallback_start in content:
        replace_between_once(
            rendezvous,
            disabled_fallback_start,
            disabled_fallback_end,
            disabled_closed,
        )

    content = rendezvous.read_text(encoding="utf-8")
    content = content.replace(
        "        mut addr: SocketAddr,\n        key: &str,\n        ws: bool,\n",
        "        addr: SocketAddr,\n        key: &str,\n        ws: bool,\n",
    )
    content = content.replace(
        "        }\n                    if sink.is_none() {\n",
        "        }\n        if sink.is_none() {\n",
    )
    rendezvous.write_text(content, encoding="utf-8")

    # Older WIP overlay revisions inserted the reload helper next to a block
    # that already carried extra vertical whitespace.  Canonicalize this one
    # generated boundary so a checkout upgraded through those revisions is
    # byte-for-byte identical to a clean first application.
    content = rendezvous.read_text(encoding="utf-8")
    content = re.sub(
        r"    }\n{3,}    async fn reload_starry_runtime\(",
        "    }\n\n    async fn reload_starry_runtime(",
        content,
    )
    rendezvous.write_text(content, encoding="utf-8")


def patch_profile_activation_hbbs(upstream: Path) -> None:
    rendezvous = upstream / "src/rendezvous_server.rs"
    peer_source = upstream / "src/peer.rs"
    peer_content = peer_source.read_text(encoding="utf-8")
    if "pub(crate) route_generation: u64," not in peer_content:
        replace_once(
            peer_source,
            "    pub(crate) last_reg_time: Instant,\n",
            "    pub(crate) last_reg_time: Instant,\n"
            "    pub(crate) route_generation: u64,\n",
        )
        replace_once(
            peer_source,
            "            last_reg_time: get_expired_time(),\n",
            "            last_reg_time: get_expired_time(),\n"
            "            route_generation: 0,\n",
        )
    content = rendezvous.read_text(encoding="utf-8")

    if "struct RegisterPkOutcome {" not in content:
        replace_once(
            rendezvous,
            "#[derive(Clone)]\npub struct RendezvousServer {\n",
            '''struct RegisterPkOutcome {
    result: register_pk_response::Result,
    id: String,
    activation: Option<profile_activation::ActivationClaim>,
}

#[derive(Clone)]
pub struct RendezvousServer {
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    udp_start = content.index("    async fn handle_udp(")
    udp_end = content.index("    async fn process_register_pk(", udp_start)
    udp = content[udp_start:udp_end]
    register_peer_start = (
        "                Some(rendezvous_message::Union::RegisterPeer(rp)) => {\n"
    )
    register_pk_start = (
        "                Some(rendezvous_message::Union::RegisterPk(rk)) => {\n"
    )
    start_at = udp.index(register_peer_start)
    end_at = udp.index(register_pk_start, start_at)
    if (
        "rp.route_generation," not in udp[start_at:end_at]
        or "claim.reject_route_change();" not in udp[start_at:end_at]
    ):
        udp = udp[:start_at] + '''                Some(rendezvous_message::Union::RegisterPeer(rp)) => {
                    if !rp.id.is_empty() {
                        log::trace!("New peer registered from {:?}", &addr);
                        let peer_id = rp.id;
                        let serial = rp.serial;
                        match profile_activation::begin_renewal(
                            &peer_id,
                            rp.activation_epoch,
                            &rp.activation_id,
                            &rp.route_lease,
                            rp.route_generation,
                        )
                        .await
                        {
                            Ok(claim) => {
                                let generation = claim.generation();
                                let previous_route = claim.previous_route();
                                let tracked = claim.tracked();
                                let route = profile_activation::RouteKind::Native(addr);
                                if tracked && previous_route != Some(route) {
                                    claim.reject_route_change();
                                    let mut response = RendezvousMessage::new();
                                    response.set_register_peer_response(RegisterPeerResponse {
                                        request_pk: true,
                                        ..Default::default()
                                    });
                                    socket.send(&response, addr).await?;
                                } else if self
                                    .update_addr(peer_id.clone(), addr, generation, socket)
                                    .await?
                                {
                                    claim.commit(route).await;
                                }
                            }
                            Err(_) => {
                                let mut response = RendezvousMessage::new();
                                response.set_register_peer_response(RegisterPeerResponse {
                                    request_pk: true,
                                    ..Default::default()
                                });
                                socket.send(&response, addr).await?;
                            }
                        }
                        if self.inner.serial > serial {
                            let mut msg_out = RendezvousMessage::new();
                            msg_out.set_configure_update(ConfigUpdate {
                                serial: self.inner.serial,
                                rendezvous_servers: (*self.rendezvous_servers).clone(),
                                ..Default::default()
                            });
                            socket.send(&msg_out, addr).await?;
                        }
                    }
                }
''' + udp[end_at:]
    start_at = udp.index(register_pk_start)
    end_marker = (
        "                Some(rendezvous_message::Union::PunchHoleRequest("
    )
    end_at = udp.index(end_marker, start_at)
    if (
        "profile_activation::record_ready_ack();" not in udp[start_at:end_at]
        or "connection_id: old_connection_id" not in udp[start_at:end_at]
    ):
        udp = udp[:start_at] + '''                Some(rendezvous_message::Union::RegisterPk(rk)) => {
                    if let Some(mut outcome) =
                        self.process_register_pk(rk, addr, Some(addr)).await
                    {
                        let mut activation_epoch = 0;
                        let mut activation_id = Bytes::new();
                        let mut route_lease = Bytes::new();
                        let mut route_generation = 0;
                        if outcome.result == register_pk_response::Result::OK {
                            if let Some(claim) = outcome.activation.take() {
                                let generation = claim.generation();
                                let previous_route = claim.previous_route();
                                let tracked = claim.tracked();
                                activation_epoch = claim.activation_epoch();
                                activation_id = claim.activation_id().clone();
                                route_lease = claim.route_lease().clone();
                                if tracked {
                                    route_generation = generation;
                                }
                                self.activate_native_route(&outcome.id, addr, generation)
                                    .await;
                                claim
                                    .commit(profile_activation::RouteKind::Native(addr))
                                    .await;
                                if tracked {
                                    if let Some(profile_activation::RouteKind::WebSocket {
                                        generation: old_generation,
                                        connection_id: old_connection_id,
                                    }) = previous_route
                                    {
                                        websocket_signal::remove_profile_route(
                                            &outcome.id,
                                            old_generation,
                                            old_connection_id,
                                        )
                                        .await;
                                    }
                                }
                            }
                        }
                        send_rk_res(
                            socket,
                            addr,
                            outcome.result,
                            activation_epoch,
                            activation_id,
                            route_lease,
                            route_generation,
                        )
                        .await?;
                        if route_generation != 0 {
                            profile_activation::record_ready_ack();
                        }
                    }
                }
''' + udp[end_at:]
    if "Union::DeactivatePeer(request)" not in udp:
        marker = "                Some(rendezvous_message::Union::PunchHoleRequest("
        insert_at = udp.index(marker)
        udp = udp[:insert_at] + '''                Some(rendezvous_message::Union::DeactivatePeer(request)) => {
                    let activation_epoch = request.activation_epoch;
                    let activation_id = request.activation_id.clone();
                    let route_generation = request.route_generation;
                    let deactivated = self.process_peer_deactivation(request).await;
                    let mut response = RendezvousMessage::new();
                    response.set_deactivate_peer_response(DeactivatePeerResponse {
                        activation_epoch,
                        activation_id,
                        deactivated,
                        route_generation,
                        ..Default::default()
                    });
                    socket.send(&response, addr).await?;
                }
''' + udp[insert_at:]
    rendezvous.write_text(content[:udp_start] + udp + content[udp_end:], encoding="utf-8")

    process = '''    async fn is_verified_fast_registration(&self, rk: &RegisterPk) -> bool {
        if rk.activation_id.len() != profile_activation::ACTIVATION_ID_BYTES
            || rk.activation_epoch == 0
            || rk.uuid.len() != 16
            || rk.pk.is_empty()
        {
            return false;
        }
        let Some(peer) = self.pm.get(&rk.id).await else {
            return false;
        };
        let peer = peer.read().await;
        !peer.uuid.is_empty() && peer.uuid == rk.uuid && peer.pk == rk.pk
    }

    async fn process_register_pk(
        &mut self,
        rk: RegisterPk,
        effective_addr: SocketAddr,
        native_addr: Option<SocketAddr>,
    ) -> Option<RegisterPkOutcome> {
        if rk.uuid.is_empty() || rk.pk.is_empty() {
            return None;
        }
        let fast_identity_verified = self.is_verified_fast_registration(&rk).await;
        let id = rk.id;
        let ip = effective_addr.ip().to_string();
        if id.len() < 6 {
            return Some(RegisterPkOutcome {
                result: register_pk_response::Result::UUID_MISMATCH,
                id,
                activation: None,
            });
        }
        if !self.check_ip_blocker(&ip, &id).await {
            return Some(RegisterPkOutcome {
                result: register_pk_response::Result::TOO_FREQUENT,
                id,
                activation: None,
            });
        }
        let peer = self.pm.get_or(&id).await;
        let (changed, ip_changed, current_addr) = {
            let peer = peer.read().await;
            let current_addr = peer.socket_addr;
            if peer.uuid.is_empty() {
                (true, false, current_addr)
            } else {
                if peer.uuid == rk.uuid {
                    if peer.info.ip != ip && peer.pk != rk.pk {
                        log::warn!("Peer identity mismatch after simultaneous IP and key change");
                        return Some(RegisterPkOutcome {
                            result: register_pk_response::Result::UUID_MISMATCH,
                            id,
                            activation: None,
                        });
                    }
                } else {
                    log::warn!("Peer UUID mismatch during registration");
                    return Some(RegisterPkOutcome {
                        result: register_pk_response::Result::UUID_MISMATCH,
                        id,
                        activation: None,
                    });
                }
                let ip_changed = peer.info.ip != ip;
                (
                    peer.uuid != rk.uuid || peer.pk != rk.pk || ip_changed,
                    ip_changed,
                    current_addr,
                )
            }
        };
        let activation = match profile_activation::begin_registration(
            &id,
            &rk.uuid,
            &rk.pk,
            rk.activation_epoch,
            &rk.activation_id,
            fast_identity_verified,
        )
        .await
        {
            Ok(claim) => claim,
            Err(_) => {
                return Some(RegisterPkOutcome {
                    result: register_pk_response::Result::TOO_FREQUENT,
                    id,
                    activation: None,
                });
            }
        };
        let mut req_pk = peer.read().await.reg_pk;
        if req_pk.1.elapsed().as_secs() > 6 {
            req_pk.0 = 0;
        } else if !fast_identity_verified && req_pk.0 > 2 {
            return Some(RegisterPkOutcome {
                result: register_pk_response::Result::TOO_FREQUENT,
                id,
                activation: None,
            });
        }
        req_pk.0 = req_pk.0.saturating_add(1);
        req_pk.1 = Instant::now();
        peer.write().await.reg_pk = req_pk;
        if ip_changed {
            let mut lock = IP_CHANGES.lock().await;
            if let Some((tm, ips)) = lock.get_mut(&id) {
                if tm.elapsed().as_secs() > IP_CHANGE_DUR {
                    *tm = Instant::now();
                    ips.clear();
                    ips.insert(ip.clone(), 1);
                } else if let Some(value) = ips.get_mut(&ip) {
                    *value += 1;
                } else {
                    ips.insert(ip.clone(), 1);
                }
            } else {
                lock.insert(
                    id.clone(),
                    (Instant::now(), HashMap::from([(ip.clone(), 1)])),
                );
            }
        }
        let result = if changed {
            self.pm
                .update_pk(
                    id.clone(),
                    peer,
                    native_addr.unwrap_or(current_addr),
                    rk.uuid,
                    rk.pk,
                    ip,
                )
                .await
        } else {
            let mut peer = peer.write().await;
            peer.last_reg_time = Instant::now();
            if let Some(addr) = native_addr {
                peer.socket_addr = addr;
            }
            register_pk_response::Result::OK
        };
        Some(RegisterPkOutcome {
            result,
            id,
            activation: (result == register_pk_response::Result::OK).then_some(activation),
        })
    }

    async fn process_peer_deactivation(&mut self, request: DeactivatePeer) -> bool {
        let peer_id = request.id;
        let Ok(claim) = profile_activation::begin_deactivation(
            &peer_id,
            &request.network_identity_uuid,
            request.activation_epoch,
            &request.activation_id,
            &request.route_lease,
            request.route_generation,
        )
        .await
        else {
            return false;
        };
        let route = claim.route();
        let route_removed = match route {
            profile_activation::RouteKind::Native(expected_addr) => {
                if let Some(peer) = self.pm.get_in_memory(&peer_id).await {
                    let mut peer = peer.write().await;
                    if peer.socket_addr == expected_addr
                        && peer.route_generation == claim.generation()
                    {
                        peer.socket_addr =
                            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
                        peer.route_generation = 0;
                        peer.last_reg_time = get_expired_time();
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            profile_activation::RouteKind::WebSocket {
                generation,
                connection_id,
            } => {
                let removed = websocket_signal::remove_profile_route(
                    &peer_id,
                    generation,
                    connection_id,
                )
                .await;
                if removed {
                    if let Some(peer) = self.pm.get_in_memory(&peer_id).await {
                        let mut peer = peer.write().await;
                        if peer.route_generation == generation {
                            peer.route_generation = 0;
                            peer.last_reg_time = get_expired_time();
                        }
                    }
                }
                removed
            }
        };
        claim.commit(route_removed).await
    }

    async fn activate_native_route(
        &self,
        peer_id: &str,
        addr: SocketAddr,
        route_generation: u64,
    ) {
        if let Some(peer) = self.pm.get_in_memory(peer_id).await {
            let mut peer = peer.write().await;
            peer.socket_addr = addr;
            peer.route_generation = route_generation;
            peer.last_reg_time = Instant::now();
        }
    }

    async fn activate_websocket_route(&self, peer_id: &str, route_generation: u64) {
        if let Some(peer) = self.pm.get_in_memory(peer_id).await {
            let mut peer = peer.write().await;
            peer.socket_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
            peer.route_generation = route_generation;
            peer.last_reg_time = Instant::now();
        }
    }

    async fn expire_websocket_route(&self, peer_id: &str, route_generation: u64) {
        if let Some(peer) = self.pm.get_in_memory(peer_id).await {
            let mut peer = peer.write().await;
            if peer.route_generation == route_generation {
                peer.route_generation = 0;
                peer.last_reg_time = get_expired_time();
            }
        }
    }

    async fn process_current_websocket_deactivation(
        &mut self,
        request: DeactivatePeer,
        token: &websocket_signal::SessionToken,
    ) -> bool {
        if request.id != token.peer_id || request.route_generation != token.generation {
            return false;
        }
        let Ok(claim) = profile_activation::begin_deactivation(
            &request.id,
            &request.network_identity_uuid,
            request.activation_epoch,
            &request.activation_id,
            &request.route_lease,
            request.route_generation,
        )
        .await
        else {
            return false;
        };
        if claim.route()
            != (profile_activation::RouteKind::WebSocket {
                generation: token.generation,
                connection_id: token.connection_id,
            })
        {
            return false;
        }
        let detached = websocket_signal::detach_profile_route(
            &token.peer_id,
            token.generation,
            token.connection_id,
        )
        .await;
        if detached {
            self.expire_websocket_route(&token.peer_id, token.generation)
                .await;
        }
        claim.commit(detached).await
    }

'''
    content = rendezvous.read_text(encoding="utf-8")
    if (
        content.count("async fn is_verified_fast_registration") != 1
        or "let fast_identity_verified = self.is_verified_fast_registration(&rk).await;"
        not in content
        or "connection_id: token.connection_id" not in content
    ):
        start_marker = (
            "    async fn is_verified_fast_registration"
            if "async fn is_verified_fast_registration" in content
            else "    async fn process_register_pk(\n"
        )
        start_at = content.index(start_marker)
        end_at = content.index(
            "    async fn touch_registered_peer(&self, peer_id: &str) {\n",
            start_at,
        )
        rendezvous.write_text(
            content[:start_at] + process + content[end_at:], encoding="utf-8"
        )

    content = rendezvous.read_text(encoding="utf-8")
    update_start = content.index("    async fn update_addr(")
    update_end = content.index("    async fn handle_hole_sent", update_start)
    update = content[update_start:update_end]
    if "        route_generation: u64,\n" not in update:
        update = update.replace(
            "        socket_addr: SocketAddr,\n        socket: &mut FramedSocket,\n",
            "        socket_addr: SocketAddr,\n"
            "        route_generation: u64,\n"
            "        socket: &mut FramedSocket,\n",
            1,
        )
        update = update.replace(
            "                old.socket_addr = socket_addr;\n"
            "                old.last_reg_time = Instant::now();\n",
            "                old.socket_addr = socket_addr;\n"
            "                if route_generation != 0 {\n"
            "                    old.route_generation = route_generation;\n"
            "                }\n"
            "                old.last_reg_time = Instant::now();\n",
            1,
        )
        rendezvous.write_text(
            content[:update_start] + update + content[update_end:], encoding="utf-8"
        )

    content = rendezvous.read_text(encoding="utf-8")
    tcp_start = content.index("    async fn handle_tcp(")
    tcp_end = content.index("    async fn update_addr(", tcp_start)
    tcp = content[tcp_start:tcp_end]
    if "Union::DeactivatePeer(request)" not in tcp:
        marker = "                Some(rendezvous_message::Union::RegisterPk(_)) => {\n"
        insert_at = tcp.index(marker)
        tcp = tcp[:insert_at] + '''                Some(rendezvous_message::Union::DeactivatePeer(request)) => {
                    let activation_epoch = request.activation_epoch;
                    let activation_id = request.activation_id.clone();
                    let route_generation = request.route_generation;
                    let deactivated = self.process_peer_deactivation(request).await;
                    let mut response = RendezvousMessage::new();
                    response.set_deactivate_peer_response(DeactivatePeerResponse {
                        activation_epoch,
                        activation_id,
                        deactivated,
                        route_generation,
                        ..Default::default()
                    });
                    Self::send_to_sink(sink, response).await;
                    return false;
                }
''' + tcp[insert_at:]
        rendezvous.write_text(
            content[:tcp_start] + tcp + content[tcp_end:], encoding="utf-8"
        )

    content = rendezvous.read_text(encoding="utf-8")
    drive_start = content.index("    async fn drive_websocket_signal(")
    drive_end = content.index("    async fn run_registered_websocket(", drive_start)
    drive = content[drive_start:drive_end]
    if (
        "self.activate_websocket_route(&outcome.id, generation)" not in drive
        or "connection_id: bound.connection_id" not in drive
    ):
        start = "                Some(rendezvous_message::Union::RegisterPk(rk)) => {\n"
        end = "                Some(_) => {\n"
        start_at = drive.index(start)
        end_at = drive.index(end, start_at)
        drive = drive[:start_at] + '''                Some(rendezvous_message::Union::RegisterPk(rk)) => {
                    if !websocket_signal::relay_ready() {
                        let mut response = RendezvousMessage::new();
                        response.set_register_pk_response(RegisterPkResponse {
                            result: register_pk_response::Result::SERVER_ERROR.into(),
                            ..Default::default()
                        });
                        if let Ok(bytes) = response.write_to_bytes() {
                            let _ = writer.send_binary(bytes);
                        }
                        log::warn!("WebSocket registration rejected: no healthy WSS Relay");
                        return None;
                    }
                    let verified_fast = self.is_verified_fast_registration(&rk).await;
                    if !verified_fast
                        && !websocket_signal::allow_registration(
                            effective_addr.ip(),
                            config.registration_rate_per_minute,
                        )
                        .await
                    {
                        let mut response = RendezvousMessage::new();
                        response.set_register_pk_response(RegisterPkResponse {
                            result: register_pk_response::Result::TOO_FREQUENT.into(),
                            ..Default::default()
                        });
                        if let Ok(bytes) = response.write_to_bytes() {
                            let _ = writer.send_binary(bytes);
                        }
                        return None;
                    }
                    if !websocket_signal::capacity_available(
                        &rk.id,
                        effective_addr.ip(),
                        config,
                    )
                    .await
                    {
                        let mut response = RendezvousMessage::new();
                        response.set_register_pk_response(RegisterPkResponse {
                            result: register_pk_response::Result::TOO_FREQUENT.into(),
                            ..Default::default()
                        });
                        if let Ok(bytes) = response.write_to_bytes() {
                            let _ = writer.send_binary(bytes);
                        }
                        return None;
                    }
                    let Some(mut outcome) =
                        self.process_register_pk(rk, effective_addr, None).await
                    else {
                        return None;
                    };
                    let mut token = None;
                    let mut activation_epoch = 0;
                    let mut activation_id = Bytes::new();
                    let mut route_lease = Bytes::new();
                    let mut route_generation = 0;
                    if outcome.result == register_pk_response::Result::OK {
                        if let Some(claim) = outcome.activation.take() {
                            let generation = claim.generation();
                            let tracked = claim.tracked();
                            match websocket_signal::bind(
                                outcome.id.clone(),
                                writer.clone(),
                                effective_addr.ip(),
                                route_addr,
                                Some(generation),
                                config,
                            )
                            .await
                            {
                                Ok(bound) => {
                                    activation_epoch = claim.activation_epoch();
                                    activation_id = claim.activation_id().clone();
                                    route_lease = claim.route_lease().clone();
                                    if tracked {
                                        route_generation = generation;
                                    }
                                    self.activate_websocket_route(&outcome.id, generation)
                                        .await;
                                    claim
                                        .commit(profile_activation::RouteKind::WebSocket {
                                            generation,
                                            connection_id: bound.connection_id,
                                        })
                                        .await;
                                    token = Some(bound);
                                }
                                Err(err) => {
                                    log::warn!("WebSocket registration capacity rejection: {err}");
                                    outcome.result = register_pk_response::Result::TOO_FREQUENT;
                                }
                            }
                        }
                    }
                    let keep_alive = (config.idle_timeout_ms / 1_500)
                        .clamp(1, i32::MAX as u64) as i32;
                    let mut response = RendezvousMessage::new();
                    response.set_register_pk_response(RegisterPkResponse {
                        result: outcome.result.into(),
                        keep_alive,
                        activation_epoch,
                        activation_id,
                        route_lease,
                        route_generation,
                        ..Default::default()
                    });
                    let sent = response
                        .write_to_bytes()
                        .ok()
                        .and_then(|bytes| writer.send_binary(bytes).ok())
                        .is_some();
                    if outcome.result != register_pk_response::Result::OK || !sent {
                        if let Some(token) = token.as_ref() {
                            if websocket_signal::remove_session(
                                token,
                                "registration response failed",
                            )
                            .await
                            {
                                self.expire_websocket_route(
                                    &token.peer_id,
                                    token.generation,
                                )
                                .await;
                            }
                        }
                        return None;
                    }
                    if route_generation != 0 {
                        profile_activation::record_ready_ack();
                    }
                    let Some(token) = token else {
                        return None;
                    };
                    self.touch_registered_peer(&token.peer_id).await;
                    return self
                        .run_registered_websocket(
                            stream,
                            writer,
                            route_addr,
                            effective_addr,
                            key,
                            config,
                            token,
                        )
                        .await;
                }
''' + drive[end_at:]
        rendezvous.write_text(
            content[:drive_start] + drive + content[drive_end:], encoding="utf-8"
        )

    content = rendezvous.read_text(encoding="utf-8")
    incoming_touch = '''                            token.touch();
                            self.touch_registered_peer(&token.peer_id).await;
'''
    incoming_touch_legacy = '''                            token.touch();
                            profile_activation::touch_route(
                                &token.peer_id,
                                profile_activation::RouteKind::WebSocket(token.generation),
                            )
                            .await;
                            self.touch_registered_peer(&token.peer_id).await;
'''
    incoming_touch_current = '''                            token.touch();
                            profile_activation::touch_route(
                                &token.peer_id,
                                profile_activation::RouteKind::WebSocket {
                                    generation: token.generation,
                                    connection_id: token.connection_id,
                                },
                            )
                            .await;
                            self.touch_registered_peer(&token.peer_id).await;
'''
    if incoming_touch_current not in content:
        replace_once(
            rendezvous,
            incoming_touch_legacy if incoming_touch_legacy in content else incoming_touch,
            incoming_touch_current,
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "process_current_websocket_deactivation(request, &token)" not in content:
        parsed_anchor = '''                            if RendezvousMessage::parse_from_bytes(&bytes).is_err() {
                                log::debug!("Closing registered WebSocket Signal session after malformed protobuf");
                                break;
                            }
                            let mut sink = Some(Sink::Ws(writer.clone()));
'''
        clean_anchor = '''                            if bytes.is_empty() {
                                continue;
                            }
                            let mut sink = Some(Sink::Ws(writer.clone()));
'''
        anchor = parsed_anchor if parsed_anchor in content else clean_anchor
        prefix = "" if anchor == parsed_anchor else '''                            if bytes.is_empty() {
                                continue;
                            }
'''
        replace_once(
            rendezvous,
            anchor,
            prefix + '''                            let parsed = match RendezvousMessage::parse_from_bytes(&bytes) {
                                Ok(parsed) => parsed,
                                Err(_) => {
                                    log::debug!("Closing registered WebSocket Signal session after malformed protobuf");
                                    break;
                                }
                            };
                            if let Some(rendezvous_message::Union::DeactivatePeer(request)) =
                                parsed.union
                            {
                                let activation_epoch = request.activation_epoch;
                                let activation_id = request.activation_id.clone();
                                let route_generation = request.route_generation;
                                let deactivated = self
                                    .process_current_websocket_deactivation(request, &token)
                                    .await;
                                let mut response = RendezvousMessage::new();
                                response.set_deactivate_peer_response(DeactivatePeerResponse {
                                    activation_epoch,
                                    activation_id,
                                    deactivated,
                                    route_generation,
                                    ..Default::default()
                                });
                                if let Ok(bytes) = response.write_to_bytes() {
                                    if writer.send_binary(bytes).is_err() {
                                        break;
                                    }
                                }
                                if deactivated {
                                    return None;
                                }
                                continue;
                            }
                            let mut sink = Some(Sink::Ws(writer.clone()));
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "profile activation cleanup after reader exit" not in content:
        replace_once(
            rendezvous,
            '''        if let Some(session) = session.as_ref() {
            websocket_signal::remove_session(session, "reader closed").await;
        }
''',
            '''        if let Some(session) = session.as_ref() {
            if websocket_signal::remove_session(session, "reader closed").await {
                // profile activation cleanup after reader exit
                self.expire_websocket_route(&session.peer_id, session.generation)
                    .await;
            }
        }
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if 'remove_session(&token, "idle timeout")' in content:
        replace_once(
            rendezvous,
            '''                    if token.idle_for() >= Duration::from_millis(config.idle_timeout_ms) {
                        websocket_signal::remove_session(&token, "idle timeout").await;
                        break;
                    }
''',
            '''                    if token.idle_for() >= Duration::from_millis(config.idle_timeout_ms) {
                        break;
                    }
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    heartbeat_without_touch = '''                _ = heartbeat.tick() => {
                    if writer.is_closed() {
                        break;
                    }
                    if token.idle_for() >= Duration::from_millis(config.idle_timeout_ms) {
                        break;
                    }
                    if writer.send_binary(Vec::new()).is_err() {
'''
    touch_before_liveness = '''                _ = heartbeat.tick() => {
                    profile_activation::touch_route(
                        &token.peer_id,
                        profile_activation::RouteKind::WebSocket {
                            generation: token.generation,
                            connection_id: token.connection_id,
                        },
                    )
                    .await;
                    if writer.is_closed() {
                        break;
                    }
                    if token.idle_for() >= Duration::from_millis(config.idle_timeout_ms) {
                        break;
                    }
'''
    legacy_touch_before_liveness = touch_before_liveness.replace(
        '''profile_activation::RouteKind::WebSocket {
                            generation: token.generation,
                            connection_id: token.connection_id,
                        },''',
        "profile_activation::RouteKind::WebSocket(token.generation),",
    )
    canonical_heartbeat = '''                _ = heartbeat.tick() => {
                    if writer.is_closed() {
                        break;
                    }
                    if token.idle_for() >= Duration::from_millis(config.idle_timeout_ms) {
                        break;
                    }
                    profile_activation::touch_route(
                        &token.peer_id,
                        profile_activation::RouteKind::WebSocket {
                            generation: token.generation,
                            connection_id: token.connection_id,
                        },
                    )
                    .await;
'''
    legacy_canonical_heartbeat = canonical_heartbeat.replace(
        '''profile_activation::RouteKind::WebSocket {
                            generation: token.generation,
                            connection_id: token.connection_id,
                        },''',
        "profile_activation::RouteKind::WebSocket(token.generation),",
    )
    if canonical_heartbeat in content:
        pass
    elif legacy_canonical_heartbeat in content:
        replace_once(
            rendezvous,
            legacy_canonical_heartbeat,
            canonical_heartbeat,
        )
    elif touch_before_liveness in content or legacy_touch_before_liveness in content:
        replace_once(
            rendezvous,
            touch_before_liveness
            if touch_before_liveness in content
            else legacy_touch_before_liveness,
            canonical_heartbeat,
        )
    else:
        replace_once(
            rendezvous,
            heartbeat_without_touch,
            canonical_heartbeat + '''                    if writer.send_binary(Vec::new()).is_err() {
''',
        )

    # Older development overlays could append the tuple-form touch call on
    # every application. RouteKind::WebSocket is now a generation-safe struct
    # variant, so remove only that obsolete exact call and retain the single
    # canonical connection-id-bound touch above.
    content = rendezvous.read_text(encoding="utf-8")
    obsolete_websocket_touch = '''                    profile_activation::touch_route(
                        &token.peer_id,
                        profile_activation::RouteKind::WebSocket(token.generation),
                    )
                    .await;
'''
    if obsolete_websocket_touch in content:
        rendezvous.write_text(
            content.replace(obsolete_websocket_touch, ""), encoding="utf-8"
        )

    content = rendezvous.read_text(encoding="utf-8")
    old_send = '''async fn send_rk_res(
    socket: &mut FramedSocket,
    addr: SocketAddr,
    res: register_pk_response::Result,
) -> ResultType<()> {
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_register_pk_response(RegisterPkResponse {
        result: res.into(),
        ..Default::default()
    });
    socket.send(&msg_out, addr).await
}
'''
    if "    activation_epoch: u64,\n    activation_id: Bytes," not in content:
        replace_once(
            rendezvous,
            old_send,
            '''async fn send_rk_res(
    socket: &mut FramedSocket,
    addr: SocketAddr,
    res: register_pk_response::Result,
    activation_epoch: u64,
    activation_id: Bytes,
    route_lease: Bytes,
    route_generation: u64,
) -> ResultType<()> {
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_register_pk_response(RegisterPkResponse {
        result: res.into(),
        activation_epoch,
        activation_id,
        route_lease,
        route_generation,
        ..Default::default()
    });
    socket.send(&msg_out, addr).await
}
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    old_augmented_send = '''    activation_epoch: u64,
    activation_id: Bytes,
    route_lease: Bytes,
) -> ResultType<()> {
'''
    if old_augmented_send in content:
        replace_once(
            rendezvous,
            old_augmented_send,
            '''    activation_epoch: u64,
    activation_id: Bytes,
    route_lease: Bytes,
    route_generation: u64,
) -> ResultType<()> {
''',
        )
        content = rendezvous.read_text(encoding="utf-8")
        send_start = content.index("async fn send_rk_res(")
        send_block = content[send_start:]
        if "        route_generation,\n" not in send_block:
            replace_once(
                rendezvous,
                '''        activation_id,
        route_lease,
        ..Default::default()
''',
                '''        activation_id,
        route_lease,
        route_generation,
        ..Default::default()
''',
            )


def patch_relay_quality_rendezvous(upstream: Path) -> None:
    rendezvous = upstream / "src/rendezvous_server.rs"
    content = rendezvous.read_text(encoding="utf-8")

    prepare_migrations = (
        (
            "            let prepared_websocket = match "
            "websocket_signal::prepare(&config.websocket_signal) {\n",
            "            let prepared_websocket = match websocket_signal::prepare(\n"
            "                &config.websocket_signal,\n"
            "                config.relay_quality.enabled,\n"
            "            ) {\n",
        ),
        (
            "            match websocket_signal::prepare(&config.websocket_signal)\n"
            "                .and_then(websocket_signal::activate_prepared)\n",
            "            match websocket_signal::prepare(\n"
            "                &config.websocket_signal,\n"
            "                config.relay_quality.enabled,\n"
            "            )\n"
            "            .and_then(websocket_signal::activate_prepared)\n",
        ),
        (
            "        let prepared_websocket = "
            "websocket_signal::prepare(&candidate.config.websocket_signal)?;\n",
            "        let prepared_websocket = websocket_signal::prepare(\n"
            "            &candidate.config.websocket_signal,\n"
            "            candidate.config.relay_quality.enabled,\n"
            "        )?;\n",
        ),
    )
    for old, new in prepare_migrations:
        if old in content:
            replace_once(rendezvous, old, new)
            content = rendezvous.read_text(encoding="utf-8")

    if "let relay_quality_protocol = ph.relay_quality_protocol;" not in content:
        replace_once(
            rendezvous,
            "        let mut ph = ph;\n"
            "        if !key.is_empty() && ph.licence_key != key {\n",
            "        let mut ph = ph;\n"
            "        let relay_quality_protocol = ph.relay_quality_protocol;\n"
            "        if !key.is_empty() && ph.licence_key != key {\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "let relay_quality_offer = relay_quality::create_offer(" not in content:
        replace_once(
            rendezvous,
            "                ph.nat_type = NatType::SYMMETRIC.into();\n"
            "            }\n"
            "            let same_intranet = !ws\n",
            "                ph.nat_type = NatType::SYMMETRIC.into();\n"
            "            }\n"
            "            let relay_quality_offer = relay_quality::create_offer(\n"
            "                relay_quality_protocol,\n"
            "                route_addr,\n"
            "                effective_addr.ip(),\n"
            "                peer_route_addr,\n"
            "                peer_effective_addr.ip(),\n"
            "                &id,\n"
            "                requirement,\n"
            "                ws,\n"
            "                target_ws,\n"
            "                &relay_server,\n"
            "            );\n"
            "            let same_intranet = !ws\n",
        )
        replace_once(
            rendezvous,
            "                msg_out.set_fetch_local_addr(FetchLocalAddr {\n"
            "                    socket_addr,\n"
            "                    relay_server,\n"
            "                    ..Default::default()\n"
            "                });\n",
            "                msg_out.set_fetch_local_addr(FetchLocalAddr {\n"
            "                    socket_addr,\n"
            "                    relay_server,\n"
            "                    relay_quality_offer: MessageField::from_option(\n"
            "                        relay_quality_offer.clone(),\n"
            "                    ),\n"
            "                    ..Default::default()\n"
            "                });\n",
        )
        replace_once(
            rendezvous,
            "                msg_out.set_punch_hole(PunchHole {\n"
            "                    socket_addr,\n"
            "                    nat_type: ph.nat_type,\n"
            "                    relay_server,\n"
            "                    ..Default::default()\n"
            "                });\n",
            "                msg_out.set_punch_hole(PunchHole {\n"
            "                    socket_addr,\n"
            "                    nat_type: ph.nat_type,\n"
            "                    relay_server,\n"
            "                    relay_quality_offer: MessageField::from_option(\n"
            "                        relay_quality_offer,\n"
            "                    ),\n"
            "                    ..Default::default()\n"
            "                });\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    legacy_quality_call = (
        "                peer_effective_addr.ip(),\n"
        "                requirement,\n"
        "                &relay_server,\n"
    )
    if legacy_quality_call in content:
        replace_once(
            rendezvous,
            legacy_quality_call,
            "                peer_effective_addr.ip(),\n"
            "                requirement,\n"
            "                ws,\n"
            "                target_ws,\n"
            "                &relay_server,\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "let quality = relay_quality::response_context(" not in content:
        hole_anchor = (
            "        let addr_a = AddrMangle::decode(&phs.socket_addr);\n"
            "        log::debug!(\n"
        )
        replace_once(
            rendezvous,
            hole_anchor,
            "        let addr_a = AddrMangle::decode(&phs.socket_addr);\n"
            "        let target_effective = websocket_signal::connection_effective(addr)\n"
            "            .await\n"
            "            .unwrap_or(addr);\n"
            "        let quality = relay_quality::response_context(\n"
            "            addr_a,\n"
            "            addr,\n"
            "            target_effective.ip(),\n"
            "            &phs.id,\n"
            "            phs.relay_quality_report.clone().into_option(),\n"
            "        );\n"
            "        log::debug!(\n",
        )
        replace_once(
            rendezvous,
            "            relay_server: phs.relay_server.clone(),\n"
            "            ..Default::default()\n",
            "            relay_server: phs.relay_server.clone(),\n"
            "            relay_quality_offer: MessageField::from_option(quality.offer),\n"
            "            relay_quality_peer_report: MessageField::from_option(quality.peer_report),\n"
            "            ..Default::default()\n",
        )

        local_anchor = (
            "        let addr_a = AddrMangle::decode(&la.socket_addr);\n"
            "        log::debug!(\n"
        )
        replace_once(
            rendezvous,
            local_anchor,
            "        let addr_a = AddrMangle::decode(&la.socket_addr);\n"
            "        let target_effective = websocket_signal::connection_effective(addr)\n"
            "            .await\n"
            "            .unwrap_or(addr);\n"
            "        let quality = relay_quality::response_context(\n"
            "            addr_a,\n"
            "            addr,\n"
            "            target_effective.ip(),\n"
            "            &la.id,\n"
            "            la.relay_quality_report.clone().into_option(),\n"
            "        );\n"
            "        log::debug!(\n",
        )
        replace_once(
            rendezvous,
            "            relay_server: la.relay_server,\n"
            "            ..Default::default()\n",
            "            relay_server: la.relay_server,\n"
            "            relay_quality_offer: MessageField::from_option(quality.offer),\n"
            "            relay_quality_peer_report: MessageField::from_option(quality.peer_report),\n"
            "            ..Default::default()\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "let quality_resolution = if let Some((" not in content:
        legacy_selection = (
            "                    if let Some(quality_decision) = relay_quality::select_for_request(\n"
            "                        effective_addr.ip(),\n"
            "                        &rf.uuid,\n"
            "                        rf.relay_quality_report.clone().into_option(),\n"
            "                    ) {\n"
            "                        rf.relay_server = quality_decision.relay_server.clone();\n"
            "                        rf.relay_quality_decision = MessageField::some(quality_decision);\n"
            "                    }\n"
            "                    let peer_id = rf.id.clone();\n"
            "                    let websocket_target = websocket_signal::route(&peer_id).await.is_some();\n"
        )
        original_selection = (
            "                    let peer_id = rf.id.clone();\n"
            "                    let websocket_target = websocket_signal::route(&peer_id).await.is_some();\n"
        )
        anchor = legacy_selection if legacy_selection in content else original_selection
        replace_once(
            rendezvous,
            anchor,
            "                    let peer_id = rf.id.clone();\n"
            "                    let websocket_route = websocket_signal::route(&peer_id).await;\n"
            "                    let websocket_target = websocket_route.is_some();\n"
            "                    let requirement = match (ws, websocket_target) {\n"
            "                        (true, true) => websocket_signal::RelayRequirement::WebSocketOnly,\n"
            "                        (true, false) | (false, true) => websocket_signal::RelayRequirement::Mixed,\n"
            "                        (false, false) => websocket_signal::RelayRequirement::NativeOnly,\n"
            "                    };\n"
            "                    let target = if let Some(route) = websocket_route.as_ref() {\n"
            "                        Some((route.route_addr, route.effective_ip))\n"
            "                    } else if let Some(peer) = self.pm.get_in_memory(&peer_id).await {\n"
            "                        let peer_addr = peer.read().await.socket_addr;\n"
            "                        Some((peer_addr, peer_addr.ip()))\n"
            "                    } else {\n"
            "                        None\n"
            "                    };\n"
            "                    let server_selected_relay = target.as_ref().map(|(_, target_ip)| {\n"
            "                        self.get_relay_server(\n"
            "                            effective_addr.ip(),\n"
            "                            *target_ip,\n"
            "                            requirement,\n"
            "                        )\n"
            "                    });\n"
            "                    let quality_resolution = if let Some((_target_route, target_ip)) = target.as_ref() {\n"
            "                        relay_quality::select_for_request(\n"
            "                            route_addr,\n"
            "                            effective_addr.ip(),\n"
            "                            *target_ip,\n"
            "                            &rf.uuid,\n"
            "                            rf.relay_quality_allocation_id.as_ref(),\n"
            "                            rf.relay_quality_report.clone().into_option(),\n"
            "                        )\n"
            "                    } else {\n"
            "                        relay_quality::RequestResolution::Legacy\n"
            "                    };\n"
            "                    let quality_selection = match quality_resolution {\n"
            "                        relay_quality::RequestResolution::Legacy => None,\n"
            "                        relay_quality::RequestResolution::Selected(selection) => {\n"
            "                            rf.relay_server = selection.decision.relay_server.clone();\n"
            "                            rf.relay_quality_decision =\n"
            "                                MessageField::some(selection.decision.clone());\n"
            "                            Some(selection)\n"
            "                        }\n"
            "                        relay_quality::RequestResolution::Expanded(dispatch) => {\n"
            "                            self.dispatch_relay_quality(dispatch).await;\n"
            "                            return true;\n"
            "                        }\n"
            "                        relay_quality::RequestResolution::Pending\n"
            "                        | relay_quality::RequestResolution::Blocked => return true,\n"
            "                    };\n"
            "                    if quality_selection.is_none() && fast_relay::enabled() {\n"
            "                        if let Some(selected) = server_selected_relay.as_ref() {\n"
            "                            rf.relay_server = selected.clone();\n"
            "                        }\n"
            "                    }\n"
            "                    let fast_media_endpoint =\n"
            "                        relay_observer::fast_media_endpoint(&rf.relay_server);\n"
            "                    rf.fast_relay_authorization = target\n"
            "                        .as_ref()\n"
            "                        .and_then(|(_, target_ip)| {\n"
            "                            fast_relay::authorization_for_request(\n"
            "                                &rf.uuid,\n"
            "                                effective_addr.ip(),\n"
            "                                *target_ip,\n"
            "                                &rf.relay_server,\n"
            "                                signal_transport,\n"
            "                                &decision,\n"
            "                                quality_selection.as_ref(),\n"
            "                                fast_media_endpoint,\n"
            "                                self.inner.sk.as_ref(),\n"
            "                            )\n"
            "                        })\n"
            "                        .unwrap_or_default();\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "relay_quality_deadline_tick" not in content:
        replace_once(
            rendezvous,
            "        let mut timer_check_relay = interval(Duration::from_millis(CHECK_RELAY_TIMEOUT));\n"
            "        loop {\n"
            "            tokio::select! {\n",
            "        let mut timer_check_relay = interval(Duration::from_millis(CHECK_RELAY_TIMEOUT));\n"
            "        let mut relay_quality_deadline_tick = interval(Duration::from_millis(100));\n"
            "        relay_quality_deadline_tick\n"
            "            .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);\n"
            "        loop {\n"
            "            tokio::select! {\n"
            "                _ = relay_quality_deadline_tick.tick() => {\n"
            "                    for dispatch in relay_quality::finalize_expired() {\n"
            "                        self.dispatch_relay_quality(dispatch).await;\n"
            "                    }\n"
            "                }\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "fn relay_quality_message(" not in content:
        replace_once(
            rendezvous,
            "    #[inline]\n"
            "    async fn send_to_tcp(&mut self, msg: RendezvousMessage, addr: SocketAddr) {\n",
            "    fn relay_quality_message(\n"
            "        payload: relay_quality::DispatchPayload,\n"
            "    ) -> RendezvousMessage {\n"
            "        let mut message = RendezvousMessage::new();\n"
            "        match payload {\n"
            "            relay_quality::DispatchPayload::Offer(offer) => {\n"
            "                message.set_relay_quality_stage_offer(offer);\n"
            "            }\n"
            "            relay_quality::DispatchPayload::Decision(decision) => {\n"
            "                message.set_relay_quality_stage_decision(decision);\n"
            "            }\n"
            "            relay_quality::DispatchPayload::Cancel(cancel) => {\n"
            "                message.set_relay_quality_cancel(cancel);\n"
            "            }\n"
            "        }\n"
            "        message\n"
            "    }\n\n"
            "    async fn dispatch_relay_quality(\n"
            "        &mut self,\n"
            "        dispatch: relay_quality::StageDispatch,\n"
            "    ) {\n"
            "        let controller = Self::relay_quality_message(dispatch.controller);\n"
            "        allow_err!(self.send_to_tcp_sync(controller, dispatch.controller_route).await);\n"
            "        let target = Self::relay_quality_message(dispatch.target);\n"
            "        if dispatch.target_websocket {\n"
            "            if !websocket_signal::send_to_peer(&dispatch.target_id, &target).await {\n"
            "                log::warn!(\"Relay Quality stage delivery failed for current WebSocket route\");\n"
            "            }\n"
            "        } else {\n"
            "            self.tx.send(Data::Msg(target.into(), dispatch.target_route)).ok();\n"
            "        }\n"
            "    }\n\n"
            "    #[inline]\n"
            "    async fn send_to_tcp(&mut self, msg: RendezvousMessage, addr: SocketAddr) {\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "Union::RelayQualityStageReport(report)" not in content:
        replace_once(
            rendezvous,
            "                Some(rendezvous_message::Union::RequestRelay(mut rf)) => {\n",
            "                Some(rendezvous_message::Union::RelayQualityStageReport(report)) => {\n"
            "                    if let Some(sink) = sink.take() {\n"
            "                        self.tcp_punch.lock().await.insert(try_into_v4(route_addr), sink);\n"
            "                    }\n"
            "                    if let relay_quality::StageReportResolution::Dispatch(dispatch) =\n"
            "                        relay_quality::handle_stage_report(\n"
            "                            route_addr,\n"
            "                            effective_addr.ip(),\n"
            "                            report,\n"
            "                        )\n"
            "                    {\n"
            "                        self.dispatch_relay_quality(dispatch).await;\n"
            "                    }\n"
            "                    return true;\n"
            "                }\n"
            "                Some(rendezvous_message::Union::RelayQualityCancel(cancel)) => {\n"
            "                    if let Some(sink) = sink.take() {\n"
            "                        self.tcp_punch.lock().await.insert(try_into_v4(route_addr), sink);\n"
            "                    }\n"
            "                    if let Some(dispatch) = relay_quality::cancel_allocation(\n"
            "                        route_addr,\n"
            "                        effective_addr.ip(),\n"
            "                        cancel,\n"
            "                    ) {\n"
            "                        self.dispatch_relay_quality(dispatch).await;\n"
            "                    }\n"
            "                    return true;\n"
            "                }\n"
            "                Some(rendezvous_message::Union::RequestRelay(mut rf)) => {\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "relay_quality::handle_stage_report(addr, addr.ip(), report)" not in content:
        replace_once(
            rendezvous,
            "                Some(rendezvous_message::Union::PunchHoleRequest(_ph)) => {\n",
            "                Some(rendezvous_message::Union::RelayQualityStageReport(report)) => {\n"
            "                    if let relay_quality::StageReportResolution::Dispatch(dispatch) =\n"
            "                        relay_quality::handle_stage_report(addr, addr.ip(), report)\n"
            "                    {\n"
            "                        self.dispatch_relay_quality(dispatch).await;\n"
            "                    }\n"
            "                }\n"
            "                Some(rendezvous_message::Union::RelayQualityCancel(cancel)) => {\n"
            "                    if let Some(dispatch) =\n"
            "                        relay_quality::cancel_allocation(addr, addr.ip(), cancel)\n"
            "                    {\n"
            "                        self.dispatch_relay_quality(dispatch).await;\n"
            "                    }\n"
            "                }\n"
            "                Some(rendezvous_message::Union::PunchHoleRequest(_ph)) => {\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "relay_quality::decision_for_response(&rr.uuid, effective_addr.ip());" not in content:
        legacy = (
            "                Some(rendezvous_message::Union::RelayResponse(mut rr)) => {\n"
            "                    if let Some(quality_decision) =\n"
            "                        relay_quality::decision_for_response(&rr.uuid)\n"
            "                    {\n"
            "                        rr.relay_server = quality_decision.relay_server.clone();\n"
            "                        rr.relay_quality_decision = MessageField::some(quality_decision);\n"
            "                    }\n"
            "                    let addr_b = AddrMangle::decode(&rr.socket_addr);\n"
        )
        original = (
            "                Some(rendezvous_message::Union::RelayResponse(mut rr)) => {\n"
            "                    let addr_b = AddrMangle::decode(&rr.socket_addr);\n"
        )
        anchor = legacy if legacy in content else original
        replace_once(
            rendezvous,
            anchor,
            "                Some(rendezvous_message::Union::RelayResponse(mut rr)) => {\n"
            "                    let relay_quality_decision =\n"
            "                        relay_quality::decision_for_response(&rr.uuid, effective_addr.ip());\n"
            "                    let addr_b = AddrMangle::decode(&rr.socket_addr);\n",
        )
        replace_once(
            rendezvous,
            "                    msg_out.set_relay_response(rr);\n",
            "                    if let Some(quality_decision) = relay_quality_decision {\n"
            "                        rr.relay_server = quality_decision.relay_server.clone();\n"
            "                        rr.relay_quality_decision = MessageField::some(quality_decision);\n"
            "                    }\n"
            "                    msg_out.set_relay_response(rr);\n",
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "fast_relay::authorization_for_response(" not in content:
        replace_once(
            rendezvous,
            "                    msg_out.set_relay_response(rr);\n",
            "                    if let Some(server_selected_relay) =\n"
            "                        fast_relay::selected_relay_for_response(\n"
            "                            &rr.uuid,\n"
            "                            effective_addr.ip(),\n"
            "                        )\n"
            "                    {\n"
            "                        rr.relay_server = server_selected_relay;\n"
            "                    }\n"
            "                    rr.fast_relay_authorization =\n"
            "                        fast_relay::authorization_for_response(\n"
            "                            &rr.uuid,\n"
            "                            effective_addr.ip(),\n"
            "                            &rr.relay_server,\n"
            "                        )\n"
            "                        .unwrap_or_default();\n"
            "                    msg_out.set_relay_response(rr);\n",
        )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("upstream", type=Path, help="clean rustdesk-server checkout")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[1]
    upstream = args.upstream.resolve()
    required = (
        upstream / "Cargo.toml",
        upstream / "src/lib.rs",
        upstream / "src/main.rs",
        upstream / "src/rendezvous_server.rs",
    )
    if not all(path.is_file() for path in required):
        raise SystemExit(f"not a compatible rustdesk-server checkout: {upstream}")

    copy_overlay(repo_root, upstream)
    patch_dependencies(upstream)
    patch_hbb_common(upstream)
    patch_common(upstream)
    patch_modules(upstream)
    patch_cli(upstream)
    patch_relay(upstream)
    patch_relay_quality_hbbr(upstream)
    patch_rendezvous(upstream)
    patch_profile_activation_hbbs(upstream)
    patch_relay_quality_rendezvous(upstream)
    print(f"rustdesk-server-starry overlay applied to {upstream}")


if __name__ == "__main__":
    main()

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


def copy_overlay(repo_root: Path, upstream: Path) -> None:
    for name in ("geo_relay.rs", "secure_tcp.rs", "starry_config.rs"):
        shutil.copyfile(repo_root / "overlay/src" / name, upstream / "src" / name)
    shutil.copyfile(
        repo_root / "config/config.example.yaml",
        upstream / "src/starry_config.example.yaml",
    )
    shutil.copytree(
        repo_root / "overlay/src/geo_relay",
        upstream / "src/geo_relay",
        dirs_exist_ok=True,
    )


def patch_dependencies(upstream: Path) -> None:
    cargo = upstream / "Cargo.toml"
    cargo_text = cargo.read_text(encoding="utf-8")
    if 'maxminddb = "0.30"' not in cargo_text:
        replace_once(
            cargo,
            'flate2 = "1.0"\n',
            'flate2 = "1.0"\nmaxminddb = "0.30"\n',
        )

    cargo_text = cargo.read_text(encoding="utf-8")
    if 'serde_yml = "0.0.13"' not in cargo_text:
        replace_once(
            cargo,
            'maxminddb = "0.30"\n',
            'maxminddb = "0.30"\nserde_yml = "0.0.13"\n',
        )

    cargo_text = cargo.read_text(encoding="utf-8")
    legacy_signature_pin = (
        '# sodiumoxide 0.2.7 requires the pre-2.0 signature trait.\n'
        'signature = "=1.5.0"\n'
    )
    ed25519_pin = (
        '# ed25519 1.5.3 fixes its signature dependency to exclude 2.x.\n'
        'ed25519 = "=1.5.3"\n'
    )
    if legacy_signature_pin in cargo_text:
        replace_once(cargo, legacy_signature_pin, ed25519_pin)
    elif 'ed25519 = "=1.5.3"' not in cargo_text:
        if re.search(r"(?m)^\s*ed25519\s*=", cargo_text):
            raise RuntimeError(
                "upstream now declares ed25519; review the compatibility pin"
            )
        replace_once(
            cargo,
            'serde_yml = "0.0.13"\n',
            'serde_yml = "0.0.13"\n'
            + ed25519_pin,
        )


def patch_modules(upstream: Path) -> None:
    lib = upstream / "src/lib.rs"
    lib_text = lib.read_text(encoding="utf-8")
    if "mod starry_config;" not in lib_text:
        replace_once(
            lib,
            "mod rendezvous_server;\n",
            "mod geo_relay;\nmod rendezvous_server;\nmod secure_tcp;\nmod starry_config;\n",
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


def patch_rendezvous(upstream: Path) -> None:
    rendezvous = upstream / "src/rendezvous_server.rs"
    content = rendezvous.read_text(encoding="utf-8")
    if "use crate::{geo_relay, secure_tcp, starry_config};" not in content:
        replace_once(
            rendezvous,
            "use crate::common::*;\nuse crate::peer::*;\n",
            "use crate::common::*;\n"
            "use crate::{geo_relay, secure_tcp, starry_config};\n"
            "use crate::peer::*;\n",
        )

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
            "type WsSink = SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, tungstenite::Message>;\n"
            "enum Sink {\n"
            "    TcpStream(secure_tcp::TcpWriteTransport),\n",
        )

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
            '        log::info!("Geo relay startup: {}", geo_relay::reload());\n'
            '        log::info!("{}", geo_relay::start_mmdb_updater());\n',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if "geo_relay::select_relay" not in content:
        replace_once(
            rendezvous,
            '''    fn get_relay_server(&self, _pa: IpAddr, _pb: IpAddr) -> String {
        if self.relay_servers.is_empty() {
            return "".to_owned();
        } else if self.relay_servers.len() == 1 {
            return self.relay_servers[0].clone();
        }
        let i = ROTATION_RELAY_SERVER.fetch_add(1, Ordering::SeqCst) % self.relay_servers.len();
        self.relay_servers[i].clone()
    }
''',
            '''    fn get_relay_server(&self, pa: IpAddr, pb: IpAddr) -> String {
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
''',
        )

    content = rendezvous.read_text(encoding="utf-8")
    if 'Some("reload-starry-config" | "rsc") =>' not in content:
        replace_once(
            rendezvous,
            '            Some("relay-servers" | "rs") => {\n',
            '            Some("reload-starry-config" | "rsc") => {\n'
            '                let outcome = starry_config::reload();\n'
            '                let relay_servers = outcome\n'
            '                    .relay_servers\n'
            '                    .unwrap_or_else(|| get_arg("relay-servers"));\n'
            '                self.tx.send(Data::RelayServers0(relay_servers)).ok();\n'
            '                res = format!("{}; {}", outcome.message, geo_relay::reload());\n'
            '            }\n'
            '            Some("reload-geo" | "rg") => {\n'
            '                res = geo_relay::reload();\n'
            '            }\n'
            '            Some("relay-servers" | "rs") => {\n',
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
    if "secure_tcp::negotiate" in content:
        if "    bytes_codec::BytesCodec,\n" in content:
            replace_once(rendezvous, "    bytes_codec::BytesCodec,\n", "")
        content = rendezvous.read_text(encoding="utf-8")
        if "    tokio_util::codec::Framed,\n" in content:
            replace_once(rendezvous, "    tokio_util::codec::Framed,\n", "")


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
    patch_modules(upstream)
    patch_cli(upstream)
    patch_rendezvous(upstream)
    print(f"rustdesk-server-starry overlay applied to {upstream}")


if __name__ == "__main__":
    main()

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


def copy_overlay(repo_root: Path, upstream: Path) -> None:
    for name in (
        "geo_relay.rs",
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
    if 'url = "2.2"' not in cargo_text:
        replace_once(
            cargo,
            'serde_yml = "0.0.13"\n',
            'serde_yml = "0.0.13"\nurl = "2.2"\n',
        )

    cargo_text = cargo.read_text(encoding="utf-8")
    plain_websocket_dependency = 'tokio-tungstenite = "0.17"\n'
    tls_websocket_dependency = (
        'tokio-tungstenite = { version = "0.17", '
        'features = ["rustls-tls-native-roots"] }\n'
    )
    if plain_websocket_dependency in cargo_text:
        replace_once(cargo, plain_websocket_dependency, tls_websocket_dependency)
    elif tls_websocket_dependency not in cargo_text:
        raise RuntimeError(
            "upstream tokio-tungstenite dependency changed; review TLS feature injection"
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

    cargo_text = cargo.read_text(encoding="utf-8")
    websocket_test_dependencies = (
        '[dev-dependencies]\n'
        'rcgen = "0.9"\n'
        'tokio-rustls = "0.23"\n\n'
    )
    if websocket_test_dependencies not in cargo_text:
        if re.search(r"(?m)^\[dev-dependencies\]$", cargo_text):
            raise RuntimeError(
                "upstream now declares dev-dependencies; review WebSocket test dependency injection"
            )
        replace_once(
            cargo,
            "[build-dependencies]\n",
            websocket_test_dependencies + "[build-dependencies]\n",
        )


def patch_modules(upstream: Path) -> None:
    lib = upstream / "src/lib.rs"
    lib_text = lib.read_text(encoding="utf-8")
    if "mod starry_config;" not in lib_text:
        replace_once(
            lib,
            "mod rendezvous_server;\n",
            "mod geo_relay;\n"
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
    if "use crate::{geo_relay, secure_tcp, starry_config, websocket_signal};" not in content:
        if "use crate::{geo_relay, secure_tcp, starry_config};" in content:
            replace_once(
                rendezvous,
                "use crate::{geo_relay, secure_tcp, starry_config};\n",
                "use crate::{geo_relay, secure_tcp, starry_config, websocket_signal};\n",
            )
        else:
            replace_once(
                rendezvous,
                "use crate::common::*;\nuse crate::peer::*;\n",
                "use crate::common::*;\n"
                "use crate::{geo_relay, secure_tcp, starry_config, websocket_signal};\n"
                "use crate::peer::*;\n",
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
            '        let geo_startup = geo_relay::reload();\n'
            '        log::info!("Geo relay startup: {}", geo_startup);\n'
            '        let mmdb_startup = geo_relay::start_mmdb_updater();\n'
            '        log::info!("{}", mmdb_startup);\n',
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
    if new_register_peer not in content:
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
    if new_update_signature not in content:
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
    transport_relay_method = '''    fn get_relay_server(
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
    if transport_relay_method not in content:
        if old_geo_method in content:
            replace_once(rendezvous, old_geo_method, transport_relay_method)
        else:
            replace_once(rendezvous, native_relay_method, transport_relay_method)

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
    old_reload_result = '                res = format!("{}; {}", outcome.message, geo_relay::reload());\n'
    new_reload_result = '''                res = format!(
                    "{}; {}; {}",
                    outcome.message,
                    geo_relay::reload(),
                    websocket_signal::reconfigure()
                );
'''
    if new_reload_result not in content:
        replace_once(rendezvous, old_reload_result, new_reload_result)

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
    new_test_geo = '''            Some("test-geo" | "tg") => {
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
    if new_test_geo not in content:
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
        let websocket = tokio_tungstenite::accept_hdr_async(stream, callback).await?;
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
        websocket_signal::register_connection(route_addr, effective_addr, connection_id).await;

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
        loop {
            let first = match timeout(config.registration_timeout_ms, stream.next()).await {
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
    if legacy_actor_sink not in content:
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
    if new_request_relay not in content:
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
            let (elapsed, peer_effective_addr, target_ws) =
                if let Some(route) = target_websocket {
                    (
                        route.idle_millis as i64,
                        SocketAddr::new(route.effective_ip, 0),
                        true,
                    )
                } else {
                    (native_elapsed, native_peer_addr, false)
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

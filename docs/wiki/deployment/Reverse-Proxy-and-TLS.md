# Reverse proxy and TLS

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-Reverse-Proxy-and-TLS)

WebSocket Signal requires two distinct public WSS paths. Nginx terminates TLS;
HBBS and HBBR continue to listen on plain private backend ports.

## Required paths

| Public path | Backend | Purpose |
| --- | --- | --- |
| `wss://id.example.com/ws/id` | Starry HBBS `127.0.0.1:21118` | Persistent identity registration and signalling. |
| `wss://relay-1.example.com/ws/relay` | Starry-image HBBR `127.0.0.1:21119` | Relay data for that exact node. |
| `https://api.example.com/` | Optional API `127.0.0.1:12345` | Independent account/admin API. |

Do not rewrite `/ws/id` to `/ws/relay` or combine all Relay names behind one
endpoint unless the resulting name still identifies the same HBBR node that
HBBS allocates.

## Reference configurations

- Complete single-host bootstrap: [`single-host.bootstrap.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/single-host.bootstrap.conf)
- Complete single-host HBBS + HBBR WSS: [`single-host.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/single-host.example.conf)
- Complete centre WSS server: [`center.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/center.example.conf)
- Complete Relay WSS server: [`relay.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/relay.example.conf)
- Complete optional API server: [`api.example.conf`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/examples/nginx/api.example.conf)
- Location-only fragments: [`examples/nginx/`](https://github.com/q1ngyang/rustdesk-server-starry/tree/main/examples/nginx)

Replace every example name and certificate path. Do not copy an existing site
configuration without first checking for duplicate `location` blocks.

The API file is a generic placeholder, not a Kessoku proxy contract. Follow
the [Kessoku Wiki](https://github.com/q1ngyang/rustdesk-api-kessoku/wiki) for
its public/internal listeners and trust boundary.

## Centre `/ws/id`

The essential contract is:

```nginx
location = /ws/id {
    proxy_pass http://127.0.0.1:21118;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_buffering off;
    proxy_read_timeout 120s;
    proxy_send_timeout 120s;
}
```

When Nginx is on the same host, the default `trusted_proxies` loopback CIDRs
allow forwarded client addresses. When a separate proxy network is used, add
only its real source CIDR. Never trust all Internet addresses merely to make
`X-Forwarded-For` visible.

## Relay `/ws/relay`

Every Relay hostname needs an exact local mapping:

```nginx
location = /ws/relay {
    proxy_pass http://127.0.0.1:21119;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_buffering off;
    proxy_read_timeout 120s;
    proxy_send_timeout 120s;
}
```

`relay_health.endpoints[].url` must use that hostname and exact path. An HTTPS
homepage, ICMP ping, or a certificate ignored by a browser warning is not an
acceptable health endpoint.

## Internal Relay `/ws/telemetry`

Relay Quality uses a separate exact path. Keep it off the general public
allow-list, permit only HBBS source networks, disable caching, and preserve the
Starry authentication headers. For example:

```nginx
location = /ws/telemetry {
    allow 10.20.0.0/16;
    deny all;
    proxy_pass http://127.0.0.1:21119;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_buffering off;
    proxy_read_timeout 15s;
    proxy_send_timeout 15s;
}
```

Use a dedicated internal hostname and mTLS where possible. HBBR still requires
the secret-file HMAC, so an accidentally reachable endpoint does not disclose
load to an unauthenticated WSS client. Never put the secret in a query string,
header literal, Nginx configuration, or access log. See
[Relay Telemetry Security and Operations](../Relay-Telemetry-Operations.md).

## Certificates

For each hostname:

1. confirm public DNS resolves to the intended ingress;
2. obtain a certificate whose SAN covers the exact name;
3. configure the full chain and private key with restricted permissions;
4. validate configuration before reload; and
5. test with normal hostname verification.

Inspect without bypassing verification:

```sh
openssl s_client \
  -connect id.example.com:443 \
  -servername id.example.com \
  -verify_return_error </dev/null
```

Do not use `curl -k`, `verify none`, raw IP substitution, or a private CA that
the clients and Starry HBBS do not trust.

## Validate Nginx and Upgrade

```sh
sudo nginx -T
sudo nginx -t
sudo systemctl reload nginx
```

Then make a transport-only Upgrade probe:

```sh
curl --http1.1 --include --max-time 5 \
  -H 'Connection: Upgrade' \
  -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' \
  -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  https://id.example.com/ws/id
```

Repeat against every `https://relay-N.example.com/ws/relay`. An HTTP 101 proves
the HTTP/TLS Upgrade path only. It does not prove RustDesk registration,
signalling, Relay UUID correlation, or desktop data.

## Firewall boundary

- Public: `443/TCP`; native ports required by clients.
- Private/local: `21118/TCP`, `21119/TCP`, API backend port.
- Never public-proxy the HBBS management command interface.
- Preserve native `21116`/`21117` paths when clients can turn WebSocket off.

## Enable WebSocket safely

1. Deploy all Nginx paths with `websocket_signal.enabled: false`.
2. Validate DNS, certificates, exact Upgrade, and backend reachability.
3. Configure one endpoint for every `relay_servers` item.
4. Reload schema v2 and confirm acceptance.
5. Set `enabled: true`, reload, and inspect `websocket-status`.
6. Test WSS-to-WSS and both mixed directions with real clients.

If one required ingress cannot be deployed, leave WebSocket Signal disabled;
native operation can continue independently.

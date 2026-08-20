# GEO rules: advanced

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-GEO-Rules-Advanced)

Use this page after a country-level rule and real Relay session have passed.
Advanced rules add precedence, direction, ASN/ISP conditions, transport-aware
eligibility, and intentional fallback. Complexity is useful only when each
branch can be tested with known public addresses.

## Grammar and precedence

The expression grammar is:

```text
expression := and-term ("/" and-term)*
and-term   := primary ("+" primary)*
primary    := predicate | "(" expression ")"
predicate  := "*" | two-letter-country | field:value
```

`+` binds more tightly than `/`. Therefore:

```text
CN/JP+asn:AS2516
```

means `CN OR (JP AND AS2516)`, not `(CN OR JP) AND AS2516`. Add parentheses
when a human reviewer could interpret it differently.

Quoted values may contain `/`, `+`, `(`, or `)`. Single and double quotes are
accepted, and a backslash escapes the next character inside a quoted value:

```yaml
client_a: "city:'A/B'+isp:\"Example (Transit)\""
```

The YAML layer and expression layer both process quoting, so prefer simple
single-quoted field values inside a double-quoted YAML scalar.

## Exact matching versus substring matching

- `continent`, `country`, `subdivision`, `city`, and their aliases compare
  case-insensitively for exact equality.
- `geoname` and `asn` compare positive numeric values.
- `isp`/`asn_org` performs a case-insensitive substring search against the ASN
  organisation name.

An ISP label is provider data, not a contractual identity. Audit actual MMDB
records before relying on a substring for security or billing decisions.

## Direction-sensitive policies

With `symmetric: false`, only the declared A-to-B orientation matches:

```yaml
- name: China Telecom initiator to US peer
  symmetric: false
  match:
    client_a: "country:CN+asn:AS4134"
    client_b: country:US
  relays:
    - relay-us-1.example.com:21117
```

Use this only when the client roles are meaningful and verified. For geographic
proximity policies, `symmetric: true` is normally safer because either endpoint
may initiate the connection.

## Nested multi-factor example

```yaml
- name: Selected East Asia access networks
  symmetric: true
  match:
    client_a: "((city:Shanghai+isp:'China Telecom')/(city:Seoul+isp:KT))+continent:AS"
    client_b: "country:CN/country:JP/country:KR"
  relays:
    - relay-asia-1.example.com:21117
    - relay-asia-2.example.com:21117
```

This requires the A-side expression to match either the Shanghai/provider pair
or the Seoul/provider pair, and also requires Asia. The B-side must be one of
three countries. Because the rule is symmetric, Starry also evaluates those
roles in reverse.

## Rule and Relay ordering

Consider:

```yaml
rules:
  - name: Specific paid route
    match:
      client_a: "country:CN+asn:AS4134"
      client_b: country:US
    relays:
      - relay-premium.example.com:21117
      - relay-us-1.example.com:21117

  - name: General CN to US
    match:
      client_a: country:CN
      client_b: country:US
    relays:
      - relay-us-1.example.com:21117

  - name: Final catch-all
    match:
      client_a: "*"
      client_b: "*"
    relays:
      - relay-asia-2.example.com:21117
```

The important behaviours are:

- rules are ordered, not scored by specificity;
- Relay lists are strict priority, not round-robin weights;
- if a matching rule has no eligible Relay, the next rule may still select;
- if no rule selects, HBBS uses official round-robin behaviour over the
  transport-eligible list; and
- a catch-all with an eligible Relay prevents later rules from being reached.

## Transport-aware eligibility

Patch v1.1.0 filters Relays before Geo ordering:

| Requirement | Eligible Relay evidence |
| --- | --- |
| `native` | Reported online by the official HBBS/HBBR native mechanism. |
| `wss` | Current generation has completed and its certificate-verified `/ws/relay` endpoint is healthy. |
| `mixed` | Both native-online and WSS-healthy for the same Relay. |

This means the same two IPs can select different Relays for different
transports. Preview all paths that your clients will use:

Call authenticated `POST /control/v1/allocations:simulate` once for each of
`native`, `wss`, and `mixed`, keeping both addresses and expected generation
constant.

For `wss` and `mixed`, inspect health first:

Use authenticated `GET /control/v1/status` to inspect WSS health first.

An empty selection is safer than assigning a Relay that cannot satisfy the
requested transport. Do not replace WSS certificate validation with ping,
plain HTTPS, or a disabled TLS check.

## Same-NAT and missing-data cases

Two clients behind the same public NAT appear with the same public IP. Test by
passing the same address twice. A symmetric rule behaves predictably; a
direction-sensitive rule cannot manufacture distinct Geo facts.

If an address has no record in a required MMDB, predicates for that fact are
false. `*` still matches. Use an explicit catch-all when that is the desired
policy, and monitor whether missing data is unexpectedly frequent.

Private, loopback, CGNAT, or proxy addresses are not a substitute for the public
addresses HBBS observes. For WebSocket clients, configure `trusted_proxies`
precisely so forwarded addresses are accepted only from the real reverse proxy.

## Change-control method

For a non-trivial rule update:

1. save the previous config and its digest;
2. list a test matrix of IP pair, direction, transport, expected rule, primary
   Relay, and failover Relay;
3. change one policy dimension at a time;
4. run the authenticated Control Agent plan/apply or runtime-reload operation and stop if validation is rejected;
5. run `test-geo` for every matrix row;
6. verify at least one real session for every transport class in use;
7. simulate first-Relay failure and recovery; and
8. keep logs and the exact configuration version with the result.

[`config.geo-advanced.yaml`](https://github.com/q1ngyang/rustdesk-server-starry/blob/main/config/config.geo-advanced.yaml)
is a complete starting example. Replace every host, data URL, and policy value.

## Anti-patterns

- Do not use Geo rules as an access-control system; they allocate Relay only.
- Do not encode dozens of city names before auditing the MMDB language values.
- Do not place an always-available catch-all first.
- Do not expect Relay list order to distribute load evenly.
- Do not make an API domain, HBBS domain, and HBBR endpoint interchangeable.
- Do not consider `test-geo`, HTTP 101, or Compose validation a completed
  remote-control acceptance test.

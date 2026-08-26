# GEO rules: basics

**English** | [简体中文](https://github.com/q1ngyang/rustdesk-server-starry/wiki/ZH-CN-GEO-Rules-Basics)

Geo rules let HBBS choose an ordered HBBR destination from the public addresses
it observes for both clients. They do not redirect API traffic, change the
client's displayed location, or modify HBBR. Start with one broad rule and two
Relays; add detail only after the decision is observable and real sessions work.

## The decision in one minute

For a connection that needs a Relay, Starry:

1. determines the transport requirement (`native`, `wss`, or `mixed`);
2. builds the currently eligible Relay list;
3. looks up Geo facts for both observed public client addresses;
4. evaluates rules from top to bottom;
5. within the first usable match, chooses the first eligible Relay in the
   rule's ordered list; and
6. falls back to the official round-robin choice among eligible Relays when no
   Geo rule can select one.

If a rule matches but all Relays in that rule are unavailable, evaluation can
continue to a later rule. Put specific policies before broad catch-alls.

## 1. Declare the Relay pool

```yaml
version: 1

relay_servers:
  - relay-asia-1.example.com:21117
  - relay-asia-2.example.com:21117
  - relay-us-1.example.com:21117
```

Every Relay later used by a rule must appear in this list. The HBBR instances
may be on the centre host or separate nodes, but each address must be reachable
by clients and must use the same HBBS public key.

## 2. Provide only the MMDB data you need

A country-only rule can use a Country or City MMDB:

```yaml
mmdb:
  update_interval_hours: 168
  update_on_start: true
  force_update: false
  download_timeout_seconds: 600
  minimum_bytes: 65536
  country:
    path: mmdb/GeoLite2-Country.mmdb
    url: https://downloads.example.com/GeoLite2-Country.mmdb
  city:
    path: mmdb/GeoLite2-City.mmdb
    url: ""
  asn:
    path: mmdb/GeoLite2-ASN.mmdb
    url: ""
```

Replace the example URL with a lawful, trusted source, or leave it empty and
place the file at the configured path yourself. No database is built into the
image. A rule that needs city facts requires the City database; ASN and ISP
rules require the ASN database.

## 3. Write the first rule

```yaml
geo:
  enabled: true
  rules:
    - name: East Asia to Asia Relay
      symmetric: true
      match:
        client_a: CN/JP/KR/TW
        client_b: "*"
      relays:
        - relay-asia-1.example.com:21117
        - relay-asia-2.example.com:21117

    - name: Default
      symmetric: true
      match:
        client_a: "*"
        client_b: "*"
      relays:
        - relay-us-1.example.com:21117
```

This says:

- when either client is in `CN`, `JP`, `KR`, or `TW`, prefer
  `relay-asia-1`, then fail over to `relay-asia-2`;
- otherwise use `relay-us-1` while it is eligible.

`symmetric: true` is important here: the client that starts the connection is
not a stable way to define geography. The rule matches when the regional
client appears as A **or** B.

## Expression essentials

| Syntax | Meaning | Example |
| --- | --- | --- |
| `*` | Any address, including one with missing MMDB facts | `client_b: "*"` |
| `/` | OR | `CN/JP/KR` |
| `+` | AND; evaluated before OR | `country:CN+asn:AS4134` |
| `( )` | Grouping | `(CN/JP)+continent:AS` |
| `XX` | Bare two-letter country code | `CN` equals `country:CN` |
| `field:value` | Match one explicit fact | `city:Shanghai` |

Quote YAML strings containing `*`, and quote expression values when they may
contain punctuation. Geo comparisons are case-insensitive. ISP matching is a
case-insensitive substring match; the other textual predicates are exact
case-insensitive matches against MMDB values.

Supported fields:

| Field | Example | Database |
| --- | --- | --- |
| `continent` | `continent:AS` | Country or City |
| `country` | `country:US` | Country or City |
| `subdivision`, `region` | `region:CA` | City |
| `city` | `city:Shanghai` | City |
| `geoname`, `city_id` | `geoname:1796236` | City |
| `asn` | `asn:AS4134` or `asn:4134` | ASN |
| `isp`, `asn_org` | `isp:'China Telecom'` | ASN |

GeoNames IDs and ASNs must be positive integers. City and subdivision names
depend on the names present in the selected MMDB. Codes or numeric IDs are
usually less ambiguous than translated names.

## 4. Reload and inspect

For initial commissioning with the supplied Compose example, restart HBBS.
Use the authenticated Control Agent for later managed reloads:

```sh
docker restart rustdesk-starry-hbbs
docker logs --tail 100 rustdesk-starry-hbbs
```

Look for an accepted Starry configuration, loaded Geo rule count, readable
database paths, and no missing-database warning for fields used by the rules.

Confirm the Relay pool through authenticated Control Agent
`GET /control/v1/relays`.

The local control port is loopback-only, requires the independent token, and
must never be exposed or reverse-proxied.

## 5. Preview with real public addresses

Call authenticated `POST /control/v1/allocations:simulate` with the two public
addresses, `transport: native`, the expected generation, and `explain: true`.

Replace the documentation-only addresses with the public source addresses that
HBBS actually observes. When both devices share one NAT, use that public
address for both arguments. The output is the selected Relay in Rust's debug
string form, or `""` if none is eligible.

Allocation simulation is a decision preview. It does not register clients, open HBBR, prove
that DNS or a firewall works, or measure latency.

## 6. Complete a real test

Use two RustDesk clients and leave their Relay Server fields empty. Confirm:

1. both clients register with this HBBS and use the expected public key;
2. a forced-Relay or naturally relayed session reaches the expected HBBR;
3. desktop control, input, and an appropriate sustained transfer work; and
4. stopping the first Relay causes the next ordered Relay to be selected only
   after HBBS observes it as unavailable.

Record both client timestamps and the matching HBBS/HBBR logs. Continue with
[Operations and Verification](https://github.com/q1ngyang/rustdesk-server-starry/wiki/Operations-and-Verification)
before calling the policy production-ready.

## Common beginner mistakes

| Symptom | Likely cause |
| --- | --- |
| The same Relay is always used | This may be correct: Relay order is strict priority, not load balancing. |
| The catch-all hides a regional rule | The catch-all appears too early. Move it last. |
| Country works but city never matches | The City MMDB is missing, unreadable, or does not contain the expected name. |
| Server policy seems ignored | A client has a non-empty Relay Server field. |
| An MMDB URL downloads an HTML page | The provider requires authentication or redirects to a licence page; use an authorised direct file source. |
| `test-geo` works but remote control does not | Geo selection was proved, but HBBR reachability/session flow was not. Test the next layer. |

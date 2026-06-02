# realm-trust.toml

`/etc/easynet/realm-trust.toml` is the daemon-side trust anchor loaded by
[`src/services/realm_trust_anchor.rs`](/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/src/services/realm_trust_anchor.rs:1).
It is the admission whitelist for caller URAs and their Ed25519 public keys.

## File format

```toml
[[trusted_agent]]
agent_ura = "easynet:///r/acme/agent/backend"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "backend"
added_at_unix_ms = 1714492800000

[[trusted_agent]]
agent_ura = "easynet:///r/acme/agent/device-01"
public_key_b64 = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB="
role = "device"
added_at_unix_ms = 1714492801234
```

## Semantics

- `agent_ura` is the canonical URA key. It must be unique within the file.
- `public_key_b64` is the base64-encoded 32-byte Ed25519 verifying key.
- `role` is one of `device`, `backend`, or `hub`.
- `added_at_unix_ms` is an audit timestamp; admission does not apply age policy.

## Loader behavior

- Missing file is treated as an empty trust set.
- Duplicate `agent_ura` entries are rejected at load time.
- Empty trust set is safe for staging, but it will reject every external caller.

## Write path

The intended write path is the daemon ability
`<self>.register_device_pubkey`, which:

1. validates the target URA belongs to the daemon's realm,
2. appends the entry atomically (`*.tmp` + fsync + rename),
3. republishes the in-memory trust-anchor cell so subsequent admissions see the new key immediately.

Manual editing is still supported for recovery work. After editing the file
out-of-band, reload the daemon's in-memory view with:

```bash
sudo kill -HUP $(pidof easynet-daemon)
```

If the edit also accompanies `[daemon.federated_peers]` or `[daemon.quota]`
changes, the same SIGHUP pass reloads those cells too. Use a full
`systemctl restart easynet-daemon` for TLS-certificate rotation or
listener/identity config changes.

## Production expectation

- `backend` should appear in the file before the production canary.
- Every paired device should have one stable `agent_ura` entry.
- Cross-realm registration is rejected; each daemon only writes and admits within its own `realm`.

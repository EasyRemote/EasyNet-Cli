# daemon-config.toml

`easynet-daemon` reads `~/.easynet/daemon-config.toml` at boot to decide
which transport-plane role it plays and where it listens. The file is
parsed by [`src/persistence/daemon_config.rs`](/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/src/persistence/daemon_config.rs:1).

## Modes

| mode | Intended host | Required listeners | Required outbound dial |
|---|---|---|---|
| `device` | end-user device behind NAT | UDS only (`~/.easynet/daemon.sock` by default) | `hub_endpoint` |
| `hub` | public rendezvous node | UDS always; optional public `listen_tcp` with TLS | none |
| `both` | production hub colocated with backend | same as `hub` | none |

## Invariants

1. `mode = "device"` must not set `listen_tcp`.
2. If `listen_tcp` is set, both `tls_cert_pem` and `tls_key_pem` must be set.
3. The daemon UDS is bound with mode `0600`; local backend/CLI clients are expected to connect through this path instead of a public TCP socket.
4. `device` mode requires `hub_endpoint`, because `<self>.session` is maintained as an outbound long-lived bidi to the hub.
5. Listener-shaping config is read once at boot. `mode`, `listen_tcp`, TLS paths,
   `hub_endpoint`, `realm`, `uds_path`, and `ledger_dir` changes take effect on
   restart.
6. SIGHUP hot-reloads only the daemon-config cells designed for live
   replacement: `[daemon.federated_peers]` and `[daemon.quota]`.

## Example: device

```toml
[daemon]
mode = "device"
realm = "acme"
hub_endpoint = "https://hub.acme.example:50051"
uds_path = "~/.easynet/daemon.sock"
```

## Example: hub / both

```toml
[daemon]
mode = "both"
realm = "acme"
listen_tcp = "0.0.0.0:50051"
tls_cert_pem = "/etc/letsencrypt/live/hub.acme.example/fullchain.pem"
tls_key_pem = "/etc/letsencrypt/live/hub.acme.example/privkey.pem"
uds_path = "~/.easynet/daemon.sock"
```

## Operator notes

- `control.sock` and `daemon.sock` are distinct UDS surfaces. `control.sock`
  is the legacy CLI IPC framing; `daemon.sock` is gRPC `Invocation`.
- `realm-trust.toml` is a separate file from `daemon-config.toml`. It is
  loaded from `/etc/easynet/realm-trust.toml` by default and governs
  admission, not listener binding.
- Current pairing-flow updates through `<self>.register_device_pubkey`
  republish the in-memory trust anchor immediately. Manual edits to
  `realm-trust.toml` can be reloaded with `sudo kill -HUP $(pidof easynet-daemon)`.
  The same SIGHUP pass also reloads `[daemon.federated_peers]` and
  `[daemon.quota]` from `daemon-config.toml`. Restart is still required if you
  are rotating config that shapes listeners, identity, ledger path, hub endpoint,
  realm, or cert files.
- The new `easynet-daemon` binary contains no flag-selectable "v1 transport"
  path. Rollback means reinstalling an older daemon binary, not toggling a config
  switch inside the current one.
- For certificate renewal, use:

```bash
sudo certbot renew
sudo systemctl restart easynet-daemon
```

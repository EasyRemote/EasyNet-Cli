# Control Plane v1 — Local IPC Wire Protocol

> Plan v10.5 R1 §"本机 IPC 接口" pin. The on-host control-plane
> protocol every Client FFI binding speaks to `easynet-daemon`.

## 1. Transport

- Linux / macOS: Unix Domain Socket at
  `~/.easynet/control.sock` with mode `0600`. Filesystem
  permission is the auth boundary; no bearer token.
- Windows: Named Pipe at `\\.\pipe\easynet-<uid>` with an ACL
  limiting access to the current user SID. (Implementation
  pending; v1 ships Unix only.)

## 2. Discovery

`easynet-daemon` writes `~/.easynet/control.json` at startup:

```json
{
  "socket_path": "/Users/<user>/.easynet/control.sock",
  "pid": 12345,
  "daemon_version": "1.17.1",
  "supported_ipc_versions": { "min": 1, "max": 1 },
  "capability_flags": ["ability_invoke", "ability_subscribe", "loopback", "misfire_policy_v1"]
}
```

A Client FFI library:
1. Reads `control.json`.
2. Computes the overlap between its own `IpcVersionRange` and the
   daemon's `supported_ipc_versions`.
3. If empty, returns `ERR_VERSION_INCOMPATIBLE` immediately
   (early failure beats a tunneled wire-format mismatch).
4. Connects to the discovered `socket_path` (or `pipe_name`).

## 3. Frame layout

Every frame on the wire is:

```
[4 bytes little-endian length][JSON UTF-8 payload]
```

`tokio_util::codec::LengthDelimitedCodec` provides this on the
Rust side; the file `scripts/control-smoke.sh` shows how to
compose the same shape from a `python3` client.

## 4. Frame types

### Client → daemon (`IncomingFrame`)

```jsonc
// RPC ability call
{ "type": "Invoke",
  "request_id": "<caller-chosen string>",
  "ability": "system.session.list",
  "args": { "include_terminated": true } }

// Streaming ability subscription
{ "type": "Subscribe",
  "subscription_id": "<caller-chosen string>",
  "ability": "system.session.attach",
  "args": { "session_id": "...", "since_seq": 0 } }

// Cancel an active subscription
{ "type": "Cancel",
  "subscription_id": "<the id from a prior Subscribe>" }
```

### Daemon → client (`OutgoingFrame`)

```jsonc
// Successful RPC response
{ "type": "Result",
  "request_id": "<echo of Invoke.request_id>",
  "value": { ... } }

// One frame in a stream
{ "type": "Frame",
  "subscription_id": "<echo of Subscribe.subscription_id>",
  "frame": { ... } }

// Stream terminated (success or otherwise)
{ "type": "Terminal",
  "subscription_id": "...",
  "reason": "completed" | "cancelled" | "error" }

// Error envelope (for Invoke OR Subscribe)
{ "type": "Error",
  "request_id": "..." | null,
  "subscription_id": "..." | null,
  "code": "ability_failed" | "protocol" | "not_found" | ...,
  "message": "human-readable diagnostic" }
```

`request_id` and `subscription_id` are returned verbatim from the
matching request frame. Pinned by the Rust unit test
`handle_preserves_subscription_id_for_subscribe_and_cancel`.

## 5. Error codes (`code` field)

| code             | meaning                                                |
|------------------|--------------------------------------------------------|
| `protocol`       | malformed frame; connection stays open                 |
| `not_found`      | unknown ability or unknown id                          |
| `ability_failed` | ability handler returned an error                     |
| `version`        | post-handshake version mismatch                        |
| `internal`       | daemon-side bug; consult logs                          |

A `protocol` error does not close the connection — the daemon
keeps reading subsequent frames. Any other error may close the
connection at the daemon's discretion.

## 6. Debugging

```bash
# socat to the UDS, send a Ping
socat - UNIX-CONNECT:$HOME/.easynet/control.sock

# scripts/control-smoke.sh — composes the framing in python
scripts/control-smoke.sh
```

The smoke script is the canonical reference for the wire format
when a Client binding hits a serialisation bug.

## 7. v2 deltas (out of scope)

- Explicit handshake before the first request (today every
  connection is treated as v1).
- Proto-encoded payloads replacing JSON.
- Per-frame timestamp for replay debugging.
- Streaming-back-pressure handshake (today the daemon writes as
  fast as the OS buffer accepts).

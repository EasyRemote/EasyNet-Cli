# Control Plane v1 — Local IPC Wire Protocol

> Plan v10.5 R1 §"本机 IPC 接口" pin. The on-host control-plane
> protocol used for local daemon boot/status and diagnostics.
> Product ability calls use daemon `Invocation` over `daemon.sock`.

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
  "capability_flags": ["boot_status", "control_diagnostics"]
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
// Boot/status stream subscription
{ "type": "subscribe",
  "subscription_id": "<caller-chosen string>",
  "ability": "system.watch_boot",
  "args": {} }

// Cancel an active boot/status subscription
{ "type": "cancel",
  "subscription_id": "<the id from a prior Subscribe>" }
```

### Daemon → client (`OutgoingFrame`)

```jsonc
// One frame in a stream
{ "type": "frame",
  "subscription_id": "<echo of Subscribe.subscription_id>",
  "frame": { "type": "ready" } }

// Stream terminated (success or otherwise)
{ "type": "terminal",
  "subscription_id": "...",
  "reason": "completed" | "cancelled" | "error" }

// Error envelope for malformed, unknown, or unsupported control frames
{ "type": "error",
  "subscription_id": "..." | null,
  "code": "protocol" | "not_found" | "version" | "shutting_down",
  "message": "human-readable diagnostic" }
```

`subscription_id` is returned verbatim from the matching boot/status
request frame. Retired product frame discriminators such as `invoke`,
`open_bidi`, `send_bidi`, and `close_bidi` are no longer part of the
schema; serde decode rejects them as `protocol` errors before handler
dispatch.

## 5. Error codes (`code` field)

| code             | meaning                                                |
|------------------|--------------------------------------------------------|
| `protocol`       | malformed frame; connection stays open                 |
| `not_found`      | unknown control subscription or id                     |
| `version`        | post-handshake version mismatch                        |
| `shutting_down`  | daemon is terminating                                  |

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

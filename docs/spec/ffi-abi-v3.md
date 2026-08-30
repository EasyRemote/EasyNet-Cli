# FFI ABI v3 — `libeasynet_cli` C ABI

Version-stable C ABI exposed by `libeasynet_cli.{so,dylib,dll,a}`.
Client bindings in Go, Python, Node, Swift, Rust, Java, and other
languages consume this surface.

## 1. Versioning

`runtime_abi_version() -> u32` is the runtime source of truth.
Every breaking change to a function signature, struct shape, exported
symbol set, or return-code semantic increments this version. The
checked-in `include/easynet_cli.h` header is the binding-facing
contract; CI asserts the header, ABI version, exported symbol set, and
error-code table stay aligned so a hand-edit cannot drift the contract
silently.

ABI v3 is the complete Invocation-only ABI. The historical
`easynet_ability_*` ability+args symbols are not exported.

## 2. Function Families

### 2.1 Lifecycle

```c
uint32_t runtime_abi_version(void);
const char* easynet_last_error(void);
void runtime_string_free(char* s);
```

Returns the ABI version number. Callers SHOULD assert the value
matches what they were built against.

```c
int32_t  runtime_init(const char* control_json_path, RuntimeHandle* out);
int32_t  runtime_shutdown(RuntimeHandle handle);
```

`runtime_init` resolves the daemon control descriptor, validates IPC
version overlap, and returns a client-session handle. This handle names
a daemon IPC session; it is not a daemon process lifecycle handle.

```c
int32_t runtime_host_start(const char* config_json, uint64_t* out_daemon_handle);
int32_t runtime_host_stop(uint64_t daemon_handle);
int32_t runtime_host_status(uint64_t daemon_handle, char** out_status_json);
int32_t runtime_host_invocation_endpoint(uint64_t daemon_handle, char** out_endpoint);
int32_t runtime_host_open_client(uint64_t daemon_handle, RuntimeHandle* out);
```

Daemon lifecycle handles are process/status handles. They are separate
from `RuntimeHandle` values returned by `runtime_init`. Bindings that
start or attach to a daemon through `runtime_host_start` should call
`runtime_host_open_client` to get an Invocation-capable
`RuntimeHandle` for `runtime_invocation_*`; they do not need to guess
or rediscover a control descriptor path.

`config_json` is an explicit object:

```json
{
  "mode": "edge",
  "runtime_instance_id": "dev-a",
  "runtime_bin": "/path/to/easynet-daemon",
  "log_path": "/path/to/easynet-daemon.log",
  "detached": true,
  "env": {"KEY": "VALUE"}
}
```

When attaching to an already-live daemon, both the control endpoint and
the Invocation endpoint must accept connections. A control-only daemon
is reported as down for lifecycle attach purposes because product calls
cannot succeed through that process.

`runtime_host_start` returns success only after both `control.sock`
and `daemon.sock` are accepting connections, or after attaching to an
already-live daemon with both endpoints accepting. A returned daemon
handle is therefore immediately usable with
`runtime_host_open_client`.

### 2.2 Complete Invocation Dispatch

```c
int32_t runtime_invocation_invoke(
    RuntimeHandle handle,
    const char* invocation_json,
    char** out_receipt_json
);

int32_t runtime_invocation_stream_open(
    RuntimeHandle handle,
    const char* invocation_json,
    RuntimeInvocationStreamCallback on_chunk,
    void* user_data,
    uint64_t* out_stream_id
);

int32_t runtime_invocation_stream_cancel(RuntimeHandle handle, uint64_t stream_id);
```

`invocation_json` carries the complete Axon Invocation tuple:

ABI v3 keeps the legacy unversioned `ability` field for compatibility. New SDK
schema work MUST canonicalize this shape into the descriptor-ref v4 projection
through Axon helpers before daemon submission. Even in v3 examples, URA fields
MUST pass Axon `parse_ura`; do not use flat `agent/<id>` or nested
`device/<id>/ability/<name>` shapes.

```json
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/agent/device.dev-a.runtime-health",
  "ability": "observe.health",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {}
}
```

For non-JSON payloads, callers pass `arguments_base64` plus
`content_type` instead of `args`. Optional `metadata` and
`caller_signature` fields are forwarded to the daemon Invocation
transport.

### 2.3 InvokeBidi Dispatch

```c
int32_t runtime_invocation_bidi_open(
    RuntimeHandle handle,
    const char* invocation_json,
    RuntimeInvocationBidiCallback on_frame,
    void* user_data,
    uint64_t* out_bidi_id
);

int32_t runtime_invocation_bidi_send(
    RuntimeHandle handle,
    uint64_t bidi_id,
    const char* frame_json
);

int32_t runtime_invocation_bidi_close(RuntimeHandle handle, uint64_t bidi_id);
int32_t runtime_invocation_bidi_cancel(RuntimeHandle handle, uint64_t bidi_id);
```

`runtime_invocation_bidi_open` uses the same complete Invocation JSON
and additionally requires:

```json
"bidi_streams": [
  {"stream_id": 1, "content_type": "application/octet-stream", "ordering": "STRICT"}
]
```

Stream and bidi ids are scoped to the `RuntimeHandle` that opened them.
A different handle cannot send, close, or cancel another handle's active
stream/session.

## 3. Error Code Table

| code | name                       | meaning                                          |
|------|----------------------------|--------------------------------------------------|
| 0    | `RUNTIME_OK`               | success                                          |
| 1    | `ERR_GENERIC`              | generic / unclassified failure                   |
| 2    | `ERR_NULL_POINTER`         | required pointer argument was null               |
| 3    | `ERR_INVALID_UTF8`         | C string argument was not valid UTF-8            |
| 4    | `ERR_INVALID_HANDLE`       | handle was never issued or already released      |
| 5    | `ERR_NOT_INITIALIZED`      | library has not been initialized                 |
| 6    | `ERR_ALREADY_INIT`         | duplicate initialization                         |
| 7    | `ERR_DAEMON_DOWN`          | daemon endpoint cannot be reached                |
| 8    | `ERR_VERSION_INCOMPATIBLE` | IPC version overlap empty                        |
| 9    | `ERR_ABILITY_FAILED`       | ability/admission execution failure              |
| 10   | `ERR_NOT_IMPLEMENTED`      | feature-gated symbol in a build without support  |
| 11   | `ERR_INVALID_ARG`          | malformed JSON, missing fields, invalid URA/base64 |
| 12   | `ERR_PERMISSION_DENIED`    | daemon/admission rejected authority              |
| 13   | `ERR_NOT_FOUND`            | requested resource or ability not found          |
| 14   | `ERR_CANCELLED`            | operation/session cancelled or already closed    |
| 15   | `ERR_PROTOCOL`             | malformed daemon protocol status/response        |
| 16   | `ERR_TIMEOUT`              | operation exceeded deadline                      |

Daemon gRPC status is preserved at the FFI boundary:

- `Unavailable` and transport connect failures map to `ERR_DAEMON_DOWN`.
- `InvalidArgument` maps to `ERR_INVALID_ARG`.
- `PermissionDenied` / `Unauthenticated` map to `ERR_PERMISSION_DENIED`.
- `NotFound` maps to `ERR_NOT_FOUND`.
- `Cancelled` maps to `ERR_CANCELLED`.
- `DeadlineExceeded` maps to `ERR_TIMEOUT`.
- `Unknown` / `Internal` / `DataLoss` map to `ERR_PROTOCOL`.
- Other non-transport daemon statuses map to `ERR_ABILITY_FAILED`.

## 4. Threading And Reentrancy

- The library owns an internal tokio runtime. C ABI calls from any
  thread block the calling thread until the daemon operation returns.
- Stream and bidi callbacks are invoked on a library-owned callback
  dispatcher thread created per opened stream/session. They are not
  invoked on the tokio I/O runtime thread. Callbacks should still
  avoid blocking indefinitely; copy the frame and signal the consumer
  through a queue.
- All handles are process-local integers. A stream/bidi id is valid
  only with the `RuntimeHandle` that opened it.

## 5. Conformance Checklist

A new Client binding must:

- [ ] Call `runtime_abi_version()` at startup; abort if it does not
      match the expected value.
- [ ] Free every `char*` returned by an `easynet_*` function via
      `runtime_string_free`.
- [ ] Cancel or close every active stream/bidi session before calling
      `runtime_shutdown`.
- [ ] Treat any non-zero return code from a wrapper as a hard error;
      consult `easynet_last_error` for diagnostics.

## 6. Out Of Scope

- async C ABI. Every call is synchronous today.
- one-method-per-ability ABI.
- ability+args ABI.
- JSON-control product Invoke/Subscribe/OpenBidi.
- Axon runtime lifecycle. This ABI starts only `easynet-daemon`.

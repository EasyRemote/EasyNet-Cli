# FFI ABI v4 - `libeasynet_cli` C ABI

Version-stable C ABI exposed by `libeasynet_cli.{so,dylib,dll,a}`.
Client bindings in Go, Python, Node, Swift, Rust, Java, and other languages
consume this surface.

ABI v4 is the Daemon SDK Runtime Core projection. It keeps the ABI v3 complete
Invocation dispatch surface and adds feature discovery, explicit daemon attach
and detach, endpoint discovery, runtime health, and the public
Draft -> Prepared -> Signed -> Submitted invocation state-machine handles.

The checked-in `include/easynet_cli.h` header is the binding-facing contract.
Rust sources under `src/ffi/` own behavior. Repository checks assert that the
header, ABI version, exported symbol set, error-code table, and this document
stay aligned.

## 1. Versioning

```c
uint32_t easynet_abi_version(void);
int32_t  easynet_feature_discovery(char** out_features_json);
const char* easynet_last_error(void);
void easynet_string_free(char* s);
```

`easynet_abi_version()` returns `4`. Bindings MUST check it at library load and
reject incompatible libraries before opening daemon traffic.

`easynet_feature_discovery` returns caller-owned JSON. The returned `char*` MUST
be released with `easynet_string_free`.

## 2. Function Families

### 2.1 SDK Session

```c
int32_t easynet_init(const char* control_json_path, EasynetHandle* out);
int32_t easynet_shutdown(EasynetHandle handle);
```

`EasynetHandle` names an Invocation-capable daemon IPC session. It is not a
daemon process lifecycle handle. Shutdown releases the session and cancels local
stream/bidi state owned by that session.

### 2.2 Daemon Lifecycle

```c
int32_t easynet_daemon_start(const char* config_json, EasynetDaemonHandle* out);
int32_t easynet_daemon_attach(const char* options_json, EasynetDaemonHandle* out);
int32_t easynet_daemon_discover(const char* options_json, char** out_discovery_json);
int32_t easynet_daemon_status(EasynetDaemonHandle handle, char** out_status_json);
int32_t easynet_daemon_endpoints(EasynetDaemonHandle handle, char** out_endpoints_json);
int32_t easynet_daemon_invocation_endpoint(EasynetDaemonHandle handle, char** out_endpoint);
int32_t easynet_daemon_open_client(EasynetDaemonHandle daemon_handle, EasynetHandle* out);
int32_t easynet_daemon_detach(EasynetDaemonHandle handle);
int32_t easynet_daemon_stop(EasynetDaemonHandle handle);
```

`start` and `attach` return success only when the Invocation endpoint is ready.
`detach` releases local ownership without stopping an already-running daemon.
`stop` is the lifecycle terminal operation for a controllable daemon handle.

### 2.3 Runtime Health

```c
int32_t easynet_runtime_health(EasynetHandle handle, char** out_health_json);
```

Health JSON separates API/session liveness from Invocation runtime readiness.
Bindings MUST NOT treat a live control socket as sufficient runtime health.

### 2.4 Invocation Dispatch

```c
int32_t easynet_invocation_invoke(
    EasynetHandle handle,
    const char* invocation_json,
    char** out_receipt_json
);
```

`invocation_json` carries the complete Invocation tuple:

```json
{
  "caller_ura": "easynet:///r/example/agent/alice.sdk",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/ability/device.dev-a.observe.health@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {}
}
```

ABI v4 accepts the v4 descriptor-ref projection. Legacy v3 `ability` JSON is
only an adapter input where explicitly documented; SDK facades must expose the
descriptor-ref form.

### 2.5 Invocation Builder Handles

```c
int32_t easynet_invocation_builder_new(EasynetInvocationBuilderId* out_builder_id);
int32_t easynet_invocation_builder_set_caller(EasynetInvocationBuilderId builder_id, const char* caller_ura);
int32_t easynet_invocation_builder_set_callee(EasynetInvocationBuilderId builder_id, const char* callee_ura);
int32_t easynet_invocation_builder_set_descriptor_ref(EasynetInvocationBuilderId builder_id, const char* descriptor_ref);
int32_t easynet_invocation_builder_set_subject(EasynetInvocationBuilderId builder_id, const char* subject_ura);
int32_t easynet_invocation_builder_set_nonce_base64(EasynetInvocationBuilderId builder_id, const char* nonce_base64);
int32_t easynet_invocation_builder_set_causal_context_json(EasynetInvocationBuilderId builder_id, const char* causal_context_json);
int32_t easynet_invocation_builder_set_args_json(EasynetInvocationBuilderId builder_id, const char* args_json);
int32_t easynet_invocation_builder_set_arguments_base64(EasynetInvocationBuilderId builder_id, const char* arguments_base64, const char* content_type);
int32_t easynet_invocation_builder_set_metadata_json(EasynetInvocationBuilderId builder_id, const char* metadata_json);
int32_t easynet_invocation_builder_set_timeout_seconds(EasynetInvocationBuilderId builder_id, uint32_t timeout_seconds);
int32_t easynet_invocation_builder_set_idempotency_key(EasynetInvocationBuilderId builder_id, const char* idempotency_key);
int32_t easynet_invocation_builder_set_caller_signature_json(EasynetInvocationBuilderId builder_id, const char* signature_json);
int32_t easynet_invocation_builder_inspect(EasynetInvocationBuilderId builder_id, char** out_invocation_json);
int32_t easynet_invocation_builder_build(EasynetInvocationBuilderId builder_id, char** out_invocation_json);
int32_t easynet_invocation_builder_prepare(EasynetHandle handle, EasynetInvocationBuilderId builder_id, const char* options_json, EasynetPreparedInvocationId* out_prepared_id, char** out_prepared_json);
int32_t easynet_invocation_builder_free(EasynetInvocationBuilderId builder_id);
```

Builder handles are mutable SDK objects. `inspect`, `build`, and `prepare`
reject incomplete seven-tuples. `inspect` does not consume the builder. `build`
and successful `builder_prepare` consume the builder handle so tuple fields
cannot be mutated after the immutable draft or canonical signing material is
created.

### 2.6 Prepare, Sign, Submit

```c
int32_t easynet_invocation_prepare(
    EasynetHandle handle,
    const char* invocation_json,
    const char* options_json,
    EasynetPreparedInvocationId* out_prepared_id,
    char** out_prepared_json
);

int32_t easynet_invocation_sign_prepared(
    EasynetPreparedInvocationId prepared_id,
    const char* signature_json,
    EasynetSignedInvocationId* out_signed_id,
    char** out_signed_json
);

int32_t easynet_invocation_submit_signed(
    EasynetHandle handle,
    EasynetSignedInvocationId signed_id,
    char** out_result_json
);

int32_t easynet_invocation_submit_signed_handle(
    EasynetHandle handle,
    EasynetSignedInvocationId signed_id,
    EasynetInvocationHandleId* out_invocation_handle_id,
    char** out_submitted_json
);

int32_t easynet_invocation_handle_await(
    EasynetHandle handle,
    EasynetInvocationHandleId invocation_handle_id,
    char** out_result_json
);

int32_t easynet_invocation_handle_cancel(
    EasynetHandle handle,
    EasynetInvocationHandleId invocation_handle_id,
    const char* reason_json,
    char** out_cancel_json
);

int32_t easynet_invocation_handle_events(
    EasynetHandle handle,
    EasynetInvocationHandleId invocation_handle_id,
    char** out_events_json
);

int32_t easynet_invocation_handle_free(
    EasynetHandle handle,
    EasynetInvocationHandleId invocation_handle_id
);

int32_t easynet_prepared_invocation_free(EasynetPreparedInvocationId prepared_id);
int32_t easynet_signed_invocation_free(EasynetSignedInvocationId signed_id);
```

`PreparedInvocation` is canonical signing material. It is not submit-ready.
`SignedInvocation` is the only submit-ready pre-runtime object. The C ABI
preserves caller signature material; bindings MUST NOT re-sign or mutate tuple
fields after prepare.

The direct JSON `easynet_invocation_prepare` entry point remains available for
bindings that already own an Invocation JSON DTO. New language facades should
prefer builder handles so the public object graph is observable before prepare.

`easynet_invocation_submit_signed_handle` is the object-model submit operation:
it consumes the signed handle and returns an `EasynetInvocationHandleId`.
Bindings observe terminal state through `handle_await`, `handle_events`, and
`handle_cancel`. Terminal state is monotonic; cancellation after a terminal
result reports `cancelled: false` and does not rewrite the result. The legacy
sync `easynet_invocation_submit_signed` remains as a convenience wrapper over
submit-handle plus await for bindings that still need a blocking call.

### 2.7 Stream And Bidi Dispatch

```c
int32_t easynet_invocation_stream_open(
    EasynetHandle handle,
    const char* invocation_json,
    EasynetInvocationStreamCallback on_chunk,
    void* user_data,
    EasynetInvocationStreamId* out_stream_id
);

int32_t easynet_invocation_stream_cancel(EasynetHandle handle, EasynetInvocationStreamId stream_id);

int32_t easynet_invocation_bidi_open(
    EasynetHandle handle,
    const char* invocation_json,
    EasynetInvocationBidiCallback on_frame,
    void* user_data,
    EasynetInvocationBidiId* out_bidi_id
);

int32_t easynet_invocation_bidi_send(
    EasynetHandle handle,
    EasynetInvocationBidiId bidi_id,
    const char* frame_json
);

int32_t easynet_invocation_bidi_close(EasynetHandle handle, EasynetInvocationBidiId bidi_id);
int32_t easynet_invocation_bidi_cancel(EasynetHandle handle, EasynetInvocationBidiId bidi_id);
```

Stream and bidi ids are scoped to the `EasynetHandle` that opened them.
Callbacks are invoked on library-owned background threads. Callback payload
strings are borrowed for the duration of the callback only.

## 3. Error Code Table

| code | name | meaning |
| --- | --- | --- |
| 0 | `EASYNET_OK` | success |
| 1 | `ERR_GENERIC` | generic or unclassified failure |
| 2 | `ERR_NULL_POINTER` | required pointer argument was null |
| 3 | `ERR_INVALID_UTF8` | C string argument was not valid UTF-8 |
| 4 | `ERR_INVALID_HANDLE` | handle was never issued or already released |
| 5 | `ERR_NOT_INITIALIZED` | library has not been initialized |
| 6 | `ERR_ALREADY_INIT` | duplicate initialization |
| 7 | `ERR_DAEMON_DOWN` | daemon endpoint cannot be reached |
| 8 | `ERR_VERSION_INCOMPATIBLE` | IPC or ABI version mismatch |
| 9 | `ERR_ABILITY_FAILED` | ability or admission execution failure |
| 10 | `ERR_NOT_IMPLEMENTED` | feature-gated symbol in a build without support |
| 11 | `ERR_INVALID_ARG` | malformed JSON, missing fields, invalid URA/base64 |
| 12 | `ERR_PERMISSION_DENIED` | daemon or admission rejected authority |
| 13 | `ERR_NOT_FOUND` | requested resource or ability not found |
| 14 | `ERR_CANCELLED` | operation/session cancelled or already closed |
| 15 | `ERR_PROTOCOL` | malformed daemon protocol status/response |
| 16 | `ERR_TIMEOUT` | operation exceeded deadline |

## 4. Retired Symbols

The historical ability+args symbols are not exported:

- `easynet_ability_invoke`
- `easynet_ability_subscribe`
- `easynet_subscription_cancel`

The ABI does not expose Axon protobuf structs, Rust pointers, generated Axon
client handles, or raw daemon socket frames.

## 5. Binding Checklist

A binding must:

- Check `easynet_abi_version() == 4`.
- Call `easynet_feature_discovery` before claiming a profile is available.
- Free every returned `char*` through `easynet_string_free`.
- Treat `PreparedInvocation` and `SignedInvocation` as different object states.
- Free prepared and signed handles explicitly.
- Close, cancel, detach, stop, or shutdown every owned object family.
- Branch on integer error codes and typed JSON, not human error strings.

# FFI ABI v1 — `libeasynet_cli` C ABI

> Plan v10.5 R1 §"FFI / cdylib" pin. Version-stable C ABI exposed
> by `libeasynet_cli.{so,dylib,dll,a}`. Client bindings in Go,
> Python, Node, Swift, Rust, Java, etc. consume this surface.

## 1. Versioning

`easynet_abi_version() -> u32` is the single source of truth.
Every breaking change to a function signature, struct shape, or
return-code semantic increments this version. cbindgen-generated
`include/easynet_cli.h` is checked in; CI asserts the header
matches the regenerated output so a hand-edit cannot drift the
contract silently.

## 2. Function families (v1 surface)

### 2.1 Lifecycle

```c
uint32_t easynet_abi_version(void);
```

Returns the ABI version number. Callers SHOULD assert the value
matches what they were built against.

```c
int32_t  easynet_init(EasynetInitMode mode, EasynetHandle* out);
int32_t  easynet_shutdown(EasynetHandle handle);
```

`easynet_init` resolves the daemon at `~/.easynet/control.json`,
dials its UDS, validates the IPC version overlap, and returns an
opaque handle in `*out`. `mode` is one of:

- `EASYNET_INIT_AUTO_SPAWN` — fork `easynet-daemon` if no
  control.json is found. Default for desktop platforms.
- `EASYNET_INIT_REQUIRE_RUNNING` — fail with `ERR_DAEMON_DOWN`
  when no daemon is listening. Use on iOS / Android once those
  platforms get bindings.

### 2.2 Generic ability invocation

```c
int32_t  easynet_ability_invoke(
    EasynetHandle  handle,
    const char*    ability_name,
    const char*    args_json,
    char**         out_response_json    // caller frees with easynet_string_free
);

uint64_t easynet_ability_subscribe(
    EasynetHandle      handle,
    const char*        ability_name,
    const char*        args_json,
    EasynetFrameCallback on_frame,
    void*              user_data
);

int32_t  easynet_subscription_cancel(EasynetHandle handle, uint64_t sub_id);
```

These two functions are the **only** dispatch primitives FFI clients
need. Every feature-specific helper below is a thin wrapper that
constructs the appropriate `ability_name` + `args_json` and calls
through.

### 2.3 Feature-specific convenience helpers

PR-ATTACH adds:
```c
int32_t  easynet_session_list(EasynetHandle, bool include_terminated, char** out);
uint64_t easynet_session_attach(EasynetHandle, const char* session_id, int64_t since_seq,
                                 EasynetFrameCallback on_frame, void* user_data);
```

PR-PERM adds:
```c
uint64_t easynet_permission_subscribe(EasynetHandle, EasynetFrameCallback on_frame, void* user_data);
int32_t  easynet_permission_decide(EasynetHandle, const char* perm_id, const char* decision);
```

PR-DISCUSS / PR-SCHED / PR-LOOP follow the same pattern.

### 2.4 Memory + errors

```c
void          easynet_string_free(char* s);
const char*   easynet_last_error(EasynetHandle handle);
```

Strings returned by the library are heap-allocated UTF-8 and must
be freed via `easynet_string_free`. `easynet_last_error` returns
a thread-local diagnostic message scoped to the handle; it is
never mutated outside an `easynet_*` call.

## 3. Error code table

| code | name                       | meaning                                          |
|------|----------------------------|--------------------------------------------------|
| 0    | `OK`                       | success                                          |
| 1    | `ERR_INVALID_ARG`          | argument validation failed                       |
| 2    | `ERR_DAEMON_DOWN`          | could not reach the local daemon                 |
| 3    | `ERR_VERSION_INCOMPATIBLE` | IPC version overlap empty                        |
| 4    | `ERR_PROTOCOL`             | wire-protocol violation (malformed JSON, etc.)   |
| 5    | `ERR_ABILITY_FAILED`       | ability handler returned an error                |
| 6    | `ERR_ALREADY_INIT`         | this handle is already initialised               |
| 7    | `ERR_TIMEOUT`              | RPC timed out                                    |
| 8    | `ERR_CANCELLED`            | subscription cancelled by caller                 |
| 99   | `ERR_INTERNAL`             | catch-all; consult `easynet_last_error`          |

## 4. Threading + reentrancy

- The library spins one I/O thread (tokio current_thread runtime)
  on `easynet_init`. C ABI calls from any thread serialise into
  that runtime via a channel; `easynet_ability_*` functions block
  the calling thread until a result is in hand.
- Callbacks (`EasynetFrameCallback`) are invoked on the I/O
  thread. Callbacks should not block — copy the frame and signal
  the consumer through a queue.
- All handles are `Send` + `Sync` at the C level (multiple threads
  may call `easynet_ability_*` on the same handle concurrently;
  the I/O thread serialises).

## 5. Conformance checklist

A new Client binding must:
- [ ] Call `easynet_abi_version()` at startup; abort if it does
      not match the expected value.
- [ ] Free every `char*` returned by an `easynet_*` function via
      `easynet_string_free`.
- [ ] Cancel every active subscription via
      `easynet_subscription_cancel` before calling
      `easynet_shutdown`.
- [ ] Treat any non-zero return code from a wrapper as a hard
      error; consult `easynet_last_error` for diagnostics.

## 6. Out of scope for v1

- async C ABI (every call is sync today; v2 may add an async
  surface for runtimes that prefer it).
- Direct streaming bytes (frames are JSON in v1; v2 may add
  proto-encoded variants).
- Capability claims (`AskContext.capability_claim` is reserved
  for v2 signed invocation).

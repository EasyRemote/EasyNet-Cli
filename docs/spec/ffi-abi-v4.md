# FFI ABI v4 - `libeasynet_cli` C ABI

Version-stable C ABI exposed by `libeasynet_cli.{so,dylib,dll,a}`.
Client bindings in Go, Python, Node, Swift, Rust, Java, and other languages
consume this surface.

ABI v4 is the Daemon SDK Runtime Core projection. It keeps the ABI v3 complete
Invocation dispatch surface and adds feature discovery, explicit daemon attach
and detach, endpoint discovery, runtime health, and the public
Draft -> Prepared -> Signed -> Submitted invocation state-machine handles plus
typed error JSON, Directory + Identity read-model projection, Receipt
fetch/projection, Host Binding host-stream codec/hash projection, and
Publication carrier projection for language bindings. It also adds Mission/EAL carrier and status projection so
language bindings submit daemon-owned orchestration through Runtime Core rather
than implementing transport facades themselves. Events directory-stream carrier
and frame projection helpers expose the daemon-owned
`federation.subscribe_directory_v2` stream without creating a second SDK event
bus.
Admin + Gateway carrier and status projection helpers expose daemon-owned
agent/session lifecycle abilities and lifecycle readiness facts without moving
backend account, pairing-token, or certificate policy into the SDK.
Surface carrier and projection helpers expose daemon-owned pages abilities and
page DTOs without moving backend rendering, browser routing, or CDN policy into
the SDK.
Compatibility carrier and projection helpers expose daemon-owned OpenAI
model/chat adapter abilities without moving product HTTP auth, billing,
rate-limit, or SSE fanout policy into the SDK.

The checked-in `include/easynet_cli.h` header is the binding-facing contract.
Rust sources under `src/ffi/` own behavior. Repository checks assert that the
header, ABI version, exported symbol set, error-code table, and this document
stay aligned.

## 1. Versioning

```c
uint32_t easynet_abi_version(void);
int32_t  easynet_feature_discovery(char** out_features_json);
const char* easynet_last_error(void);
int32_t  easynet_last_error_json(char** out_error_json);
int32_t  easynet_error_json(int32_t code, const char* message, char** out_error_json);
void easynet_string_free(char* s);
```

`easynet_abi_version()` returns `4`. Bindings MUST check it at library load and
reject incompatible libraries before opening daemon traffic.

`easynet_feature_discovery` returns caller-owned JSON. The returned `char*` MUST
be released with `easynet_string_free`.

`easynet_last_error` remains the legacy borrowed, thread-local human message.
`easynet_last_error_json` returns caller-owned schema-backed `DaemonError` JSON
for the current thread's last recorded error, or JSON `null` when no error is
recorded. `easynet_error_json` projects an explicit ABI return code and optional
message into the same DTO so bindings can branch on typed fields without parsing
human strings. Returned JSON strings MUST be released with `easynet_string_free`.

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
int32_t easynet_invocation_stream_close(EasynetHandle handle, EasynetInvocationStreamId stream_id);

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

int32_t easynet_invocation_bidi_close_send(EasynetHandle handle, EasynetInvocationBidiId bidi_id);
int32_t easynet_invocation_bidi_close(EasynetHandle handle, EasynetInvocationBidiId bidi_id);
int32_t easynet_invocation_bidi_cancel(EasynetHandle handle, EasynetInvocationBidiId bidi_id);
```

Stream and bidi ids are scoped to the `EasynetHandle` that opened them.
Callbacks are invoked on library-owned background threads. Callback payload
strings are borrowed for the duration of the callback only.

`easynet_invocation_stream_close` releases the local stream handle and stops
the background reader; it is distinct from cancel in the binding-facing object
model even though server-stream close is still a local resource action.
Unknown stream ids are treated as already closed. Owner mismatches return
`ERR_INVALID_HANDLE`.

`easynet_invocation_bidi_close_send` sends one EOF control frame and keeps the
session registered so bindings can continue to receive down-direction frames.
The operation is idempotent after the first successful EOF. Subsequent
up-direction sends return `ERR_CANCELLED` without removing the session.
`easynet_invocation_bidi_close` sends EOF if it has not already been sent, then
releases the local session handle. `easynet_invocation_bidi_cancel` releases
the local session without sending EOF.

### 2.8 Directory + Identity Read-Model Projection

```c
int32_t easynet_identity_project_ura(
    EasynetHandle handle,
    const char* ura,
    char** out_identity_json
);

int32_t easynet_identity_build_ura(
    EasynetHandle handle,
    const char* request_json,
    char** out_identity_json
);

int32_t easynet_identity_project_descriptor_ref(
    EasynetHandle handle,
    const char* descriptor_ref,
    char** out_descriptor_json
);

int32_t easynet_identity_build_descriptor_ref(
    EasynetHandle handle,
    const char* request_json,
    char** out_descriptor_json
);

int32_t easynet_directory_build_list_devices_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_directory_build_list_agents_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_directory_build_list_abilities_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_directory_build_resolve_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_directory_project_device_page(
    EasynetHandle handle,
    const char* devices_json,
    char** out_page_json
);

int32_t easynet_directory_project_agent_page(
    EasynetHandle handle,
    const char* agents_json,
    char** out_page_json
);

int32_t easynet_directory_project_ability_page(
    EasynetHandle handle,
    const char* abilities_json,
    char** out_page_json
);

int32_t easynet_directory_project_resolved_ref(
    EasynetHandle handle,
    const char* answer_json,
    char** out_resolved_ref_json
);
```

Identity projection functions are scoped to a live `EasynetHandle`, matching the
SDK object graph's `IdentityClient`. The functions delegate URA parsing and
building to Axon-owned URA helpers through `crate::core::ura`; they do not
define a second URA grammar. DescriptorRef projection/building delegates to
Axon `canonical_ability_descriptor_ref` and related helper functions.

Directory read-model functions are scoped to the same live `EasynetHandle`,
matching the SDK object graph's `DirectoryClient`. The carrier builders return
complete Invocation JSON for existing daemon read-model abilities:
`node.list`, `agent.list`, `meta.list_abilities`, and `namespace.resolve`. The
SDK requires explicit caller, callee, subject, descriptor version, nonce, causal
context, and bounded page controls where pagination applies; bindings submit
returned carriers through Runtime Core. The projection functions normalize
daemon rows into `DirectoryPage` DTOs with explicit `DefaultPageSize = 50`,
`MaxPageSize = 500`, offset cursors, and `source = "read_model"`.
`easynet_directory_project_resolved_ref` normalizes daemon `ResolveAnswer`
proto-JSON into a stable `ResolvedRef` DTO while preserving resolver facts such
as `answer_kind`, owner/ability/route URAs, next-hop evidence, negative answers,
authority, cache policy, and the raw answer in metadata. `resolve` does not
execute abilities, pick routes in the SDK, or fan out through
`federation.resolve`; daemon resolver state remains authoritative.
`list_abilities` maps the public SDK `owner_ura` query to the daemon ability's
historical `agent_ura` parameter, but it does not fan out across every agent by
default. ABI v4 Directory support is `read_model_projection_partial`: directory
subscribe convenience wrappers, signing-key lifecycle APIs, backend database
projections, and language facades remain future work.

### 2.9 Receipt Fetch and Projection

```c
int32_t easynet_receipt_build_fetch_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_receipt_project(
    EasynetHandle handle,
    const char* receipt_json,
    char** out_summary_json
);

int32_t easynet_receipt_verify(
    EasynetHandle handle,
    const char* receipt_json,
    char** out_verification_json
);

int32_t easynet_receipt_verify_chain(
    EasynetHandle handle,
    const char* request_json,
    char** out_verification_json
);

int32_t easynet_receipt_causal_ref(
    EasynetHandle handle,
    const char* receipt_json,
    char** out_causal_ref_json
);
```

Receipt projection functions are scoped to a live `EasynetHandle`, matching the
SDK object graph's `ReceiptClient`. `build_fetch_invocation` returns complete
Invocation JSON for daemon `invocation.history.get` with exactly one public
selector: `invocation_ura`, `request_id`, or `trace_id`. Bindings submit the
returned carrier through Runtime Core; the SDK does not open the daemon ledger
file, fabricate receipt URAs, or use control frames for receipt reads.
`project` normalizes receipt-like JSON into the shared `ReceiptSummary` DTO.
`verify` is conservative in ABI v4: it returns typed JSON with
`verified: false` for summary-only data and does not claim Axon cryptographic
verification. `verify_chain` accepts `{receipts:[...]}` and returns daemon
receipt-chain continuity facts separately from cryptographic verification.
`causal_ref` builds a scalar causal context only when the input contains an
explicit non-empty `receipt_ura` and a valid 32-byte receipt hash
(`self_hash_hex`, `receipt_hash_hex`, or `receipt_hash`).

### 2.10 Host Binding Codec

```c
int32_t easynet_host_binding_build(
    EasynetHandle handle,
    const char* request_json,
    char** out_binding_json
);

int32_t easynet_host_binding_decode_request(
    EasynetHandle handle,
    const char* envelope_json,
    char** out_request_json
);

int32_t easynet_host_binding_encode_item(
    EasynetHandle handle,
    const char* item_json,
    char** out_frame_json
);

int32_t easynet_host_binding_encode_error(
    EasynetHandle handle,
    const char* error_json,
    char** out_frame_json
);

int32_t easynet_host_binding_encode_terminal(
    EasynetHandle handle,
    const char* terminal_json,
    char** out_frame_json
);

int32_t easynet_host_binding_fold_output_hash(
    EasynetHandle handle,
    const char* fold_json,
    char** out_state_json
);
```

Host Binding codec functions are scoped to a live `EasynetHandle`, matching
the SDK object graph's `HostBindingClient`. They are pure DTO projections: they
do not spawn a warm host, inspect Python functions, load decorators, scan
package directories, or dial the daemon.

`build` validates `HostStreamBindingRequest`, canonicalizes
`descriptor_ref` through Axon helpers, requires the shared
`host-stream-frame.schema.json` frame schema, and returns a typed binding DTO
with endpoint, readiness, cleanup, timeout, lifecycle ownership, and hash
metadata. The endpoint must satisfy the same local host-stream invariant as the
daemon executor: absolute Unix socket path, no `..` components.

`decode_request` validates the current daemon host-stream envelope and returns
`HostStreamRequest` with `function`, `args`, `call_id`, and `caller`.
`encode_item`, `encode_error`, and `encode_terminal` return shared
`HostStreamFrame` DTOs. `fold_output_hash` accepts explicit `HostStreamHashState`
plus `seq` and `value`, then returns the next state using the daemon-owned
`sha256(prev_hash || seq_be || canonical_json(value))` algorithm.

### 2.11 Publication Carriers

```c
int32_t easynet_publication_build_resource_ref(
    EasynetHandle handle,
    const char* request_json,
    char** out_resource_ref_json
);

int32_t easynet_publication_validate_package(
    EasynetHandle handle,
    const char* request_json,
    char** out_validation_json
);

int32_t easynet_publication_build_deploy_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_publication_build_unpublish_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);
```

Publication carrier functions are scoped to a live `EasynetHandle`, matching
the SDK object graph's `PublicationClient`. They are pure DTO/carrier
projection functions. `build_resource_ref` constructs daemon-authored local
filesystem `ResourceRef` objects for absolute paths under daemon virtual roots.
`validate_package` reads an ability package directory's `ability.json`, parses
it through the daemon `AbilityManifest` validator, and returns deterministic
package facts including namespace, wire key, descriptor version, exec kind, and
manifest hash.

`build_deploy_invocation` and `build_unpublish_invocation` return complete
Invocation JSON carriers for existing daemon system abilities (`ability.deploy`
and `ability.unpublish`). They require explicit caller, callee, subject,
descriptor version, nonce, and causal context fields. Bindings submit the
returned Invocation through Runtime Core; these helpers do not execute
publication, claim terminal receipts, scan package directories for list/show,
or fake enable/disable state.

### 2.12 Mission Carriers And Status

```c
int32_t easynet_mission_build_run_eal_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_mission_build_run_file_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_mission_build_track_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_mission_build_cancel_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_mission_project_status(
    EasynetHandle handle,
    const char* status_json,
    char** out_status_json
);

int32_t easynet_mission_project_events(
    EasynetHandle handle,
    const char* events_json,
    char** out_page_json
);
```

Mission carrier functions are scoped to a live `EasynetHandle`, matching the
SDK object graph's `MissionClient`. `build_run_eal_invocation` returns a
complete Invocation JSON carrier for daemon `mission.run` from EAL source text.
`build_run_file_invocation` reads an absolute local EAL source file, uses the
file path as the default label, and returns the same `mission.run` carrier.
`build_track_invocation` and `build_cancel_invocation` map SDK `mission_id` to
the existing daemon ability argument `run_id` while preserving the complete
Invocation tuple.

`easynet_mission_project_status` normalizes daemon `mission.run`,
`mission.track`, or `mission.cancel` JSON into `MissionStatus`. The projection
exposes mission id, terminal state, partial failure count, cancellation state,
parent invocation context when available, parent receipt URA when observable,
child invocation refs, child receipt refs, and output artifact refs. It never
fabricates receipt anchors for receipt-less steps. Bindings submit returned
Invocation carriers through Runtime Core; these helpers do not execute EAL,
start a mission runtime, or replace daemon Mission/EAL semantics.
`easynet_mission_project_events` normalizes daemon mission timeline replay JSON
into `MissionEventPage` with explicit sequence cursors and terminal event
state. It does not read mission run directories, infer cursors from timestamps
or array positions, or create a second event bus.

### 2.13 Events Directory Stream

```c
int32_t easynet_events_build_directory_subscription_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_events_project_directory_event(
    EasynetHandle handle,
    const char* event_json,
    char** out_event_json
);

int32_t easynet_events_project_terminal(
    EasynetHandle handle,
    const char* terminal_json,
    char** out_event_json
);

int32_t easynet_events_project_drop_report(
    EasynetHandle handle,
    const char* drop_json,
    char** out_event_json
);
```

Events functions are scoped to a live `EasynetHandle`, matching the SDK object
graph's `EventClient`. `build_directory_subscription_invocation` returns a
complete Invocation JSON carrier for daemon `federation.subscribe_directory_v2`.
It requires explicit caller, callee, subject, descriptor version, nonce, and
causal context fields. Bindings submit the carrier through Runtime Core stream
open; this helper does not open the stream itself.

`easynet_events_project_directory_event` accepts a daemon `DirectoryEvent`
frame plus explicit cursor information, for example
`{"cursor":{"stream":"directory","sequence":8},"event":{"type":"heartbeat",...}}`,
and returns a typed Events `EventFrame` with `snapshot`, live delta, heartbeat,
typed subject/realm refs, cursor, resume token, `occurred_unix_ms`, payload,
drop count, reconnect hint, terminal flag, and metadata. The daemon raw event
wire shape does not carry resume state, so bindings must supply a cursor or
sequence maintained by their stream reader; the SDK does not infer cursor
positions from array indexes or timestamps.

`easynet_events_project_drop_report` and `easynet_events_project_terminal`
create first-class non-payload stream lifecycle frames. Dropped-event reports
must include a non-zero `dropped_count`; terminal frames are explicit final
states. ABI v4 Events support is `directory_stream_partial`: it stabilizes the
real daemon directory stream carrier/projection boundary, but does not claim
device/session/invocation event subscriptions, backend SSE/WebSocket fanout, or
daemon-side directory filtering semantics.

### 2.14 Admin + Gateway Carriers And Status

```c
int32_t easynet_admin_build_agent_list_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_admin_build_agent_start_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_admin_build_agent_stop_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_admin_build_agent_refresh_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_admin_build_session_list_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_admin_project_gateway_status(
    EasynetHandle handle,
    const char* status_json,
    char** out_status_json
);

int32_t easynet_admin_project_agent_records(
    EasynetHandle handle,
    const char* agents_json,
    char** out_agents_json
);

int32_t easynet_admin_project_agent_lifecycle_result(
    EasynetHandle handle,
    const char* result_json,
    char** out_result_json
);
```

Admin + Gateway functions are scoped to a live `EasynetHandle`, matching the
SDK object graph's `AdminClient`. The carrier builders return complete
Invocation JSON for daemon-owned `agent.list`, `agent.start`, `agent.stop`,
`agent.refresh`, and `session.list`. They require explicit caller, callee,
subject, descriptor version, nonce, and causal context fields. Bindings submit
returned carriers through Runtime Core; these helpers do not execute admin
operations directly.

`easynet_admin_project_gateway_status` accepts daemon lifecycle/status JSON
with `runtime_status`, `daemon`, `runtime`, and `product_presence` facts and
returns `GatewayStatus`. The projection keeps process liveness, control
readiness, Invocation runtime readiness, directory readiness, trust readiness,
and public listener readiness as separate booleans. A control-only daemon is
reported as degraded rather than collapsed into a generic failure.

`easynet_admin_project_agent_records` projects daemon `agent.list` results into
SDK `AgentRecord` page DTOs. Missing hosted-agent URAs remain null; the SDK
derives owner refs only from valid Agent URAs and never fabricates identities.
`easynet_admin_project_agent_lifecycle_result` normalizes daemon
`agent.start`, `agent.stop`, and `agent.refresh` outcomes into typed lifecycle
results. ABI v4 Admin + Gateway support is `carrier_status_partial`: pairing
token creation/validation, credential verification, certificate policy,
gateway onboarding UX, and full device-session CRUD remain future daemon or
product-profile work.

### 2.15 Surface Page Carriers And Projections

```c
int32_t easynet_surface_build_list_pages_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_surface_build_create_page_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_surface_build_delete_page_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_surface_build_manifest_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_surface_project_page_record(
    EasynetHandle handle,
    const char* page_json,
    char** out_page_json
);

int32_t easynet_surface_project_page_page(
    EasynetHandle handle,
    const char* pages_json,
    char** out_page_json
);

int32_t easynet_surface_project_manifest(
    EasynetHandle handle,
    const char* page_json,
    char** out_manifest_json
);

int32_t easynet_surface_project_public_page_ref(
    EasynetHandle handle,
    const char* page_json,
    char** out_ref_json
);

int32_t easynet_surface_project_mutation_result(
    EasynetHandle handle,
    const char* result_json,
    char** out_result_json
);
```

Surface functions are scoped to a live `EasynetHandle`, matching the SDK
object graph's `SurfaceClient`. The carrier builders return complete
Invocation JSON for daemon-owned `pages.list`, `pages.publish`, `pages.get`,
and `pages.unpublish`. They require explicit caller, callee, subject,
descriptor version, nonce, and causal context fields. Bindings submit returned
carriers through Runtime Core; these helpers do not render HTML, call backend
HTTP routes, or open page folders directly.

The projection helpers normalize daemon page facts into `PageRecord`,
`Page<PageRecord>`, `SurfaceManifest`, `PublicPageRef`, and
`SurfaceMutationResult` DTOs. Page identity is derived only from explicit page
facts such as `project_ura`, `owner_ura`, `realm`, and `user`; public refs are
product route references, not daemon transport endpoints. ABI v4 Surface
support is `carrier_projection_partial`: backend route serving, browser auth,
CDN/cache policy, content-management UX, and full surface status remain future
daemon or product-profile work.

### 2.16 Compatibility Carriers And Projections

```c
int32_t easynet_compatibility_build_list_models_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_compatibility_build_chat_completion_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_compatibility_build_stream_chat_completion_invocation(
    EasynetHandle handle,
    const char* request_json,
    char** out_invocation_json
);

int32_t easynet_compatibility_project_model_page(
    EasynetHandle handle,
    const char* models_json,
    char** out_models_json
);

int32_t easynet_compatibility_project_chat_completion(
    EasynetHandle handle,
    const char* completion_json,
    char** out_completion_json
);

int32_t easynet_compatibility_project_chat_stream(
    EasynetHandle handle,
    const char* stream_json,
    char** out_stream_json
);

int32_t easynet_compatibility_project_file_upload(
    EasynetHandle handle,
    const char* file_json,
    char** out_file_json
);

int32_t easynet_compatibility_project_file(
    EasynetHandle handle,
    const char* file_json,
    char** out_file_json
);

int32_t easynet_compatibility_project_file_delete_result(
    EasynetHandle handle,
    const char* result_json,
    char** out_result_json
);
```

Compatibility functions are scoped to a live `EasynetHandle`, matching the SDK
object graph's `CompatibilityClient`. The carrier builders return complete
Invocation JSON for daemon-owned `openai.list_models` and
`openai.chat_completions`. They require explicit caller, callee, subject,
descriptor version, nonce, and causal context fields. Bindings submit returned
carriers through Runtime Core; these helpers do not execute model calls
directly.

`easynet_compatibility_build_chat_completion_invocation` rejects requests whose
OpenAI `stream` flag is true. Streaming callers use
`easynet_compatibility_build_stream_chat_completion_invocation`, which makes
`request.stream = true` explicit in the returned carrier. Both paths require
`request.model` to be a canonical agent-owned chat Ability URA; provider
nicknames such as `gpt-5` are not accepted at this daemon SDK layer.

`easynet_compatibility_project_model_page`,
`easynet_compatibility_project_chat_completion`, and
`easynet_compatibility_project_chat_stream` validate daemon-returned
OpenAI-compatible envelopes and project them into SDK DTOs with
`profile = "compatibility"`.

`easynet_compatibility_project_file_upload`,
`easynet_compatibility_project_file`, and
`easynet_compatibility_project_file_delete_result` adapt SDK file/resource
facts into OpenAI-compatible file DTOs. They do not invent `openai.files.*`
daemon abilities, delete files, parse multipart requests, or own product file
storage policy. ABI v4 Compatibility support is `carrier_projection_partial`:
product API-key policy, quota/rate-limit policy, billing, HTTP route shaping,
multipart upload handling, and SSE/WebSocket fanout remain product-profile or
wrapper-profile work.

### 2.17 Convenience Wrapper Record Projections

```c
int32_t easynet_wrappers_project_file_record(
    EasynetHandle handle,
    const char* file_json,
    char** out_file_json
);

int32_t easynet_wrappers_project_terminal_session(
    EasynetHandle handle,
    const char* session_json,
    char** out_session_json
);

int32_t easynet_wrappers_project_remote_desktop_session(
    EasynetHandle handle,
    const char* session_json,
    char** out_session_json
);

int32_t easynet_wrappers_project_browser_session(
    EasynetHandle handle,
    const char* session_json,
    char** out_session_json
);

int32_t easynet_wrappers_project_media_session(
    EasynetHandle handle,
    const char* session_json,
    char** out_session_json
);
```

Wrapper functions are scoped to a live `EasynetHandle`, matching the SDK object
graph's file, terminal, remote desktop, browser, and media wrapper clients.
They project daemon/resource/session facts into schema-backed SDK DTO records
with `profile = "wrappers"`. They do not start sessions, open product
WebSockets, parse multipart requests, own backend storage/auth policy, or
replace Runtime Core Invocation, StreamHandle, or BidiSession execution paths.
ABI v4 wrapper support is `record_projection_partial`: execution helpers and
language facades remain future wrapper-profile work.

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

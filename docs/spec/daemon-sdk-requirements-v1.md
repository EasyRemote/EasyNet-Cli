# Daemon SDK Requirements v1

Status: active design spec.

Audience: EasyNet-Cli, EasyNet backend, EasyRemote, and language binding
maintainers.

Purpose: define the public SDK surface owned by EasyNet-Cli for controlling and
calling `easynet-daemon`. This is a Daemon SDK, not a command-line SDK. The
`easynet` executable is one consumer of this SDK; it is not the SDK boundary.

Normative language: `MUST`, `MUST NOT`, `SHOULD`, and `MAY` are intentional.
Any implementation that violates a `MUST` is not compatible with this spec.

## 1. Review Corrections Integrated

This revision repairs the root causes that made prior SDK planning
non-convergent:

1. **Duplicate architecture source of truth**: language support, OOP model,
   state machines, and project structure were appended after an already complete
   API spec. They are now first-class sections in one normative document.
2. **Implementation/facade confusion**: Go, Python, Node, Java, and Swift are
   language facades. The native Rust daemon SDK core plus C ABI projection are
   the semantic implementation. Facades may own ergonomics, not daemon or Axon
   semantics.
3. **Weak object model**: lifecycle, runtime client, invocation construction,
   prepared signing material, streams, bidi sessions, directory, identity, and
   health are now explicit SDK objects with ownership and terminal operations.
4. **Underspecified state machines**: daemon lifecycle, client connection,
   invocation, stream, bidi, directory subscription, and explicit aggregate
   fan-out now have terminal-state rules.
5. **Unbounded listing risk**: ordinary device/agent/ability listing is a
   read-model facade. Per-agent/per-ability fan-out is forbidden in ordinary
   list methods and allowed only through named aggregate abilities with bounds.
6. **Project structure drift**: required roots, ownership per directory, and
   structural migration rules now define the target shape without relying on
   examples or gallery code as hidden behavior.

## 2. Decision

EasyNet product clients MUST depend on EasyNet-Cli SDK surfaces, not directly on
EasyNet-Axon SDKs or raw `axon.v1` generated protocol types.

The target dependency graph is:

```text
EasyNet backend / EasyRemote / GUI / host app / easynet CLI
  -> EasyNet-Cli Daemon SDK
  -> local easynet-daemon
  -> daemon-owned Axon adapter
  -> Axon SDK, proto, LocalRuntime, admission, receipts
```

The forbidden product graph is:

```text
EasyNet backend / EasyRemote
  -> EasyNet-Axon SDK or generated axon.v1 proto
  -> raw axon-runtime
```

This does not mean the SDK hides the words Invocation, URA, AbilityDescriptor,
or Receipt. Those are public EasyNet/Axon semantic terms. It means consumers do
not import Axon packages, do not construct Axon proto messages, and do not start
or dial raw `axon-runtime` for product paths.

Product deployments use `easynet-daemon`. Raw `axon-runtime` remains valid for
Axon protocol reference runtimes, Axon SDK tests, protocol examples, and
third-party minimal runtimes, but not for EasyNet product paths.

## 3. Ownership

| Layer | Owns | Must not own |
| --- | --- | --- |
| Axon | Invocation wire shape, canonical bytes, URA profile, admission, receipt verification, stream/bidi protocol invariants | EasyNet daemon lifecycle, plugins, keyring forwarding, EAL/Mission product policy, backend DB/product UX |
| EasyNet-Cli daemon | Device/Hub process lifecycle, local sockets, identity, keyring, plugin/MCP/EAL/Mission execution, local and remote dispatch, session state, daemon system abilities | Browser auth, product database, OAuth/JWT, frontend DTOs |
| EasyNet-Cli SDK core | Daemon lifecycle/client object model, language-neutral DTOs, C ABI handle model, daemon transport wrappers, typed errors, conformance fixtures | New Axon protocol semantics, backend product state, EasyRemote decorators |
| Language SDK facades | Idiomatic builders, async adapters, packaging, generated DTO bindings, retry/error mapping | Canonical Invocation algorithms, receipt verification algorithms, daemon policy, Axon admission semantics |
| EasyNet backend | Browser/product HTTP API, user/device dashboards, DB projections, Join Token UX | Ability execution, daemon session routing, Axon protocol rules |
| EasyRemote | Python developer facade for turning functions/classes/pipelines into abilities | Low-level daemon transport, canonical signing, receipt verification, daemon process ABI |

## 4. Consumers and Release Tiers

The SDK MUST support these first-class consumers:

1. EasyNet backend, primarily through a Go SDK.
2. EasyRemote, through a Python SDK and/or Python binding over `libeasynet_cli`.
3. The `easynet` command-line executable.
4. Desktop or GUI products that need daemon lifecycle and invocation.
5. Third-party host apps that expose local resources as EasyNet abilities.

The EasyNet backend MUST NOT depend on EasyRemote. EasyRemote is a product
facade above the daemon SDK, not a runtime substrate for the backend.

| Tier | Language target | Primary consumer | Required at | Notes |
| --- | --- | --- | --- | --- |
| P0 | Rust | native SDK core, daemon internals, native tests, FFI implementation | SDK core freeze | Source-of-truth implementation and conformance runner host. |
| P0 | C ABI | Python/Node/Swift/JVM bridge option, native embedding | SDK core freeze | Stable ABI surface in `include/easynet_cli.h`; no Axon symbols exposed. |
| P0 | Go | EasyNet backend / Hub | backend cutover | Preferred backend facade; uses daemon local transport and/or C ABI helpers without importing Axon or owning Axon semantics. |
| P0 | Python | EasyRemote and local automation | EasyRemote cutover | Must support Pythonic OOP and async-friendly wrappers without making EasyRemote depend on backend. |
| P1 | Node / TypeScript | desktop tools, extension hosts, local dev tooling | post-backend cutover | Must provide typed declarations and promise/async iterator APIs. |
| P1 | Java / Kotlin JVM | enterprise and Android-adjacent integrations | post-backend cutover | Java API is normative; Kotlin gets idiomatic helpers over it. |
| P1 | Swift | macOS/iOS-adjacent clients | post-backend cutover | Swift Package Manager distribution; async/await first. |
| P2 | React/browser facade | frontend product SDK only | optional | Not a daemon SDK. Browser code cannot call local daemon UDS; any React facade talks to EasyNet backend. |

Rust, C ABI, Go, and Python are the first cutover languages because they cover
daemon implementation, backend, and EasyRemote. Node, Java, and Swift must be
designed at the same semantic level even if they ship later; the P0 design MUST
NOT paint them into a corner.

## 5. Public SDK Surfaces and Single Implementation Rule

The SDK family SHOULD have these public surfaces:

| Surface | Role | Stability target |
| --- | --- | --- |
| Native Rust SDK, `easynet_cli::{daemon,runtime,invocation}` | The single semantic implementation owned by EasyNet-Cli: daemon lifecycle, daemon client object model, EasyNet policy wrappers, and Axon delegation boundary | Semver crate API |
| C ABI, `libeasynet_cli` | Stable ABI projection of the native Rust SDK for language bindings | ABI versioned, header-checked |
| Go SDK, `easynet.run/cli/sdk/go/...` | Primary EasyNet backend facade over daemon SDK semantics | Semver module API |
| Python SDK, package name TBD | Primary EasyRemote low-level facade over daemon SDK semantics | Semver package API |
| Future bindings | Node, Swift, Java, etc. | Generated or hand-written facade over the same schema and Rust/C ABI core |

All surfaces MUST share the same logical data model and JSON schemas for
Invocation, authority metadata, daemon status, receipt summaries, stream events,
directory events, and typed errors.

There is one daemon SDK semantic implementation, not one implementation per
language.

1. The native Rust SDK is the source-of-truth implementation for EasyNet-Cli
   daemon lifecycle, local socket discovery, daemon readiness, EasyNet policy
   wrappers, language-neutral DTOs, and FFI handle ownership.
2. `libeasynet_cli` projects that Rust implementation as a stable C ABI.
3. Go, Python, Node/TypeScript, Java, Swift, and future SDKs are facades or
   wrappers. They may own idiomatic builders, async adapters, packaging,
   generated type bindings, retry classification, and error mapping, but they
   MUST NOT fork daemon SDK semantics.
4. If a non-Rust SDK talks directly to the daemon local transport for deployment
   reasons, that transport code is still a facade. It may serialize requests,
   manage connection lifecycle, and map errors, but it MUST NOT reimplement
   canonical Invocation signing material, receipt verification, Axon admission
   semantics, or stream/bidi terminal-state rules.
5. Semantics already implemented by EasyNet-Axon MUST be delegated to Axon from
   the Rust daemon SDK core or daemon runtime adapter. EasyNet-Cli MUST NOT
   create a second Axon SDK inside CLI language bindings.

## 6. Non-goals

The Daemon SDK MUST NOT expose:

1. `StartAxonRuntime`, `start_runtime`, or any raw `axon-runtime` lifecycle.
2. Raw `pb.InvokeRequest`, `pb.InvokeResponse`, `pb.InvokeBidiUp`, or other
   generated `axon.v1` types at product-facing boundaries.
3. Direct imports of `easynet.run/axon/*` in product SDK APIs.
4. `InvokeAbility(name, args)` or any public ability+args-only call surface.
5. A shell transport such as `RunCLICommand("easynet ...")`.
6. Product invocation over `control.sock` JSON frames.
7. Backend auth/JWT/OAuth/database helpers.
8. Frontend DTOs or HTTP route helpers.
9. One method per system ability as the only calling model.
10. SDK-side per-target fan-out for governed ability calls.

Convenience helpers MAY exist, but every helper MUST eventually produce an
inspectable complete Invocation before dispatch.

## 7. Core Invariants

### 7.1 Complete Invocation

Every dispatch API MUST preserve the complete Invocation tuple:

```text
caller, callee, ability, subject, nonce, causal_context, args -> receipt
```

`ability` is represented by a descriptor-bound reference:

```text
DescriptorRef = ability_ura@descriptor_version
```

SDK builders MAY fill defaults such as a fresh nonce or root causal context, but
the filled values MUST be visible before dispatch. Hidden `subject`, hidden
`nonce`, or silently empty causal context at a public boundary is a correctness
bug.

### 7.2 Identity Split

The SDK MUST keep these separate:

| Term | Meaning |
| --- | --- |
| URA | Routable logical identity/address for agents, devices, abilities, resources, sessions, receipts |
| Endpoint | Transport locator such as UDS path or TCP endpoint |
| Process | Local OS process, e.g. `easynet-daemon` |
| AbilityDescriptor | Versioned governed interface with schema, policy, observability, and receipt semantics |
| AbilityImpl | Versioned executable binding that can satisfy one descriptor version |
| Plugin/Skill | Implementation resource that may be used by an AbilityImpl |

`callee` is the logical agent/device that advertises an AbilityDescriptor. It is
not the UDS path, TCP endpoint, daemon process, node id string, plugin instance,
or skill name.

### 7.3 Transport Split

`control.sock` is only for boot/status/discovery. Product calls MUST use the
daemon Invocation endpoint (`daemon.sock` locally, TCP+TLS for remote
device/Hub traffic). SDKs MAY use `control.json` to discover the current
Invocation endpoint, but MUST NOT dispatch product ability calls over control
frames.

### 7.4 Axon Dependency Containment

SDK implementation crates/packages MAY depend on Axon SDKs and generated
protocol types internally only where the layer owns the adapter. Public product
APIs MUST wrap those in EasyNet-Cli owned DTOs and typed errors.

### 7.5 Descriptor and Receipt Binding

`AbilityDescriptor` MUST carry descriptor version and schema hash. `AbilityImpl`
MUST carry implementation hash and runtime environment. Invocation/Receipt
projections exposed by the SDK MUST preserve enough fields for callers to bind:

1. descriptor version,
2. schema hash,
3. implementation hash when daemon returns it,
4. runtime environment when daemon returns it,
5. authority proof,
6. input hash,
7. output hash,
8. parent receipts,
9. terminal state,
10. signature or verifiable receipt reference.

The SDK does not create these protocol truths. It preserves and projects them.

## 8. Target Project Structure

The current repository already contains Rust daemon code and a C header. The
target structure makes the SDK boundary explicit and follows Axon's separation
between core, SDK, packaging, documents, and examples.

### 8.1 Required Roots

```text
EasyNet-Cli/
  README.md
  PROJECT_STRUCTURE.md
  Cargo.toml
  include/
    easynet_cli.h
  src/
    daemon/
      lifecycle/
      health/
      runtime_client/
      directory/
      identity/
      transport/
    ffi/
      abi_version.rs
      handles.rs
      runtime.rs
      invocation.rs
      errors.rs
    protocol/
      canonical_json.rs
      invocation_json.rs
      receipt_json.rs
      error_json.rs
      health_json.rs
      event_json.rs
  sdk/
    README.md
    SDK_INTERFACE_SPEC.md
    SDK_PARITY.md
    CONFORMANCE_SUITE.md
    schemas/
      invocation.schema.json
      receipt.schema.json
      error.schema.json
      health.schema.json
      events.schema.json
    conformance/
      cases/
      fixtures/
      runner/
    rust/
    c/
    go/
    python/
    node/
    java/
    swift/
  docs/
    spec/
      daemon-sdk-requirements-v1.md
  packaging/
    release/
    sdk-pack/
  examples/
  gallery/
```

If the existing physical layout differs, migration MUST be staged. The spec
defines the final ownership model; it does not require a single large move.

### 8.2 Ownership by Directory

| Path | Owns | Must not own |
| --- | --- | --- |
| `src/daemon/` | daemon process lifecycle, UDS endpoints, local runtime orchestration, native SDK core objects | language-specific SDK ergonomics |
| `src/protocol/` | canonical JSON projection, schema projection, typed daemon protocol DTOs | product helpers or EasyRemote decorators |
| `src/ffi/` | C ABI handle model and error conversion | daemon business policy |
| `include/easynet_cli.h` | stable public ABI declarations | generated Axon structs |
| `sdk/schemas/` | JSON schema for language parity and generated test data | implementation-only Rust structs |
| `sdk/conformance/` | golden fixtures and cross-language behavior tests | examples that silently define behavior |
| `sdk/rust/` | Native Rust SDK public surface if separated from current crate modules | duplicated Axon or daemon semantics |
| `sdk/c/` | C examples, ABI docs, generated binding metadata | duplicated C implementation |
| `sdk/go/` | Go facade package used by EasyNet backend | Axon imports in public package surface or duplicated Axon semantics |
| `sdk/python/` | Python facade package consumed by EasyRemote | EasyRemote-specific decorators as core SDK or duplicated Axon semantics |
| `sdk/node/` | Node/TypeScript facade package | browser-only assumptions or duplicated Axon semantics |
| `sdk/java/` | Java/JVM facade package | Android UI concerns or duplicated Axon semantics |
| `sdk/swift/` | Swift facade package | platform UI concerns |
| `packaging/` | release bundles, wheels, npm/maven/spm/crate metadata | SDK semantics |
| `examples/` | runnable examples | protocol orchestration source of truth |
| `gallery/` | demos and showcases | SDK tests or normative behavior |

### 8.3 Structural Migration Rules

1. Physical moves use `git mv` so history remains traceable.
2. Do not mix large structural moves with unrelated logic changes.
3. Every moved public artifact must update docs, tests, CI, release scripts, and
   generated references in the same stage.
4. There must be one source of truth per artifact class:
   - Invocation schema: `sdk/schemas/invocation.schema.json`.
   - C ABI declaration: `include/easynet_cli.h`.
   - Rust daemon transport: `src/daemon/transport/`.
   - Native daemon SDK semantics: Rust crate `easynet_cli`.
   - Language wrappers: `sdk/<language>/`.
   - Golden behavior: `sdk/conformance/cases/`.
5. `examples/` and `gallery/` can import SDK packages; SDK packages must never
   import examples or gallery code.
6. Backend cutover must depend on `sdk/go`, not `src/daemon` internals.
7. EasyRemote cutover must depend on `sdk/python` and/or C ABI, not backend code.

## 9. OOP Object Model

The SDK is object-oriented at the public boundary even when a target language
has a functional or procedural implementation style. C ABI represents objects as
opaque handles; all other languages expose classes/structs/interfaces with the
same lifecycle.

### 9.1 Object Graph

```text
SdkEnvironment
  -> DaemonHandle
       -> RuntimeClient
            -> InvocationBuilder
                 -> InvocationDraft
                      -> PreparedInvocation
                           -> SignedInvocation
                                -> InvocationHandle
                                     -> InvocationResult
            -> StreamHandle
            -> BidiSession
       -> DirectoryClient
       -> IdentityClient
       -> HealthClient
```

No object in this graph may expose raw Axon client/proto/runtime types.

### 9.2 Core Objects

| Object | Responsibility | Owns resources | Terminal operation |
| --- | --- | --- | --- |
| `SdkEnvironment` | SDK initialization, version checks, default paths, feature flags | global SDK config only | `close` / drop |
| `DaemonHandle` | discover, start, attach, stop, inspect daemon process | optional child process plus socket metadata | `stop`, `detach`, drop |
| `RuntimeClient` | authenticated connection to daemon invocation endpoint | UDS/TCP connection pool, retry policy, request ids | `close` |
| `InvocationBuilder` | construct a complete seven-tuple draft | mutable draft only | `build`, `prepare`, `invoke` |
| `InvocationDraft` | inspectable immutable seven-tuple before canonical prepare | tuple snapshot | `prepare`, `invoke` |
| `PreparedInvocation` | immutable canonical signing material | canonical bytes, request id, tuple snapshot | `sign`, `submit_unsigned` if policy allows |
| `SignedInvocation` | immutable caller-signed invocation | signature, signer id, prepared tuple | `submit` |
| `InvocationHandle` | observe submitted invocation | invocation id, event cursor | `await_result`, `cancel`, `events` |
| `InvocationResult` | terminal result projection | receipt, output, terminal status | none |
| `StreamHandle` | receive server-stream events with terminal state | stream cursor and backpressure state | `close`, `cancel` |
| `BidiSession` | send/receive bidirectional frames | send queue, receive cursor, frame0 metadata | `close_send`, `close`, `cancel` |
| `DirectoryClient` | device/agent/ability catalog read model | subscription cursors | `close` |
| `IdentityClient` | local caller identity and signing key access | key handles, policy metadata | `close` |
| `HealthClient` | readiness and diagnostics | none or short-lived connection | none |
| `DaemonError` | typed error and retry classification | code, message, source, retry hint | none |

### 9.3 Required Interfaces

The exact syntax is language-specific, but every target must represent these
interfaces.

```text
interface DaemonHandle {
  status() -> DaemonStatus
  endpoints() -> Endpoints
  runtime() -> RuntimeClient
  directory() -> DirectoryClient
  identity() -> IdentityClient
  health() -> HealthClient
  stop(options) -> StopResult
  detach() -> void
}

interface RuntimeClient {
  new_invocation() -> InvocationBuilder
  invoke(invocation) -> InvocationResult
  prepare(invocation) -> PreparedInvocation
  submit_signed(signed) -> InvocationHandle
  open_stream(invocation) -> StreamHandle
  open_bidi(invocation, frame0) -> BidiSession
  close() -> void
}

interface InvocationBuilder {
  caller(ura) -> InvocationBuilder
  callee(ura) -> InvocationBuilder
  descriptor(ref) -> InvocationBuilder
  subject(ura_or_subject) -> InvocationBuilder
  nonce(bytes_or_string) -> InvocationBuilder
  causal_context(context) -> InvocationBuilder
  args(json_or_bytes) -> InvocationBuilder
  inspect() -> InvocationDraft
  prepare() -> PreparedInvocation
  invoke() -> InvocationResult
}
```

Convenience helpers are allowed only if they lower to this object graph:

```text
client.device("edge-01").ability("fs.read").subject(file_ura).invoke(args)
```

The helper above is valid only if the SDK can expose the resulting full
Invocation draft before dispatch.

### 9.4 Lifetime Rules

1. `DaemonHandle` may own a process or attach to an existing process.
2. `detach` releases the local handle without stopping an existing daemon.
3. `stop` is idempotent and may only stop a process the handle is authorized to
   control.
4. `RuntimeClient` does not imply process ownership.
5. Closing a `RuntimeClient` never stops the daemon.
6. `InvocationBuilder` is mutable and not thread-safe unless a language marks it
   as such.
7. `InvocationDraft`, `PreparedInvocation`, and `SignedInvocation` are immutable
   and safe to share where the language supports immutable sharing.
8. `InvocationHandle.cancel` sends cancellation to daemon/runtime; dropping the
   local handle does not imply cancellation unless explicitly documented.
9. `StreamHandle` and `BidiSession` must expose explicit close/cancel semantics.
10. All object methods that touch the daemon must surface typed `DaemonError`,
    not string-only failures.

### 9.5 Language-specific Shape

| Concept | Rust | C ABI | Go | Python | Node/TS | Java | Swift |
| --- | --- | --- | --- | --- | --- | --- | --- |
| daemon handle | `DaemonHandle` struct | `easynet_daemon_t*` | `*DaemonHandle` | `DaemonHandle` | `DaemonHandle` | `DaemonHandle` | `DaemonHandle` |
| runtime client | `RuntimeClient` | `easynet_runtime_t*` | `*runtime.Client` | `RuntimeClient` | `RuntimeClient` | `RuntimeClient` | `RuntimeClient` |
| builder | owned builder | `easynet_invocation_builder_t*` | pointer receiver builder | mutable object | chainable class | builder class | value or class builder |
| async stream | `Stream<Item=Event>` | polling/callback handle | channel/iterator | async iterator | `AsyncIterable` | `Flow.Publisher` or iterator | `AsyncSequence` |
| close | `Drop` + explicit close | explicit destroy | `Close()` | context manager + `close` | `close()` | `close()` / `AutoCloseable` | `close()` / `deinit` |
| cancellation | cancellation token | cancel function | `context.Context` | task cancellation + explicit cancel | `AbortSignal` + explicit cancel | `CompletableFuture.cancel` + explicit cancel | task cancellation + explicit cancel |

The C ABI is not exempt from the object model; it implements the same objects as
opaque handles.

### 9.6 Naming Conventions

All languages expose the same objects and state transitions with idiomatic names:

| Target | Naming rule | Example |
| --- | --- | --- |
| Rust | `snake_case` methods, `PascalCase` types | `RuntimeClient::connect_local`, `InvocationBuilder::with_subject` |
| C ABI | `easynet_*` symbol prefix, explicit handles | `easynet_runtime_connect_local`, `easynet_invocation_builder_set_subject` |
| Go | `PascalCase` exported names | `runtime.ConnectLocal`, `InvocationBuilder.WithSubject` |
| Python | `snake_case` methods, `PascalCase` classes | `RuntimeClient.connect_local`, `InvocationBuilder.with_subject` |
| Node / TypeScript | `camelCase` methods, `PascalCase` classes | `RuntimeClient.connectLocal`, `builder.withSubject` |
| Java / Kotlin | `camelCase` methods, `PascalCase` classes | `RuntimeClient.connectLocal`, `InvocationBuilder.withSubject` |
| Swift | `camelCase` methods, `PascalCase` types | `RuntimeClient.connectLocal`, `InvocationBuilder.withSubject` |

Semantic divergence hidden behind naming is a conformance failure. For example,
`submit_signed`, `SubmitSigned`, `submitSigned`, and
`easynet_invocation_submit_signed` must all preserve the same pre-signed caller
material, return the same receipt shape, and emit the same terminal state.

## 10. Normative State Machines

The SDK must model daemon lifecycle separately from invocation execution. A
healthy API process with a dead daemon is not a healthy runtime.

### 10.1 Daemon Lifecycle State Machine

```text
Unknown
  -> Discovered
  -> Starting
  -> ControlReady
  -> InvocationReady
  -> Running
  -> Stopping
  -> Stopped
```

Failure/degraded states:

```text
ConfigInvalid
PermissionDenied
VersionMismatch
ControlOnly
InvocationDown
StartFailed
CrashLoop
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Unknown` | `discover_ok` | `Discovered` | SDK found install paths and expected sockets. |
| `Unknown` | `config_invalid` | `ConfigInvalid` | Required config is missing or invalid. |
| `Discovered` | `start_requested` | `Starting` | SDK may spawn daemon only if policy allows. |
| `Discovered` | `attach_running` | `ControlReady` | Existing daemon control socket is reachable. |
| `Starting` | `control_socket_ready` | `ControlReady` | Control channel is alive but product calls are not ready. |
| `ControlReady` | `invocation_socket_ready` | `InvocationReady` | Invocation endpoint is reachable and version-compatible. |
| `InvocationReady` | `readiness_ok` | `Running` | Directory, identity, and runtime readiness checks passed. |
| `ControlReady` | `invocation_socket_missing` | `ControlOnly` | `/health` may pass liveness, but ability calls must fail readiness. |
| `InvocationReady` | `version_mismatch` | `VersionMismatch` | SDK and daemon protocol versions are incompatible. |
| `Running` | `stop_requested` | `Stopping` | Stop is idempotent. |
| `Stopping` | `process_exited` | `Stopped` | Handle can be dropped safely. |
| any non-terminal | `permission_denied` | `PermissionDenied` | Socket or key material is inaccessible. |
| any non-terminal | `process_crashed` | `CrashLoop` | Retry policy may restart or report failure. |

Invariants:

1. `RuntimeClient` may be created only from `InvocationReady` or `Running`.
2. `ControlReady` is not sufficient for product ability calls.
3. `ControlOnly` MUST be reported as degraded readiness, not success.
4. `stop` is idempotent.
5. `detach` never sends a stop signal.
6. Version mismatch is terminal for that client instance.

### 10.2 Client Connection State Machine

```text
Idle -> Resolving -> Connecting -> Ready -> Closed
                         |          |
                         v          v
                      Failed     Degraded -> Reconnecting -> Ready
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Idle` | `connect` | `Resolving` | Resolve socket path, protocol version, and auth material. |
| `Resolving` | `resolved` | `Connecting` | Begin daemon transport connection. |
| `Connecting` | `handshake_ok` | `Ready` | SDK can send invocation traffic. |
| `Connecting` | `handshake_failed` | `Failed` | Error includes retry hint. |
| `Ready` | `transport_lost` | `Degraded` | In-flight calls receive typed transport errors or resume if allowed. |
| `Degraded` | `retry_allowed` | `Reconnecting` | Reconnect policy starts. |
| `Reconnecting` | `handshake_ok` | `Ready` | New calls may continue. |
| any | `close` | `Closed` | Explicit close wins over reconnect. |

Invariants:

1. New invocation submissions require `Ready`.
2. Reconnect is a transport concern; it must not silently replay a
   non-idempotent invocation unless the invocation carries an idempotency key
   and replay policy.
3. `Closed` is terminal.
4. `Failed` may be retried only by constructing or reopening a client.

### 10.3 Invocation Draft, Signing, and Submission State Machine

This SDK state machine wraps, but does not redefine, Axon's invocation state
machine. Axon runtime states remain:

```text
ACCEPTED -> ADMITTED -> DISPATCHED -> RUNNING -> COMPLETED
                                               -> FAILED
                                               -> TIMED_OUT
                                               -> CANCELLED
```

Daemon SDK adds pre-runtime states:

```text
Draft -> Prepared -> Signed -> Submitted -> Accepted -> Admitted
  -> Dispatched -> Running -> Completed
                            -> Failed
                            -> TimedOut
                            -> Cancelled
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Draft` | `prepare` | `Prepared` | All seven Invocation fields are complete and canonicalized. |
| `Draft` | `invoke` | `Submitted` | Convenience path prepares/signs/submits according to local policy. |
| `Prepared` | `sign` | `Signed` | Signature covers canonical bytes. |
| `Prepared` | `submit_unsigned_allowed` | `Submitted` | Only if daemon policy explicitly supports local signing. |
| `Signed` | `submit` | `Submitted` | Caller signature is preserved. |
| `Submitted` | `accepted` | `Accepted` | Daemon accepted request envelope. |
| `Accepted` | `admitted` | `Admitted` | Runtime admission succeeded. |
| `Admitted` | `dispatched` | `Dispatched` | Callee dispatch selected. |
| `Dispatched` | `running` | `Running` | Ability execution started. |
| `Running` | `completed` | `Completed` | Terminal success with receipt. |
| non-terminal | `failed` | `Failed` | Terminal failure with typed error and receipt if available. |
| non-terminal | `timed_out` | `TimedOut` | Terminal timeout. |
| non-terminal | `cancelled` | `Cancelled` | Terminal cancellation. |

Invariants:

1. `Draft` must expose caller, callee, ability descriptor, subject, nonce,
   causal context, and args before prepare/submit.
2. `Prepared` is immutable.
3. `Signed` is immutable.
4. `submit_signed` must not re-sign or mutate caller signature material.
5. Terminal states are monotonic.
6. Receipt chain verification must refer to terminal state and canonical
   invocation material.
7. SDK convenience methods may skip user-visible intermediate objects only if
   tracing/debug APIs can reveal the same states.

### 10.4 Stream State Machine

```text
Opening -> Open -> TerminalFrameSeen -> Draining -> Closed
               |                         |
               v                         v
            Failed                    Cancelled
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Opening` | `stream_ready` | `Open` | First stream metadata accepted. |
| `Open` | `data_frame` | `Open` | Preserve daemon event order. |
| `Open` | `terminal_frame` | `TerminalFrameSeen` | No new data frames may be accepted after this point. |
| `TerminalFrameSeen` | `drain_complete` | `Draining` | Client drains buffered frames. |
| `Draining` | `close` | `Closed` | Terminal receipt available. |
| non-terminal | `cancel` | `Cancelled` | Cancel request sent to daemon/runtime. |
| non-terminal | `transport_error` | `Failed` | Error includes whether resume is possible. |

Invariants:

1. Stream events are ordered per invocation.
2. Terminal frame appears at most once.
3. `Closed`, `Cancelled`, and `Failed` are terminal.
4. Backpressure behavior must be documented per language.
5. Stream queues must be bounded by named constants.

### 10.5 Bidirectional Session State Machine

```text
Created -> Opening -> Open -> HalfClosedLocal -> Terminal -> Closed
                         |          |
                         v          v
                  HalfClosedRemote  Cancelled
                         |
                         v
                      Terminal
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Created` | `send_frame0` | `Opening` | Frame0 contains invocation/session metadata. |
| `Opening` | `session_accepted` | `Open` | Daemon accepted bidi session. |
| `Open` | `send_frame` | `Open` | Frame ordering is preserved. |
| `Open` | `close_send` | `HalfClosedLocal` | Client will not send more frames. |
| `Open` | `remote_close_send` | `HalfClosedRemote` | Remote will not send more frames. |
| `HalfClosedLocal` | `remote_terminal` | `Terminal` | Remote completes. |
| `HalfClosedRemote` | `close_send` | `Terminal` | Both sides closed send path. |
| non-terminal | `cancel` | `Cancelled` | Terminal local cancellation. |
| `Terminal` | `close` | `Closed` | All receipts/events drained. |

Invariants:

1. Frame0 is required and sent exactly once.
2. Local close-send is not the same as cancel.
3. Session terminal receipt must be observable before final close where daemon
   provides it.
4. Dropping a local object must not silently pretend remote terminal success.
5. Send and receive queues must be bounded by named constants.

### 10.6 Directory Subscription State Machine

```text
Opening -> CatchingUp -> Live -> Resuming -> Live
                         |             |
                         v             v
                       Closed        Failed
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Opening` | `snapshot_start` | `CatchingUp` | Initial catalog snapshot begins. |
| `CatchingUp` | `snapshot_complete` | `Live` | Live delta stream starts after snapshot cursor. |
| `Live` | `transport_lost` | `Resuming` | SDK attempts cursor-based resume. |
| `Resuming` | `resume_ok` | `Live` | No duplicate events after resume cursor. |
| `Resuming` | `resume_failed` | `Failed` | Caller must reopen full subscription. |
| any non-terminal | `close` | `Closed` | Caller intentionally closed subscription. |

Invariants:

1. Snapshot events precede live delta events.
2. Resume must not duplicate a committed event id.
3. Directory event types must be stable across languages.
4. Backend read models must be able to rebuild from snapshot plus deltas.

### 10.7 Aggregate Fan-out State Machine

Ordinary SDK list methods do not fan out. When a caller explicitly needs
fleet-wide aggregation, the SDK invokes a named aggregate ability hosted by the
daemon or hub.

```text
Planned -> Dispatching -> Collecting -> Completed
                              |             |
                              v             v
                           Partial       Failed
                              |
                              v
                           TimedOut
```

Transitions:

| From | Event | To | Rule |
| --- | --- | --- | --- |
| `Planned` | `invoke_aggregate_ability` | `Dispatching` | One parent Invocation enters daemon/hub aggregate ability. |
| `Dispatching` | `child_invocations_started` | `Collecting` | Child Invocations carry parent receipt/causal context. |
| `Collecting` | `all_children_terminal` | `Completed` | Result includes child receipt refs. |
| `Collecting` | `some_children_terminal_some_failed` | `Partial` | Partial result includes per-target errors. |
| `Collecting` | `deadline_elapsed` | `TimedOut` | Timeout result includes completed child receipt refs. |
| non-terminal | `aggregate_failed` | `Failed` | Parent receipt records aggregate failure. |

Invariants:

1. Fan-out concurrency is bounded by an explicit daemon-side limit.
2. Fan-out deadline is bounded by the parent Invocation timeout.
3. Partial results are explicit and typed.
4. Each child call is an Invocation with its own receipt.
5. SDK language facades do not run their own governed fan-out loops.

## 11. Daemon Lifecycle API

Lifecycle APIs manage or attach to a local `easynet-daemon` process. They do not
perform ability dispatch.

Required Go shape:

```go
type Mode string

const (
    ModeDevice Mode = "device"
    ModeHub    Mode = "hub"
    ModeBoth   Mode = "both"
)

type StartConfig struct {
    Mode        Mode
    Realm       string
    NodeID      string
    HomeDir     string
    DaemonBin   string
    LogPath     string
    Detached    bool
    Env         map[string]string

    UDSPath     string
    ListenTCP   string
    TLSCertPath string
    TLSKeyPath  string
    HubEndpoint string
    TrustPath   string
}

type Endpoints struct {
    ControlEndpoint    string
    InvocationEndpoint string
    PublicEndpoint     string
}

type Handle interface {
    Status(ctx context.Context) (DaemonStatus, error)
    Endpoints() Endpoints
    OpenClient(ctx context.Context, opts runtime.ConnectOptions) (*runtime.Client, error)
    Stop(ctx context.Context) error
    Detach() error
}

func Start(ctx context.Context, cfg StartConfig) (Handle, error)
func Attach(ctx context.Context, opts AttachOptions) (Handle, error)
func Discover(ctx context.Context, opts DiscoverOptions) (Endpoints, error)
func ConnectLocal(ctx context.Context, opts runtime.ConnectOptions) (*runtime.Client, error)
```

Requirements:

1. `Start` MUST return only after both control and Invocation endpoints are
   accepting connections.
2. `Attach` MUST fail closed when a control endpoint is alive but the Invocation
   endpoint is down.
3. `Discover` MUST prefer the daemon-advertised `invocation_endpoint` over
   hard-coded `~/.easynet/daemon.sock`.
4. `ModeDevice` MUST NOT accept a public TCP listener.
5. Hub or both mode with `ListenTCP` MUST require TLS material or an explicit
   daemon-side provisioning path.
6. `Detach` MUST NOT stop the daemon.
7. `Stop` MUST be idempotent and authority-checked.

## 12. Runtime Client API

Runtime APIs submit complete Invocations to the daemon.

Required Go shape:

```go
type Client struct{}

type ConnectOptions struct {
    Endpoint        string
    ControlPath     string
    DialTimeout     time.Duration
    InvokeTimeout   time.Duration
    MaxMessageBytes int
    Signer          identity.Signer
    Reconnect       bool
}

func Connect(ctx context.Context, opts ConnectOptions) (*Client, error)
func (c *Client) Health(ctx context.Context) (RuntimeHealth, error)
func (c *Client) NewInvocation() *invocation.Builder
func (c *Client) Invoke(ctx context.Context, inv invocation.Invocation) (invocation.Result, error)
func (c *Client) Prepare(ctx context.Context, draft invocation.Draft, opts invocation.PrepareOptions) (invocation.PreparedInvocation, invocation.SigningMaterial, error)
func (c *Client) SubmitSigned(ctx context.Context, signed invocation.SignedInvocation) (invocation.Handle, error)
func (c *Client) InvokeStream(ctx context.Context, inv invocation.Invocation) (session.Stream, error)
func (c *Client) OpenBidi(ctx context.Context, inv invocation.Invocation, streams []session.StreamDescriptor) (session.Bidi, error)
func (c *Client) Close() error
```

Requirements:

1. `Client` MUST hide gRPC, UDS, generated protocol types, reconnect state, and
   frame-zero construction.
2. `Invoke` MUST return an SDK `invocation.Result`, not raw proto.
3. `InvokeStream` MUST expose SDK `session.Stream` events and terminal state,
   not raw `InvokeStreamChunk`.
4. `OpenBidi` MUST send a correct frame0 before returning the session.
5. Reconnecting clients MUST invalidate only transport-broken connections, not
   application-level invocation failures.
6. Reconnect MUST NOT replay non-idempotent invocations unless replay policy and
   idempotency key are explicit in the Invocation.

## 13. Invocation API

The Invocation package is the SDK's load-bearing contract.

Required Go shape:

```go
type URA string
type DescriptorRef string // ability_ura@descriptor_version

type Invocation struct {
    CallerURA     URA
    CalleeURA     URA
    DescriptorRef DescriptorRef
    SubjectURA    URA
    Nonce         [16]byte
    Causal        CausalContext
    Args          Payload
    ContentType   string

    Metadata      map[string]string
    Authority     AuthorityProof
    CallerSig     *CallerSignature
    Timeout       time.Duration
    Idempotency   *IdempotencyPolicy
}

type Payload struct {
    JSON        any
    Raw         []byte
    ContentType string
}

type Builder struct{}

func New(caller, callee URA, descriptor DescriptorRef, subject URA) *Builder
func (b *Builder) JSON(v any) *Builder
func (b *Builder) Bytes(contentType string, raw []byte) *Builder
func (b *Builder) Nonce(n [16]byte) *Builder
func (b *Builder) FreshNonce() *Builder
func (b *Builder) CausalNone(reason string) *Builder
func (b *Builder) CausalScalar(ref ReceiptRef) *Builder
func (b *Builder) CausalVector(refs []ReceiptRef) *Builder
func (b *Builder) CausalMerkle(root [32]byte, proofURA URA) *Builder
func (b *Builder) Authority(a AuthorityProof) *Builder
func (b *Builder) CallerSignature(sig CallerSignature) *Builder
func (b *Builder) Timeout(d time.Duration) *Builder
func (b *Builder) Idempotency(policy IdempotencyPolicy) *Builder
func (b *Builder) Metadata(k, v string) *Builder
func (b *Builder) Inspect() Draft
func (b *Builder) Build() (Invocation, error)
```

Requirements:

1. `Build` MUST reject empty caller, callee, descriptor, subject, args content
   type, invalid nonce length, and ambiguous payload forms.
2. `DescriptorRef` MUST bind ability identity and descriptor version.
3. `Payload` MUST support JSON and raw bytes without re-encoding raw payloads.
4. SDKs MUST NOT publish a public `InvokeAbility(name, args)` API that bypasses
   caller/callee/subject/nonce/causal visibility.
5. `Inspect` MUST show the full draft tuple and defaults that have already been
   filled.

## 14. Prepare, Sign, Submit

This API supports browser/user signing flows and backend-mediated signed submit.

Required Go shape:

```go
type Draft struct {
    CallerURA     URA
    CalleeURA     URA
    DescriptorRef DescriptorRef
    SubjectURA    URA
    Args          Payload
    Causal        CausalContext
    Metadata      map[string]string
    Timeout       time.Duration
    Idempotency   *IdempotencyPolicy
}

type PrepareOptions struct {
    ResolveDescriptor bool
    FillNonce         bool
    RequireUserSig    bool
    ExpiresIn         time.Duration
}

type PreparedInvocation struct {
    Invocation    Invocation
    RequestID     string
    DescriptorRef DescriptorRef
    ExpiresAt     time.Time
}

type SigningMaterial struct {
    CanonicalBytes []byte
    ArgsDigestHex  string
    NonceBase64    string
    SignedFields   []string
}

type SignedInvocation struct {
    Prepared PreparedInvocation
    Signature CallerSignature
}

func (c *Client) Prepare(ctx context.Context, draft invocation.Draft, opts invocation.PrepareOptions) (invocation.PreparedInvocation, invocation.SigningMaterial, error)
func (c *Client) SubmitSigned(ctx context.Context, signed invocation.SignedInvocation) (invocation.Handle, error)
```

Requirements:

1. Canonical bytes MUST be computed inside the CLI SDK core or daemon-owned
   Axon adapter by delegating to Axon internals, never by product code.
2. `SigningMaterial` MUST be stable across Go, Python, Rust, and C ABI bindings
   for the same Invocation.
3. `SubmitSigned` MUST preserve the caller's signature and public key material;
   it MUST NOT replace a user signature with the backend's hub key.
4. Prepared invocations MUST expire or carry enough metadata for callers to
   reject stale browser signatures.

## 15. Authority API

The SDK MUST expose authority metadata as typed values.

Required Go shape:

```go
type Signer interface {
    PublicKey() []byte
    Sign(ctx context.Context, msg []byte) ([]byte, error)
}

type SessionAuthority struct {
    IssuerURA   invocation.URA
    SubjectURA  invocation.URA
    AudienceURA invocation.URA
    Scopes      []string
    ExpiresAt   time.Time
    Signature   []byte
}

type DelegationProof struct {
    IssuerURA   invocation.URA
    SubjectURA  invocation.URA
    AudienceURA invocation.URA
    Scopes      []string
    ExpiresAt   time.Time
    Signature   []byte
}

func NewSessionAuthority(ctx context.Context, signer Signer, req SessionAuthorityRequest) (SessionAuthority, error)
func NewDelegationProof(ctx context.Context, signer Signer, req DelegationRequest) (DelegationProof, error)
```

Requirements:

1. `SessionAuthority` and `DelegationProof` MUST be mutually exclusive on one
   Invocation.
2. Authority metadata MUST bind issuer, subject, audience, scopes, expiry, and
   signature.
3. Builder APIs MUST reject ambiguous authority.
4. AuthorityBinding must govern both advertisement and invocation permission.

## 16. Directory, Catalog, and Listing API

Directory APIs project daemon read models. They may invoke daemon abilities such
as `namespace.resolve`, `namespace.proxy_resolve`, `federation.resolve`, or
`federation.list_user_devices` internally, but product consumers do not need to
know those carrier names.

Required Go shape:

```go
type Client struct { Runtime *runtime.Client }

func (d *Client) Resolve(ctx context.Context, q ResolveQuery) (ResolveAnswer, error)
func (d *Client) ListDevices(ctx context.Context, q DeviceQuery) (Page[Device], error)
func (d *Client) ListAgents(ctx context.Context, q AgentQuery) (Page[Agent], error)
func (d *Client) ListAbilities(ctx context.Context, q AbilityQuery) (Page[AbilityDescriptor], error)
func (d *Client) Subscribe(ctx context.Context, f DirectoryFilter) (DirectoryStream, error)
```

Requirements:

1. Directory results MUST expose URAs for owners, devices, agents, and
   abilities.
2. Directory errors MUST use typed SDK errors, not human string parsing.
3. Event subscriptions MUST expose SDK event DTOs, not raw federation proto.
4. Every list method MUST support cursor pagination.
5. Every list method MUST define `DefaultPageSize` and `MaxPageSize` as named
   constants in each implementation.
6. A list method returning all rows without pagination is forbidden at public
   SDK boundaries.

### 16.1 Complexity Contract

Ordinary list methods are read-model queries, not distributed fan-out.

| Method | Required data source | Complexity target | Forbidden implementation |
| --- | --- | --- | --- |
| `ListDevices` | daemon/hub directory read model indexed by realm/user/device | `O(page_size + filter_cost)` | dialing every device |
| `ListAgents` | local hosted-agent registry or hub directory projection | `O(page_size + filter_cost)` | calling each agent |
| `ListAbilities` | ability catalog projection keyed by owner/descriptor/filter | `O(page_size + filter_cost)` | calling `meta.list_abilities` on every agent by default |
| `Resolve` | indexed directory/namespace lookup | `O(log n + result_count)` or documented equivalent | scanning all devices for exact URA lookup |
| `Subscribe` | snapshot cursor plus delta log | `O(snapshot_page + delta_count)` per page | replaying full catalog for each delta |

`filter_cost` must be bounded by indexed predicates or by a documented scan over
a local in-memory snapshot. It must not hide network fan-out.

### 16.2 Fan-out and Facade Rules

1. Language SDK facades MUST NOT fan out governed ability calls across devices,
   agents, or abilities on their own.
2. Fleet-wide aggregation belongs in a named daemon/hub aggregate ability, such
   as `aggregate.list_abilities_catalog`.
3. Aggregate abilities MUST create child Invocations for each governed child
   call.
4. Aggregate abilities MUST expose max concurrency, deadline, page size,
   partial-result semantics, and per-target error summaries.
5. SDK facade methods MAY call one aggregate ability. That is still one parent
   Invocation from the SDK's point of view.
6. Facade helpers MUST be named to reveal aggregation, for example
   `AggregateAbilities`, not `ListAbilities`.
7. A default `ListAbilities` call MUST return the catalog projection already
   known to the daemon/hub, not trigger live remote discovery.

## 17. Identity API

The SDK MUST expose identity helpers needed by daemon clients and product
backends while keeping keyring policy daemon-owned.

Required capabilities:

1. Load or initialize local Hub identity for a realm.
2. Load device identity from daemon/keyring context.
3. Construct URAs through SDK-owned builders.
4. Register, list, and revoke user signing keys through daemon identity
   abilities.
5. Provide `Signer` implementations for local Ed25519 identities.

The SDK MUST NOT expose backend JWT/OAuth/session concepts.

## 18. Stream, Bidi, and Session API

SDK sessions hide raw daemon frame details.

Required Go shape:

```go
type Stream interface {
    Recv(ctx context.Context) (StreamEvent, error)
    Close() error
    Cancel(ctx context.Context) error
}

type Bidi interface {
    Send(ctx context.Context, frame UpFrame) error
    Recv(ctx context.Context) (DownFrame, error)
    CloseSend() error
    Close() error
    Cancel(ctx context.Context) error
}
```

Requirements:

1. Streams MUST make terminal state explicit.
2. Bidi sessions MUST hide frame0 construction and sequence numbering.
3. EOF, terminal receipt, cleanup receipt, timeout, and transport cancellation
   MUST be distinguishable.
4. Long-lived streams MUST support caller cancellation through context and an
   explicit close/cancel method.
5. Stream and bidi queues MUST be bounded by named constants.

## 19. Files, Terminal, and Remote Desktop Wrappers

These wrappers are convenience APIs over complete Invocation and bidi sessions.
They MUST NOT be the only way to reach those abilities.

Required capabilities:

```go
func OpenFileTransfer(ctx context.Context, c *runtime.Client, req FileTransferRequest) (FileTransfer, error)
func OpenTerminal(ctx context.Context, c *runtime.Client, req TerminalRequest) (TerminalSession, error)
func OpenRemoteDesktop(ctx context.Context, c *runtime.Client, req RemoteDesktopRequest) (RemoteDesktopSession, error)
```

Requirements:

1. Wrappers MUST still produce a complete Invocation internally.
2. Wrappers MUST surface admission failure, routing failure, terminal failure,
   timeout, and client cancellation as typed errors.
3. Backend HTTP/WS bridges MAY consume these wrappers, but wrappers MUST NOT
   import backend packages.

## 20. Device, Plugin, AbilityImpl, and Mission APIs

These APIs primarily serve CLI, desktop apps, local host apps, and EasyRemote.
Backend usage is allowed but not required.

Required capabilities:

```go
func JoinHub(ctx context.Context, req JoinRequest) (JoinResult, error)
func LeaveHub(ctx context.Context, req LeaveRequest) error

func InstallPlugin(ctx context.Context, source string, opts InstallOptions) (PluginInstallResult, error)
func EnableAbilityImpl(ctx context.Context, id AbilityImplID) error
func DisableAbilityImpl(ctx context.Context, id AbilityImplID) error

func RunMission(ctx context.Context, req MissionRequest) (MissionResult, error)
func TrackMission(ctx context.Context, id MissionID) (MissionStatus, error)
func CancelMission(ctx context.Context, id MissionID) error
```

Requirements:

1. Plugin, skill, and host-process management belongs to implementation-resource
   management, not protocol ability identity.
2. Mission/EAL helpers MUST create child Invocations for ability calls rather
   than redefining Invocation semantics.
3. Composite ability orchestration belongs in Mission/EAL or daemon-owned
   aggregate abilities unless the flow is trivial, latency-critical, or
   session-oriented.

## 21. Health and Diagnostics API

SDK health MUST separate product process liveness from runtime readiness.

Required shape:

```go
type RuntimeHealth struct {
    DaemonVersion       string
    SDKVersion          string
    Mode                daemon.Mode
    Realm               string
    PID                 int
    ControlEndpoint     string
    InvocationEndpoint  string
    PublicEndpoint      string
    ControlReady        bool
    InvocationReady     bool
    PublicListenerReady bool
    TrustReady          bool
    DirectoryReady      bool
    RuntimeReady        bool
    LastError           *RuntimeError
}
```

Requirements:

1. Local health MUST check the Invocation endpoint, not only control discovery.
2. Hub health SHOULD check public TCP+TLS listener readiness when configured.
3. Health MUST expose typed readiness flags so product backends can distinguish
   "API alive but runtime unavailable" from successful runtime readiness.
4. `RuntimeReady=false` with `ControlReady=true` MUST be possible and visible.

## 22. Error Model

All bindings MUST map daemon, transport, admission, timeout, cancellation, and
protocol failures into typed errors.

Required Go shape:

```go
type ErrorCode string

const (
    ErrDaemonOffline     ErrorCode = "DAEMON_OFFLINE"
    ErrPermissionDenied  ErrorCode = "PERMISSION_DENIED"
    ErrAdmissionDenied   ErrorCode = "ADMISSION_DENIED"
    ErrAbilityNotFound   ErrorCode = "ABILITY_NOT_FOUND"
    ErrRouteUnavailable  ErrorCode = "ROUTE_UNAVAILABLE"
    ErrTimeout           ErrorCode = "TIMEOUT"
    ErrCancelled         ErrorCode = "CANCELLED"
    ErrInvalidInvocation ErrorCode = "INVALID_INVOCATION"
    ErrProtocolMismatch  ErrorCode = "PROTOCOL_MISMATCH"
    ErrVersionMismatch   ErrorCode = "VERSION_MISMATCH"
    ErrControlOnly       ErrorCode = "CONTROL_ONLY"
)

type RuntimeError struct {
    Code       ErrorCode
    Stage      string
    Message    string
    Retryable  bool
    ReceiptURA string
    Cause      error
}
```

Requirements:

1. Product consumers MUST NOT parse human-readable daemon error strings.
2. Errors SHOULD preserve receipt URA or invocation id when the daemon provides
   one.
3. Retriability MUST be explicit; do not infer it from string fragments.
4. Error stages MUST align with the state machines in section 10.

## 23. Wire JSON Schemas

The C ABI, Python SDK, and any binding that does not use generated Go/Rust types
MUST accept the same Invocation JSON shape:

```json
{
  "caller_ura": "easynet:///r/example/agent/alice",
  "callee_ura": "easynet:///r/example/device/dev-a",
  "descriptor_ref": "easynet:///r/example/device/dev-a/ability/observe.health@1.0.0",
  "subject_ura": "easynet:///r/example/device/dev-a",
  "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
  "causal_context": {"form": "none"},
  "args": {},
  "content_type": "application/json",
  "metadata": {},
  "caller_signature": {
    "algorithm": "ed25519",
    "signature_base64": "...",
    "signer_public_key_base64": "..."
  }
}
```

For non-JSON payloads, callers pass `arguments_base64` and `content_type`
instead of `args`.

The SDK MUST reject:

1. Both `args` and `arguments_base64`.
2. Neither `args` nor `arguments_base64`.
3. Empty tuple fields.
4. Missing descriptor version in descriptor-bound calls.
5. Non-16-byte nonces.
6. Ambiguous authority metadata.
7. Unknown state-machine terminal names.
8. Page requests above `MaxPageSize`.

## 24. Transport Strategy by Language

The public API is stable even when transport implementation changes.

| Target | Default transport | Allowed fallback | Forbidden exposure |
| --- | --- | --- | --- |
| Rust | native daemon SDK core plus daemon invocation UDS client | direct in-process test adapter | public Axon client/proto types in stable product API |
| C ABI | opaque handles over the Rust SDK core | none | raw Rust/Axon pointers or generated proto structs |
| Go | thin daemon local-transport facade over generated schemas | C ABI for canonical/signing helper calls if needed | `easynet.run/axon/*` imports in public packages or reimplemented Axon algorithms |
| Python | C ABI bridge over Rust SDK core | native daemon transport only for request marshalling | CLI text scraping or reimplemented Axon algorithms |
| Node / TS | N-API/C ABI bridge or thin daemon transport facade | child-process CLI only for dev diagnostics | shelling out for normal invocation or reimplemented Axon algorithms |
| Java / JVM | JNI/C ABI bridge or thin daemon transport facade | none for normal invocation | generated Axon protobufs in public API or reimplemented Axon algorithms |
| Swift | Swift package over C ABI or thin daemon transport facade | none for normal invocation | Axon symbols in public API or reimplemented Axon algorithms |

Command-line binaries are consumers of the SDK. The SDK MUST NOT be a wrapper
around terminal text output.

For this table, "thin daemon transport facade" means request marshalling,
connection lifecycle, retry classification, and idiomatic async adaptation. It
does not include canonical Invocation material generation, Axon receipt-chain
verification, admission semantics, or Axon stream/bidi state machines. Those
semantics are delegated to Axon through the native Rust daemon SDK core or the
daemon runtime adapter.

## 25. Language Capability Parity

Capability parity is measured by behavior, not by identical function names.

| Module | Rust | C ABI | Go | Python | Node/TS | Java/JVM | Swift | Requirement |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| ABI/version discovery | yes | yes | yes | yes | yes | yes | yes | P0 for Rust/C/Go/Python; P1 for others |
| Daemon lifecycle attach/start/stop | yes | yes | yes | yes | yes | yes | yes | P0 for Rust/C/Go/Python |
| Runtime connect over UDS | yes | yes | yes | yes | yes | yes | yes | P0 for Rust/C/Go/Python |
| Complete Invocation builder | yes | yes | yes | yes | yes | yes | yes | P0 all designed, staged release |
| Prepare/sign/submit | yes | yes | yes | yes | yes | yes | yes | P0 for Rust/C/Go/Python |
| Unary invoke | yes | yes | yes | yes | yes | yes | yes | P0 for Rust/C/Go/Python |
| Server stream | yes | yes | yes | yes | yes | yes | yes | P1 if not needed for first backend cutover |
| Bidi session | yes | yes | yes | yes | yes | yes | yes | P1 |
| Directory/catalog | yes | yes | yes | yes | yes | yes | yes | P0 for backend read models |
| Identity/signer | yes | yes | yes | yes | yes | yes | yes | P0 prepare/sign parity |
| Health/diagnostics | yes | yes | yes | yes | yes | yes | yes | P0 backend readiness |
| File transfer wrapper | yes | yes | yes | yes | yes | yes | yes | P1 |
| Terminal wrapper | yes | yes | yes | yes | yes | yes | yes | P1 |
| Remote desktop wrapper | yes | yes | yes | yes | yes | yes | yes | P2 |
| Device/plugin lifecycle | yes | yes | yes | yes | yes | yes | yes | P1 |
| Mission lifecycle | yes | yes | yes | yes | yes | yes | yes | P1 |
| Conformance runner | yes | yes | yes | yes | yes | yes | yes | Required before language marked stable |

The matrix is aspirational for P1/P2 languages but normative for shape. A
language may ship later; it must not ship with a different concept model.

## 26. SDK Interface Spec Files to Add

This document is the requirements source. The SDK work should add these
Axon-style files under `sdk/`:

| File | Purpose |
| --- | --- |
| `sdk/README.md` | Entry point for SDK users and maintainers. |
| `sdk/SDK_INTERFACE_SPEC.md` | Normative OOP API for every supported language. |
| `sdk/SDK_PARITY.md` | Feature and behavior parity matrix by language. |
| `sdk/CONFORMANCE_SUITE.md` | How to run golden fixtures against every SDK. |
| `sdk/schemas/*.schema.json` | Canonical public JSON shapes. |
| `sdk/conformance/cases/*.yaml` | Language-neutral behavior tests. |
| `sdk/conformance/fixtures/*` | Canonical invocation, receipt, event, and error fixtures. |

`SDK_INTERFACE_SPEC.md` MUST include:

1. Object model and method list for each core object.
2. Language naming mapping.
3. Sync/async behavior per method.
4. Ownership and lifetime rules.
5. Thread-safety rules.
6. Error taxonomy.
7. State machine references.
8. Minimum examples for unary, stream, bidi, directory, aggregate fan-out, and
   health.

`SDK_PARITY.md` MUST include:

1. Language support tier.
2. Feature support table.
3. Platform support table.
4. Known gaps and deadlines.
5. API stability level per package.
6. Packaging status per registry.

`CONFORMANCE_SUITE.md` MUST include:

1. Test fixture format.
2. Runner contract.
3. Required cases before a language can be marked stable.
4. CI commands for Rust, C ABI, Go, Python, Node, Java, and Swift.
5. Rules for adding a new language.

## 27. Conformance Requirements

Before backend cutover, the repo MUST provide language-neutral conformance cases
for the P0 languages. Before P1 language release, each P1 SDK must pass the same
case set.

Minimum conformance cases:

1. `version/abi_compatible`: SDK detects compatible daemon/ABI version.
2. `version/abi_incompatible`: SDK returns typed version mismatch.
3. `daemon/control_only`: control socket up but invocation socket down reports
   degraded runtime readiness.
4. `daemon/permission_denied`: socket permission failure maps to
   `PermissionDenied`.
5. `invocation/complete_tuple`: builder rejects missing caller, callee, ability,
   subject, nonce, causal context, or args.
6. `invocation/canonical_material`: all languages produce identical canonical
   signing bytes for the same draft.
7. `invocation/presigned_submit`: `submit_signed` preserves caller signature.
8. `invocation/terminal_monotonicity`: terminal states cannot transition again.
9. `authority/mutual_exclusion`: local daemon signing and caller pre-signing
   rules are enforced.
10. `stream/order_terminal`: stream data frames are ordered and terminal appears
    once.
11. `stream/backpressure_bound`: stream queue overflow produces typed backpressure
    or cancellation behavior.
12. `bidi/frame0_required`: bidi session without frame0 is rejected.
13. `bidi/close_send_not_cancel`: local half-close is distinguishable from
    cancel.
14. `directory/snapshot_then_live`: directory subscription emits snapshot before
    live deltas.
15. `directory/list_pagination`: list APIs reject requests above max page size.
16. `directory/no_default_fanout`: ordinary list APIs do not perform per-agent
    live remote calls.
17. `aggregate/partial_result`: aggregate fan-out returns typed partial results
    and child receipt refs.
18. `error/retry_hint`: retryable and non-retryable errors are classified
    consistently.
19. `health/api_vs_runtime`: API liveness and daemon runtime readiness are
    distinct.
20. `backend/import_ban`: backend cannot import Axon public packages after
    cutover.

Each case should have:

```yaml
id: invocation/complete_tuple
description: Builder rejects incomplete seven-tuple drafts.
given:
  draft:
    caller: "easynet:///r/acme/agent/alice"
    callee: "easynet:///r/acme/device/edge-01"
    descriptor_ref: "easynet:///r/acme/device/edge-01/ability/fs.read@1.0.0"
    subject_ura: null
    nonce_base64: "AQIDBAUGBwgJCgsMDQ4PEA=="
    causal_context: {"form": "none"}
    args: {"path": "/tmp/a.txt"}
expect:
  error_code: "INVALID_INVOCATION"
  missing_fields: ["subject_ura"]
```

## 28. EasyRemote Relationship

EasyRemote SHOULD become a high-level consumer of the Python Daemon SDK.

The following EasyRemote responsibilities SHOULD move to the SDK:

1. `libeasynet_cli` loading and ABI version checks.
2. Daemon lifecycle/status wrappers.
3. Complete Invocation JSON codec.
4. URA builders/parsers used for daemon calls.
5. Canonical signing material and caller signature transport.
6. Receipt and typed error projection.
7. Ability/agent/mission daemon control surfaces that are not Python-specific.

The following SHOULD remain in EasyRemote:

1. `ComputeNode`.
2. `@node.register`.
3. `@remote`.
4. Python schema extraction from functions/classes.
5. Warm Python host process integration.
6. `Pipeline` Python DSL.
7. Gallery, examples, and product positioning.

EasyRemote MUST NOT be a dependency of EasyNet backend.

## 29. Backend Migration Requirements

EasyNet backend migration is complete only when:

1. Backend `go.mod` no longer requires `easynet.run/axon/sdk/go`.
2. Backend does not import `easynet.run/axon/*`.
3. Backend does not expose or depend on generated `axon.v1` proto types.
4. Backend runtime injection is named `Runtime` or `DaemonRuntime`, not `Axon`.
5. Backend ability invoke, directory, events, identity, file, terminal, remote
   desktop, and OpenAI compatibility paths all use the CLI Go SDK.
6. Backend tests use `fakeRuntimeClient`, not `fakeAxonClient`.
7. Backend health distinguishes API liveness from daemon runtime readiness.
8. Backend list endpoints use daemon SDK directory pages or named aggregate
   abilities; they do not manually fan out to device ability endpoints.

## 30. Conformance and CI Gates

The SDK repo MUST provide these gates before backend cutover:

1. Golden conformance cases for Invocation JSON to canonical signing material.
2. Rust, C ABI, Go, and Python parity tests for the same Invocation fixture.
3. ABI v3 header/export/version checks.
4. Go SDK import ban: Go SDK packages must not import Axon. Axon dependency is
   contained in the native Rust daemon SDK core and daemon runtime adapter.
5. EasyNet backend import ban: no `easynet.run/axon/*` after cutover.
6. `control.sock` product-call ban: no `Invoke`, `Subscribe`, or `OpenBidi`
   product dispatch over JSON control frames.
7. Facade fan-out ban: ordinary SDK list methods do not run per-target
   governed calls.
8. Live daemon smoke covering unary, stream, bidi, file transfer, and typed
   terminal failure.
9. Health smoke covering daemon down, UDS permission denied, public listener
   down, control-only, version mismatch, and trust not ready.

## 31. Implementation Phases

The migration from the current architecture to the complete SDK architecture
must be staged by semantic boundary.

### 31.1 Phase A - SDK Spec and Scaffold

Deliverables:

1. Add `sdk/README.md`, `SDK_INTERFACE_SPEC.md`, `SDK_PARITY.md`, and
   `CONFORMANCE_SUITE.md`.
2. Add `sdk/schemas/` placeholders for Invocation, receipt, error, health, and
   events.
3. Add `PROJECT_STRUCTURE.md` or update it if it already exists.
4. Add CI checks that spec files and schema fixtures are present.

Exit criteria:

1. Every supported language has an owner and tier.
2. Every SDK object has a documented lifecycle.
3. Every state machine is referenced by the interface spec.

### 31.2 Phase B - Rust Core and C ABI Freeze

Deliverables:

1. Split Rust daemon SDK facade from daemon process internals where needed.
2. Normalize canonical Invocation JSON and signing material generation.
3. Freeze C ABI handle names for daemon, runtime, invocation builder, prepared
   invocation, signed invocation, stream, bidi, directory, identity, health, and
   error.
4. Add ABI version and feature discovery.
5. Wrap any unstable raw Axon proto returns behind SDK DTOs before marking the
   Rust API stable.

Exit criteria:

1. Rust and C ABI pass canonical fixture conformance.
2. C header exposes no Axon symbols.
3. CLI binary uses SDK objects, not duplicate command-only code.
4. Stable Rust daemon SDK public API exposes SDK DTOs, not raw `axon.v1` proto
   message types.

### 31.3 Phase C - Directory and Aggregate Read Model

Deliverables:

1. Define `Page<T>`, cursor, filter, and directory event schemas.
2. Implement paginated `ListDevices`, `ListAgents`, and `ListAbilities`.
3. Define named aggregate abilities for fleet-wide rich catalog reads.
4. Add fan-out bounds, partial-result error taxonomy, and child receipt refs.

Exit criteria:

1. Ordinary list methods satisfy `O(page_size + filter_cost)`.
2. Aggregate methods expose max concurrency and deadlines.
3. Facade fan-out ban passes CI/conformance.

### 31.4 Phase D - Go SDK for EasyNet Backend

Deliverables:

1. Add `sdk/go` package.
2. Implement `ConnectLocal`, `DaemonHandle`, `RuntimeClient`,
   `InvocationBuilder`, `Prepare`, `SubmitSigned`, `Invoke`, `HealthClient`,
   `DirectoryClient`, and typed errors.
3. Add backend fake runtime client and import-ban test.
4. Cut backend runtime adapter from Axon SDK to CLI Go SDK.
5. Ensure Go owns only facade ergonomics, generated DTOs, transport marshalling,
   retries, and typed error mapping; it must not reimplement Axon
   canonicalization, admission, receipt verification, or stream/bidi state
   semantics.

Exit criteria:

1. EasyNet backend has no public Axon imports.
2. Backend `/health` distinguishes API liveness from daemon runtime readiness.
3. Backend ability calls fail clearly when daemon invocation socket is absent.
4. Backend list APIs use directory pages or aggregate abilities, not SDK-side
   fan-out loops.

### 31.5 Phase E - Python SDK and EasyRemote Extraction

Deliverables:

1. Add `sdk/python` package.
2. Move generic daemon client, ABI loading, Invocation codec, URA parsing,
   signing material, receipt projection, and typed errors out of EasyRemote.
3. Keep EasyRemote-specific decorators, `ComputeNode`, `@remote`, `Pipeline`,
   and gallery code inside EasyRemote.

Exit criteria:

1. EasyRemote imports EasyNet Daemon SDK rather than owning daemon transport.
2. Python conformance cases pass.
3. EasyRemote remains a consumer, not a dependency of backend.

### 31.6 Phase F - Node, Java, and Swift SDKs

Deliverables:

1. Add package skeletons and generated schema types.
2. Implement daemon lifecycle, runtime client, Invocation builder, invoke,
   health, directory, and errors.
3. Add async stream and bidi APIs using each language's idioms.

Exit criteria:

1. Each language passes shared conformance cases before being marked stable.
2. Packaging metadata exists for npm, Maven, and Swift Package Manager.
3. No package exposes Axon symbols.

### 31.7 Phase G - Product and CI Enforcement

Deliverables:

1. Backend import-ban enforcement for Axon.
2. Product-call ban over `control.sock`.
3. Runtime readiness smoke tests for daemon down, invocation socket down,
   permission denied, version mismatch, control-only, and trust-not-ready.
4. Release packaging for daemon plus P0 SDKs.

Exit criteria:

1. Production Hub deploy documents `easynet-api.service` and
   `easynet-daemon.service` as separate processes.
2. Ability invocation requires daemon runtime readiness.
3. SDK conformance runs in CI.

## 32. MVP Scope

MVP order:

1. Rust daemon SDK core and C ABI: lifecycle, runtime connection, complete
   Invocation, prepare/sign/submit, typed errors, health.
2. Directory page/query model and no-default-fan-out conformance.
3. Go SDK facade: `ConnectLocal`, `Invocation`, `Prepare`, `SubmitSigned`,
   `Invoke`, `Health`, `Directory`.
4. Reconnecting client, stream, and bidi abstractions.
5. File, terminal, and remote desktop wrappers.
6. Python SDK extraction for EasyRemote.
7. Device join, plugin, and mission lifecycle helpers.
8. Backend cutover and import-ban enforcement.

The minimum stable Rust core is complete Invocation plus signing material plus
daemon client plus typed health and error projection. Daemon lifecycle alone is
not sufficient.

## 33. Open Questions

1. Exact public module path for the Go SDK (`easynet.run/cli/sdk/go` vs
   `easynet.run/easynet-cli/sdk/go`).
2. Exact Python package name (`easynet-runtime`, `easynet-cli-sdk`, or another
   name).
3. Whether the Go facade should use daemon local transport only or link
   `libeasynet_cli` for selected local helpers. Either path must delegate
   canonical/signing/receipt semantics to the Rust daemon SDK core or daemon.
4. Whether ACME/TLS provisioning belongs in SDK lifecycle or remains an
   operator/Gateway convenience.
5. Full receipt fetch and receipt-chain verification API shape once receipt URA
   builders and fetch paths are finalized.
6. Exact default values for `DefaultPageSize`, `MaxPageSize`,
   `MaxFanoutConcurrency`, and aggregate deadline caps. These must be named
   constants with engineering justification before implementation.

## 34. Final Abstraction Statement

The correct public abstraction is:

```text
Daemon SDK = typed, object-oriented, multi-language client for the local
easynet-daemon runtime boundary.
```

The CLI command binary is one consumer of this SDK. EasyNet backend is another.
EasyRemote is another. The SDK is not command-output scraping, not direct Axon
exposure, and not a hidden fan-out engine in language facade code.

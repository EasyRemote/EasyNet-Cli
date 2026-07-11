# SDK Interface Spec

This file is the implementation-facing contract for the EasyNet Daemon SDK.
The requirements source remains `docs/spec/daemon-sdk-requirements-v1.md`; this
file records the staged public API shape that language bindings must project.

## Object Graph

```text
SdkEnvironment
  -> NativeRuntimeHandle
       -> RuntimeClient
       -> HealthClient
       -> IdentityClient
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
       -> ReceiptClient
       -> PublicationClient
       -> HostBindingClient
       -> MissionClient
       -> AdminClient
       -> EventClient
       -> SurfaceClient
       -> CompatibilityClient
       -> HealthClient
```

No public object in this graph may expose raw Axon client/proto/runtime types.

## Runtime Core

| Object | Owns | Terminal operation |
| --- | --- | --- |
| `SdkEnvironment` | process-level SDK initialization and feature discovery | `close` |
| `NativeRuntimeHandle` | one SDK-owned native Runtime, Health, and Identity provider lifecycle | `close` |
| `DaemonHandle` | start, attach, discover, endpoints, stop, detach | `stop`, `detach` |
| `RuntimeClient` | daemon Invocation endpoint session and runtime health | `close` |
| `InvocationBuilder` | mutable seven-tuple construction | `inspect`, `prepare`, `invoke` |
| `InvocationDraft` | immutable complete seven-tuple snapshot | `prepare`, `invoke` |
| `PreparedInvocation` | Axon-delegated canonical signing material | `sign`, `close/free` |
| `SignedInvocation` | submit-ready caller or daemon-signed envelope | `submit`, `close/free` |
| `InvocationHandle` | submitted invocation observation and cancellation | `await_result`, `cancel`, `close` |
| `InvocationResult` | terminal output, error, and receipt projection | none |
| `StreamHandle` | ordered stream frames and terminal event | `close`, `cancel` |
| `BidiSession` | bidirectional frames and terminal event | `close_send`, `close`, `cancel` |

`PreparedInvocation` is not executable. `SignedInvocation` is the only
submit-ready pre-runtime object.

Python's `InvocationWireProjector` is a stateless binding adapter over
`AddressingClient`: it may project a host seven-tuple object to an
`InvocationDraft` or wire DTO, but owns neither Runtime transport nor lifecycle.
It is the object-shaped counterpart of constructing and inspecting an
`InvocationDraft` through the typed Go builder; it does not define a separate
runtime capability.

## Profile Clients

| Profile | Client | Public responsibility |
| --- | --- | --- |
| Directory + Identity | `DirectoryClient`, `IdentityClient` | URA/ref builders through Axon delegation, local identity, paginated directory, subscription |
| Receipt | `ReceiptClient` | fetch, project, verify, and derive causal refs from receipts |
| Publication | `PublicationClient` | ability package/resource refs, deploy/list/show/enable/disable/unpublish |
| Host Binding | `HostBindingClient` | host-stream request/frame/error/terminal codec and output hash |
| Mission | `MissionClient` | EAL/source/file run, track, cancel, mission events |
| Admin + Gateway | `AdminClient` | gateway readiness, hub/device admin, agent lifecycle |
| Events | `EventClient` | directory/device/session/invocation subscriptions and cursors |
| Surface | `SurfaceClient` | page/surface records and public refs |
| Compatibility | `CompatibilityClient` | OpenAI-style model/chat/file adapters over governed daemon abilities |
| Wrappers | file, terminal, remote desktop, browser, media clients | convenience over Runtime Core plus profile DTOs |
| Health | `HealthClient` | readiness and diagnostics |

Profile clients must not become one-method-per-ability protocol forks.
Identity projection may validate/build URAs and AbilityDescriptorRefs through
Axon-delegated helpers, but directory list/subscribe and signing-key lifecycle
remain separate Directory + Identity methods rather than string utilities.
Directory read-model carrier projection may build complete Invocation carriers
for daemon `node.list`, `agent.list`, `meta.list_abilities`, and
`namespace.resolve`; project daemon rows into paginated `DirectoryPage` DTOs;
project daemon `ResolveAnswer` JSON into stable `ResolvedRef` DTOs; and expose
named `DefaultPageSize`/`MaxPageSize` guardrails. It must not perform per-agent
or per-device live fan-out, hide unpaginated all-row reads behind public list
methods, select routes in the SDK, call `federation.resolve` for exact
Directory resolve, or leak the daemon `meta.list_abilities` historical
`agent_ura` parameter as the public owner-filter name.
Receipt carrier/projection may build complete Invocation carriers for daemon
`invocation.history.get`, normalize summary DTOs, and derive causal refs from
explicit receipt facts, but it must not open daemon ledger files directly,
fabricate receipt URAs, or mark summary-only data `verified: true` before an
Axon-backed verifier proves a full receipt.
Publication carrier projection may build daemon-authored local ResourceRefs,
validate ability package manifests, and build complete Invocation JSON for
daemon publication system abilities. It may claim list/show/enable/disable
runtime results only when they execute through daemon read models or equivalent
governed abilities. It must not derive mutation semantics from catalogue rows.
Host Binding codec projection may build host-stream binding DTOs, decode
daemon request envelopes, encode shared item/error/terminal frames, and fold
output hashes. It must not execute product host code, inspect language
decorators, load dependencies, or own warm-host process lifecycle.
Surface carrier/projection may build complete Invocation carriers for daemon
`project_list`, `pages.publish`, `pages.get`, and `pages.unpublish`, normalize
page records, and build public page refs from explicit daemon page facts. It
must not render HTML, own browser auth, call backend product routes, or open
page folders directly.
Mission carrier/status/event projection may build complete Invocation carriers
for daemon `mission.run`, `mission.track`, and `mission.cancel`, read explicit
local EAL source files for `RunFile`, normalize daemon mission results into
`MissionStatus`, and project daemon mission timeline replay into
`MissionEventPage` with explicit sequence cursors. It must not execute EAL,
create a second mission runtime, read mission run directories from language
facades, infer cursors from timestamps or array positions, or fabricate child
receipt refs for receipt-less steps.
Events directory-stream projection may build complete Invocation carriers for
daemon `federation.subscribe_directory_v2` and normalize daemon `DirectoryEvent`
frames into `EventFrame` DTOs with explicit cursor, resume token,
dropped-event, and terminal state. It must not infer cursor positions from raw
event timestamps or array indexes, create a second event bus, claim
device/session/invocation event subscriptions, or promise daemon-side filtering
until the daemon stream consumes those query fields.
Admin + Gateway carrier/status projection may build complete Invocation
carriers for daemon `agent.list`, `agent.start`, `agent.stop`,
`agent.refresh`, and `session.list`; project daemon lifecycle facts into
`GatewayStatus`; and normalize daemon agent rows/lifecycle results into SDK
DTOs. It must not own backend account state, pairing-token HTTP, certificate
policy, browser session UX, or fabricate hosted-agent URAs.
Compatibility carrier/projection may build complete Invocation carriers for
daemon `openai.list_models` and `openai.chat_completions`; require canonical
agent-owned chat Ability URA model ids; project daemon OpenAI-compatible
model, unary chat, and stream chunk envelopes into SDK DTOs; and adapt SDK
file/resource facts into Compatibility file DTOs. It must not invent daemon
`openai.files.*` abilities, own multipart upload or storage policy, own product
API-key policy, quota/rate limits, billing, HTTP route shaping, SSE fanout, or
treat OpenAI schemas as daemon protocol.
Convenience wrapper record projection may normalize file, terminal, remote
desktop, browser, and media session facts into shared SDK DTOs. It must not
start sessions, own backend HTTP/WebSocket/auth policy, or bypass Runtime Core
Invocation, StreamHandle, or BidiSession execution paths.

## Invocation Tuple

Every invocation draft must expose:

- `caller_ura`
- `callee_ura`
- `descriptor_ref`
- `subject_ura`
- `nonce_base64`
- `causal_context`
- exactly one of `args` or `arguments_base64`

The SDK rejects missing tuple fields before prepare or submit. DescriptorRef
and canonical signing material are delegated to Axon helpers through the Rust
daemon SDK core.

## Language Naming

| Target | Type style | Method style |
| --- | --- | --- |
| Rust | `PascalCase` | `snake_case` |
| C ABI | opaque integer handles | `easynet_*` |
| Go | `PascalCase` exported types | `PascalCase` exported methods |
| Python | `PascalCase` classes | `snake_case` |
| Node/TypeScript | `PascalCase` classes | `camelCase` |
| Java/Kotlin | `PascalCase` classes | `camelCase` |
| Swift | `PascalCase` types | `camelCase` |

Names may be idiomatic, but state transitions and DTO semantics must match.

## Error Taxonomy

All languages must branch on typed errors, not human strings:

- `InvalidArgument`
- `InvalidHandle`
- `VersionIncompatible`
- `DaemonOffline`
- `ControlOnly`
- `PermissionDenied`
- `NotFound`
- `Cancelled`
- `Protocol`
- `Timeout`
- `AbilityFailed`
- `NotImplemented`
- `Generic`

The C ABI integer codes are the current cross-language compatibility floor.
Bindings that cross the C ABI should use `easynet_error_json` or
`easynet_last_error_json` to project those return codes into the shared
`sdk/schemas/error.schema.json` DTO instead of parsing `easynet_last_error()`
message text.

## Stability Gates

A language is stable only when:

- It exposes Runtime Core and every declared profile through this object graph.
- Its public API accepts and returns DTOs covered by `sdk/schemas`.
- It passes `sdk/conformance` cases for every declared profile.
- It exposes no raw Axon/proto/runtime types.
- Its convenience helpers lower to inspectable `InvocationDraft` before
  dispatch.

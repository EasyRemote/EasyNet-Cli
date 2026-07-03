# SDK Interface Spec

This file is the implementation-facing contract for the EasyNet Daemon SDK.
The requirements source remains `docs/spec/daemon-sdk-requirements-v1.md`; this
file records the staged public API shape that language bindings must project.

## Object Graph

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
Receipt projection may normalize summary DTOs and derive causal refs from
explicit receipt facts, but summary-only data must remain `verified: false`
until an Axon-backed verifier proves a full receipt.

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
- `DaemonDown`
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

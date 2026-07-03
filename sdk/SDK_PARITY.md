# SDK Parity

Parity is measured by behavior and public state transitions, not by identical
method spelling.

## Language Tiers

| Language | Tier | Primary consumer | Current status |
| --- | --- | --- | --- |
| Rust | P0 | native SDK core and FFI implementation | partial Runtime Core |
| C ABI | P0 | language binding projection | partial ABI v4 Runtime Core |
| Go | P0 | EasyNet backend/Hub | placeholder |
| Python | P0 | EasyRemote | placeholder |
| Node/TypeScript | P1 | desktop tools and extensions | placeholder |
| Java/JVM | P1 | enterprise and Android-adjacent integrations | placeholder |
| Swift | P1 | macOS/iOS-adjacent clients | placeholder |

## Capability Matrix

| Capability | Rust | C ABI | Go | Python | Node | Java | Swift |
| --- | --- | --- | --- | --- | --- | --- | --- |
| ABI/version discovery | partial | partial | gap | gap | gap | gap | gap |
| daemon start/attach/discover/stop/detach | partial | partial | gap | gap | gap | gap | gap |
| runtime health | partial | partial | gap | gap | gap | gap | gap |
| typed errors | partial | typed JSON partial | gap | gap | gap | gap | gap |
| complete invocation draft | partial | builder handles partial | gap | gap | gap | gap | gap |
| prepare/sign/submit | partial | handle observation partial | gap | gap | gap | gap | gap |
| unary invoke | partial | partial | gap | gap | gap | gap | gap |
| stream | existing dispatch | lifecycle partial | gap | gap | gap | gap | gap |
| bidi | existing dispatch | lifecycle partial | gap | gap | gap | gap | gap |
| directory + identity | read-model/projection partial | read-model/projection partial | gap | gap | gap | gap | gap |
| receipt | fetch/projection partial | fetch/projection partial | gap | gap | gap | gap | gap |
| publication | carrier partial | carrier partial | gap | gap | gap | gap | gap |
| host binding | codec/hash partial | codec/hash partial | gap | gap | gap | gap | gap |
| mission | carrier/status partial | carrier/status partial | gap | gap | gap | gap | gap |
| admin + gateway | carrier/status partial | carrier/status partial | gap | gap | gap | gap | gap |
| events | directory stream partial | directory stream partial | gap | gap | gap | gap | gap |
| surface | carrier/projection partial | carrier/projection partial | gap | gap | gap | gap | gap |
| compatibility | carrier/projection partial | carrier/projection partial | gap | gap | gap | gap | gap |
| conformance runner | scaffold | scaffold | gap | gap | gap | gap | gap |

## Known Gaps

- C ABI now exposes invocation builder handles and submitted InvocationHandle
  await/cancel/events/free handles for unary submit; live event streaming and
  Axon-backed receipt verification remain incomplete.
- C ABI now exposes schema-shaped typed error JSON for ABI return codes; broad
  language facade error classes and per-profile source refs remain incomplete.
- Receipt fetch carrier, projection, and causal-ref guardrails exist for
  Rust/C ABI over daemon `invocation.history.get`; Axon-backed full
  verification, fetched-record execution convenience, and language facades
  remain incomplete.
- Directory read-model carrier/page guardrails, `namespace.resolve`
  carrier/resolved-ref projection guardrails, and Identity URA/DescriptorRef
  projection guardrails exist for Rust/C ABI; subscribe convenience wrappers,
  signer lifecycle, and language facades remain incomplete.
- Publication ResourceRef/package validation/deploy-unpublish carrier guardrails
  exist for Rust/C ABI; daemon list/show/enable/disable read models, execution
  wrappers, and language facades remain incomplete.
- Host Binding codec/hash guardrails exist for Rust/C ABI; product host
  lifecycle, language facades, and profile-ready conformance runners remain
  incomplete.
- Mission carrier/status guardrails exist for Rust/C ABI; live event streams,
  daemon track/cancel convenience methods, language facades, and profile-ready
  conformance runners remain incomplete.
- Events Directory stream carrier/frame guardrails exist for Rust/C ABI over
  daemon `federation.subscribe_directory_v2`; device/session/invocation event
  streams, daemon-side directory filtering, backend SSE/WebSocket fanout, and
  language facades remain incomplete.
- Admin + Gateway carrier/status guardrails exist for Rust/C ABI over daemon
  `agent.list/start/stop/refresh`, `session.list`, lifecycle status, and
  agent-record projections; pairing token flows, credential verification,
  certificate policy, full device-session CRUD, and language facades remain
  incomplete.
- Surface page carrier/projection guardrails exist for Rust/C ABI over daemon
  `pages.list/publish/get/unpublish`; backend route serving, browser auth,
  CDN/cache policy, full surface status, content-management UX, and language
  facades remain incomplete.
- Compatibility carrier/projection guardrails exist for Rust/C ABI over daemon
  `openai.list_models` and `openai.chat_completions`; compatibility file
  create/get/delete, product API-key policy, quota/rate limits, billing,
  backend HTTP route shaping, SSE/WebSocket fanout, and language facades remain
  incomplete.
- Directory subscribe convenience methods, Axon-backed receipt verification,
  full surface status, and convenience wrappers are schema/conformance
  scaffolds only.
- Go and Python packages need real Runtime Core facades before backend or
  EasyRemote cutover.
- C ABI stream/bidi now exposes local stream close and bidi close-send
  half-close controls; schema-backed terminal events, bounded backpressure
  conformance, and P1 language facades remain incomplete.

## Stability Levels

| Level | Meaning |
| --- | --- |
| scaffold | files, schemas, and conformance case names exist |
| partial | code exists for part of the object family and is covered by narrow tests |
| profile-ready | all public methods for the profile pass conformance in one language |
| language-stable | all declared profiles pass conformance for that language |
| cutover-ready | product import bans and route/facade smokes pass |

No current language is `language-stable`.

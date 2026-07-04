# SDK Parity

Parity is measured by behavior and public state transitions, not by identical
method spelling.

## Language Tiers

| Language | Tier | Primary consumer | Current status |
| --- | --- | --- | --- |
| Rust | P0 | native SDK core and FFI implementation | partial Runtime Core |
| C ABI | P0 | language binding projection | partial ABI v4 Runtime Core |
| Go | P0 | EasyNet backend/Hub | Runtime Core discovery/daemon-lifecycle/connect-local lifecycle composition/connection/health/errors/invocation-draft/unary/stream/bidi/handle/prepare-submit plus case-aware conformance execution for selected local facade and projection actions, Directory + Identity, Receipt, Publication, Host Binding, Mission, Admin + Gateway, Events multi-stream subscriptions/device history pages, Surface seams, Compatibility seams, and Wrapper execution seams partial |
| Python | P0 | EasyRemote | Runtime Core discovery/daemon-lifecycle/connect-local lifecycle composition/connection/health/errors/invocation-draft/unary/stream/bidi/handle/prepare-submit plus case-aware conformance execution for selected local facade and projection actions, Directory + Identity, Receipt, Publication, Host Binding, Mission, Admin + Gateway, Events multi-stream subscriptions/device history pages, Surface seams, Compatibility seams, and Wrapper execution seams partial |
| Node/TypeScript | P1 | desktop tools and extensions | placeholder |
| Java/JVM | P1 | enterprise and Android-adjacent integrations | placeholder |
| Swift | P1 | macOS/iOS-adjacent clients | placeholder |

## Capability Matrix

| Capability | Rust | C ABI | Go | Python | Node | Java | Swift |
| --- | --- | --- | --- | --- | --- | --- | --- |
| ABI/version discovery | partial | partial | partial | partial | gap | gap | gap |
| daemon start/attach/discover/stop/detach | partial | partial | state/connect-local seam partial | state/connect-local seam partial | gap | gap | gap |
| runtime connection state | partial | partial | transport seam partial | transport seam partial | gap | gap | gap |
| runtime health | partial | partial | partial | partial | gap | gap | gap |
| typed errors | partial | typed JSON partial | partial | partial | gap | gap | gap |
| complete invocation draft | partial | builder handles partial | partial | partial | gap | gap | gap |
| prepare/sign/submit | partial | handle observation partial | transport seam partial | transport seam partial | gap | gap | gap |
| unary invoke | partial | partial | transport seam partial | transport seam partial | gap | gap | gap |
| stream | existing dispatch | lifecycle partial | state seam partial | state seam partial | gap | gap | gap |
| bidi | existing dispatch | lifecycle partial | state seam partial | state seam partial | gap | gap | gap |
| directory + identity | read-model/projection partial | read-model/projection partial | read-model/projection seam partial | read-model/projection seam partial | gap | gap | gap |
| receipt | fetch/projection partial | fetch/projection partial | projection seam partial | projection seam partial | gap | gap | gap |
| publication | carrier partial | carrier partial | carrier/read-model seam partial | carrier/read-model seam partial | gap | gap | gap |
| host binding | codec/hash partial | codec/hash partial | codec/hash seam partial | codec/hash seam partial | gap | gap | gap |
| mission | carrier/status partial | carrier/status partial | carrier/status seam partial | carrier/status seam partial | gap | gap | gap |
| admin + gateway | carrier/status partial | carrier/status partial | carrier/status seam partial | carrier/status seam partial | gap | gap | gap |
| events | directory stream partial | directory stream partial | directory stream seam partial | directory stream seam partial | gap | gap | gap |
| surface | carrier/projection partial | carrier/projection partial | carrier/projection seam partial | carrier/projection seam partial | gap | gap | gap |
| compatibility | carrier/projection partial | carrier/projection partial | carrier/projection seam partial | carrier/projection seam partial | gap | gap | gap |
| wrappers | carrier/projection partial | carrier/projection partial | execution carrier seam partial | execution carrier seam partial | gap | gap | gap |
| conformance runner | manifest partial | manifest partial | case-aware local facade/projection actions partial | case-aware local facade/projection actions partial | gap | gap | gap |

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
  lifecycle, language facades, and behavior-executing profile conformance
  adapters remain incomplete.
- Mission carrier/status guardrails exist for Rust/C ABI; live event streams,
  daemon track/cancel convenience methods, language facades, and
  behavior-executing profile conformance adapters remain incomplete.
- Events Directory stream carrier/frame guardrails exist for Rust/C ABI over
  daemon `federation.subscribe_directory_v2`; device/session/invocation event
  streams, daemon-side directory filtering, backend SSE/WebSocket fanout, and
  product cutovers remain incomplete.
- Admin + Gateway carrier/status guardrails exist for Rust/C ABI over daemon
  `agent.list/start/stop/refresh`, `session.list`, lifecycle status, and
  agent-record projections; Go/Python facade seams also cover Hub join/leave,
  pairing preflight/create/validate, device credential verification, device
  revocation, and device-session create/list/delete projections. Certificate
  policy, concrete daemon trust/session carriers, and product cutovers remain
  incomplete.
- Surface page carrier/projection guardrails exist for Rust/C ABI over daemon
  `pages.list/publish/get/unpublish`; Go/Python facade seams now expose page
  carriers, typed page records, public page refs, manifests, mutation results,
  and SurfaceHealth readiness projections. Backend route serving, browser auth,
  CDN/cache policy, content-management UX, concrete surface health carriers, and
  product cutovers remain incomplete.
- Compatibility carrier/projection guardrails exist for Rust/C ABI over daemon
  `openai.list_models`, `openai.chat_completions`, and file upload/retrieve/delete
  carriers plus file adapter projections over SDK file/resource facts; product API-key policy,
  quota/rate limits, billing, backend HTTP route shaping, multipart
  upload/storage policy, SSE/WebSocket fanout, and language facades remain
  incomplete.
- Directory subscribe convenience methods, Axon-backed receipt verification,
  and full surface status are schema/conformance scaffolds only.
- Convenience wrapper carrier/projection guardrails exist for Rust/C ABI over
  file, terminal, remote desktop, browser, and media session DTOs; execution
  helpers, backend HTTP/WebSocket bridges, storage policy, and product cutovers
  remain incomplete.
- Go package exposes Runtime Core feature/version discovery with root client close, runtime connection
  state, DaemonHandle lifecycle status/endpoints/start/attach/discover/stop/
  detach/open-runtime/connect-local state seams, runtime health readiness facts,
  schema-backed typed SDK error projection, complete Invocation draft
  construction with inspect/build handle consumption, prepared/signed Invocation DTOs, unary InvocationResult
  projection, StreamHandle state observation, BidiSession frame ordering/
  half-close/cancel/terminal-close observation, InvocationHandle
  await/cancel/events/close observation, and RuntimeClient
  invoke/invoke-stream/open-bidi/prepare/prepare-builder/submit-signed/close methods behind narrow JSON
  transport seams; concrete daemon process spawn/default C ABI or UDS transport, concrete bidi adapters, profile
  clients, backend import-ban integration, per-profile error source refs, and
  remaining profile conformance action execution remain incomplete before backend cutover.
- Go Directory + Identity facade exposes `DirectoryClient` resolve/list
  read-model pages with bounded pagination, directory subscription state seams,
  and close state seams plus `IdentityClient` descriptor, identity, ResourceRef,
  signing-key lifecycle, signer-handle projection, and close state seams; directory live transport
  adapters, local signer implementations, concrete daemon carriers, and backend
  route cutover remain incomplete.
- Go Receipt facade exposes `ReceiptClient` fetch/project/verify/causal-ref
  projection, `invocation.history.get` fetch Invocation carrier construction,
  and close state seams over opaque receipt refs; Axon-backed cryptographic
  verification, concrete daemon carrier execution adapter, receipt URI construction
  after RFC-007, and backend history/metrics cutover remain incomplete.
- Go Publication facade exposes `PublicationClient` resource-ref,
  package-validation, deploy/unpublish Invocation carrier, deploy-result, plugin
  install projection, published-ability read-model seams, and close state seams; concrete daemon
  carriers, list/show/enable/disable runtime execution, host binding bridge,
  plugin/skill lifecycle policy, and backend publication cutover remain
  incomplete.
- Go Host Binding facade exposes `HostBindingClient` binding DTO, envelope
  decode, item/error/terminal frame encoding, and output-hash folding seams over
  schema-backed transport projections plus close state seams; product host readiness execution,
  cleanup execution, local canonical JSON/hash implementation, EasyRemote host
  process integration, and behavior-executing conformance remain incomplete.
- Go Mission facade exposes `MissionClient` run/run-file/track/cancel
  Invocation carrier builders plus daemon `MissionStatus` and
  `MissionEventPage` projection seams and close state seams; daemon mission execution carriers,
  concrete live-tail adapters, child Invocation behavior conformance,
  scheduler/retry policy, and backend automation cutover remain incomplete.
- Go Admin + Gateway facade exposes `AdminClient` agent list/start/stop/refresh
  and session-list Invocation carrier builders plus `GatewayStatus`,
  `AdminAgentPage`, lifecycle-result, pairing token, device credential,
  credential verification, typed device-session projection seams, and close
  state seams;
  certificate policy, concrete daemon trust/session carriers, and backend route
  cutover remain incomplete.
- Go Events facade exposes `EventClient` directory/device/session/invocation
  subscription Invocation carrier builders plus `EventFrame` cursor,
  resume-token, drop-report, terminal projection seams, and bounded device
  event history pages plus close state seams; daemon-side filtering, live stream transport adapters,
  and backend SSE/WebSocket cutover remain incomplete.
- Go Surface facade exposes `SurfaceClient` page list/create/delete/manifest
  Invocation carrier builders plus `SurfacePageRecord`, `SurfacePagePage`,
  `SurfaceManifest`, `SurfacePublicPageRef`, and `SurfaceMutationResult`
  projection seams, plus `SurfaceHealth`/`SurfaceStatus` readiness seams and
  close state seams;
  backend route serving, browser auth, CDN/cache policy, content-management UX,
  concrete surface health carriers, and backend page-route cutover remain
  incomplete.
- Go Compatibility facade exposes `CompatibilityClient` list-models, chat,
  stream-chat, and file upload/get/delete Invocation carrier builders plus
  model, chat, stream, file, file-delete projection seams, and close state seams; product API-key
  policy, quota/rate limits, billing, backend HTTP route shaping, multipart
  storage execution, SSE/WebSocket fanout, and backend compatibility-route
  cutover remain incomplete.
- Go Wrapper facade exposes `WrapperClient` file, terminal, remote desktop,
  browser, and media session Invocation carrier builders, transport-backed
  helper close state seams, and record projections; backend HTTP/WebSocket bridges, storage
  policy, concrete stream/bidi adapters, and product wrapper cutovers remain
  incomplete.
- Python package exposes Runtime Core feature/version discovery with root client close, public
  `SdkEnvironment` process-root factories over private C ABI v4 discovery/daemon lifecycle/open-runtime/runtime health/unary/stream/bidi/prepare-submit handle transport plus C ABI-backed RuntimeConnection connector, runtime
  connection state, DaemonHandle lifecycle status/endpoints/start/attach/
  discover/stop/detach/open-runtime/connect-local state seams, runtime health readiness
  facts, schema-backed typed SDK error projection, complete Invocation draft
  construction with inspect/build handle consumption, `AbilityInvocationClient`
  descriptor-delegated complete tuple build/invoke/stream/bidi facade, prepared/signed Invocation DTOs, signer workflow
  objects over daemon-authorized handles, unary InvocationResult projection plus non-verifying terminal
  receipt projection, StreamHandle state observation, BidiSession frame ordering/
  half-close/cancel/terminal-close observation, InvocationHandle
  await/cancel/events/close observation, and
  RuntimeClient invoke/invoke-stream/open-bidi/prepare/prepare-builder/prepare-and-sign/submit-signed/close methods behind narrow
  transport protocols with timeout-aware stream/bidi receive; public `DaemonInvocationTransport` dict/JSON unary,
  stream, and bidi facade over RuntimeClient/C ABI v4;
  `SdkEnvironment.addressing_client()` and package-level functions for the
  Axon-delegated URA/DescriptorRef helper subset, including SDK-owned
  EasyRemote-style descriptor-ref cutover tests; private C ABI v4 profile carrier/projection bridges for
  Receipt, Directory, Publication, Host Binding, Mission, Admin + Gateway, Events,
  Surface, Compatibility, and Wrapper carriers/records; direct UDS transport,
  live profile execution adapters, actual EasyRemote repository extraction, per-profile
  error source refs, and remaining profile conformance action execution remain incomplete before
  EasyRemote cutover.
- Python Directory + Identity facade exposes `DirectoryClient` resolve/list
  read-model pages with bounded pagination, list/resolve Invocation carrier
  builders, Directory projection helpers, directory subscription state seams,
  and close state seams plus `AddressingClient` and `IdentityClient`
  Axon-delegated `parse_ura`, `owner_ability_ura`,
  `owner_ura_for_ability`, `owner_ability_descriptor_ref`, and
  `canonical_ability_descriptor_ref` helper
  facades, `IdentityClient` descriptor, identity, ResourceRef,
  signing-key lifecycle, signer-handle projection, and close state
  seams; Python now has private C ABI v4 identity projection, profile carrier
  transports, and C ABI-backed resolve/list read-model execution through
  Runtime Core invoke, while
  directory subscription live adapters, local signer implementations, and
  actual EasyRemote repository extraction remain incomplete.
- Python Receipt facade exposes `ReceiptClient` fetch/project/verify/causal-ref
  projection, `invocation.history.get` fetch Invocation carrier construction,
  receipt-derived child `causal_context` adapters, C ABI-backed fetch execution
  through Runtime Core invoke, and close state seams over opaque receipt refs;
  Axon-backed cryptographic verification, receipt URI construction after RFC-007,
  and actual EasyRemote context/receipt extraction remain incomplete.
- Python Publication facade exposes `PublicationClient` resource-ref,
  package-validation, C ABI-backed deploy execution through Runtime Core invoke,
  published-ability list execution through Runtime Core invoke, deploy/unpublish
  Invocation carrier, deploy-result, plugin install projection,
  published-ability read-model seams, and close state seams; concrete daemon
  show/enable/disable/unpublish runtime execution, host binding bridge,
  EasyRemote decorator/package extraction, and plugin/skill lifecycle policy
  remain incomplete.
- Python Host Binding facade exposes `HostBindingClient` binding DTO, envelope
  decode, item/error/terminal frame encoding, and output-hash folding seams over
  schema-backed transport projections plus close state seams; product host readiness execution,
  cleanup execution, local canonical JSON/hash implementation, EasyRemote warm
  host integration, and behavior-executing conformance remain incomplete.
- Python Mission facade exposes `MissionClient` run/run-file/track/cancel
  Invocation carrier builders, C ABI-backed run/run-file/track/cancel execution
  through Runtime Core invoke, plus daemon `MissionStatus` and
  `MissionEventPage` projection seams and close state seams; concrete live-tail adapters,
  child Invocation behavior conformance,
  EasyRemote Pipeline extraction, and scheduler/retry policy remain incomplete.
- Python Admin + Gateway facade exposes `AdminClient` agent
  list/start/stop/refresh and session-list Invocation carrier builders,
  C ABI-backed agent list/start/stop/refresh execution through Runtime Core,
  plus `GatewayStatus`, `AdminAgentPage`, lifecycle-result, pairing token,
  device credential, credential verification, and typed device-session
  projection seams plus close state seams; gateway live status carriers,
  concrete daemon trust/session carriers, certificate policy, and EasyRemote
  Server/AgentControl extraction remain incomplete.
- Python Events facade exposes `EventClient` directory/device/session/invocation
  subscription Invocation carrier builders plus `EventFrame` cursor,
  resume-token, drop-report, terminal projection seams, and bounded device
  event history pages plus close state seams; daemon-side filtering, live stream transport adapters,
  and product cutovers remain incomplete.
- Python Surface facade exposes `SurfaceClient` page list/create/delete/manifest
  Invocation carrier builders, C ABI-backed page list/create/delete/manifest
  execution through Runtime Core, plus `SurfacePageRecord`, `SurfacePagePage`,
  `SurfaceManifest`, `SurfacePublicPageRef`, and `SurfaceMutationResult`
  projection seams, plus `SurfaceHealth`/`SurfaceStatus` readiness seams and
  close state seams; concrete surface health carriers, backend route serving,
  browser auth, CDN/cache policy, content-management UX, and product cutovers
  remain incomplete.
- Python Compatibility facade exposes `CompatibilityClient` list-models, chat,
  stream-chat, and file upload/get/delete Invocation carrier builders plus
  model, chat, stream, file, file-delete projection seams, and close state
  seams; Python concrete C ABI transport now backs unary list-models,
  non-stream chat, and file upload/get/delete execution through Runtime Core,
  while product API-key policy, quota/rate limits, billing, backend HTTP route
  shaping, multipart storage execution, SSE/WebSocket fanout, streaming chat
  adapters, and EasyRemote/Hub compatibility cutovers remain incomplete.
- Python Wrapper facade exposes `WrapperClient` file, terminal, remote desktop,
  browser, and media session Invocation carrier builders, transport-backed
  helper close state seams, and record projections; Python concrete C ABI
  transport now backs wrapper carrier builders plus record-returning file,
  terminal, remote desktop, browser, and media helper execution through Runtime
  Core, while backend HTTP/WebSocket bridges, storage policy, concrete
  stream/bidi adapters, and product wrapper cutovers remain incomplete.
- C ABI stream/bidi now exposes local stream close and bidi close-send
  half-close controls; schema-backed terminal events, bounded backpressure
  conformance, and P1 language facades remain incomplete.
- Go and Python now consume shared SDK conformance cases and fixtures for
  selected local facade/projection actions covering Runtime Core
  Invocation/health, Directory + Identity read-model/projection behavior,
  Mission carrier/status behavior, Publication ResourceRef/package/carrier
  behavior, Admin + Gateway carrier/status behavior, Events directory-stream
  behavior, Surface page carrier/projection behavior, Compatibility OpenAI
  carrier/projection behavior, Host Binding codec/hash behavior, Receipt
  projection, and Wrapper record projection. Daemon-backed behavior-executing
  action adapters over the full case manifest remain incomplete.

## Stability Levels

| Level | Meaning |
| --- | --- |
| scaffold | files, schemas, and conformance case names exist |
| partial | code exists for part of the object family and is covered by narrow tests |
| profile-ready | all public methods for the profile pass conformance in one language |
| language-stable | all declared profiles pass conformance for that language |
| cutover-ready | product import bans and route/facade smokes pass |

No current language is `language-stable`.

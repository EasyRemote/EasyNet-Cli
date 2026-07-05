# SDK Parity

Parity is measured by behavior and public state transitions, not by identical
method spelling.

## Language Tiers

| Language | Tier | Primary consumer | Current capability-state summary |
| --- | --- | --- | --- |
| Rust | P0 | native SDK core and FFI implementation | provider-backed Runtime Core substrate; language parity tracked through FFI and future Rust SDK public matrix |
| C ABI | P0 | language binding projection | provider-backed ABI v4 Runtime Core projection for shipped handles and carriers |
| Go | P0 | EasyNet backend/Hub | provider-backed for Runtime Core, Directory + Identity, Receipt, Publication, Host Binding, Mission, Admin + Gateway, Events, Surface, Compatibility, Wrappers, and the shared conformance runner |
| Python | P0 | EasyRemote | provider-backed for Runtime Core, Directory + Identity, Receipt, Publication, Host Binding, Mission, Admin + Gateway, Events, Surface, Compatibility, Wrappers, and the shared conformance runner |
| Node/TypeScript | P1 | desktop tools and extensions | unsupported |
| Java/JVM | P1 | enterprise and Android-adjacent integrations | unsupported |
| Swift | P1 | macOS/iOS-adjacent clients | unsupported |

## Capability Matrix

The canonical Go/Python state model is machine-checked by
`sdk/conformance/sdk-parity-matrix.json`. Markdown rows below are a summary of
that artifact and must use the same four states only: `unsupported`, `seam`,
`provider-backed`, and `cutover-ready`.

| Capability | Profile | Go | Python | Remaining work |
| --- | --- | --- | --- | --- |
| ABI/version discovery | Runtime Core | provider-backed | provider-backed | Non-P0 bindings and package stability gates remain incomplete. |
| daemon start/attach/discover/stop/detach | Runtime Core | provider-backed | provider-backed | External product repositories still need lower-layer deletion gates before their own cutover claims. |
| runtime connection state | Runtime Core | provider-backed | provider-backed | Remote TCP/TLS daemon endpoint policy is still outside the stable SDK gate. |
| runtime health | Runtime Core | provider-backed | provider-backed | Product health route shaping remains outside the daemon SDK and is not SDK stability evidence. |
| typed errors | Runtime Core | provider-backed | provider-backed | P1 language error classes and package-level source refs remain incomplete. |
| complete invocation draft | Runtime Core | provider-backed | provider-backed | Full package stability claim still depends on all shipped profile gates. |
| prepare/sign/submit | Runtime Core | provider-backed | provider-backed | Daemon-owned signer acquisition and keyring policy cutover remain incomplete. |
| unary invoke | Runtime Core | provider-backed | provider-backed | External product dispatch imports still need repository-local boundary audits. |
| stream | Runtime Core | provider-backed | provider-backed | C ABI terminal projection adapters and bounded backpressure conformance remain incomplete. |
| bidi | Runtime Core | provider-backed | provider-backed | Concrete product wrapper stream adapters remain incomplete. |
| directory + identity | Directory + Identity | provider-backed | provider-backed | External consumer repository extraction and route cutover remain incomplete outside the daemon SDK. |
| receipt | Receipt | provider-backed | provider-backed | Axon-backed full cryptographic verification and RFC-007 receipt URA construction remain incomplete. |
| publication | Publication | provider-backed | provider-backed | Plugin policy, host binding bridge, and external product extraction remain incomplete outside the daemon SDK. |
| host binding | Host Binding | provider-backed | provider-backed | Product host process startup, user-code execution, and downstream product cutover remain outside the daemon SDK. |
| mission | Mission | provider-backed | provider-backed | Daemon-backed child Invocation execution behavior, live stream adapters, and scheduler policy remain incomplete. |
| admin + gateway | Admin + Gateway | provider-backed | provider-backed | Certificate policy, trust persistence, and product pairing lifecycle cutover remain outside the daemon SDK. |
| events | Events | provider-backed | provider-backed | Daemon-side filtering plus external SSE/WebSocket fanout and product cutovers remain incomplete. |
| surface | Surface | provider-backed | provider-backed | Backend route serving, browser auth, cache policy, and content UX remain product-owned and incomplete. |
| compatibility | Compatibility | provider-backed | provider-backed | Product API-key policy, quota, billing, HTTP route shaping, multipart storage, and streaming adapters remain incomplete. |
| wrappers | Wrappers | provider-backed | provider-backed | Backend HTTP/WebSocket bridges, storage policy, concrete stream/bidi adapters, and product wrapper cutovers remain incomplete. |
| conformance runner | SDK Parity | provider-backed | provider-backed | Non-P0 language action-adapter reports and external product cutover smokes remain incomplete outside the Go/Python SDK parity gate. |

## Known Gaps

- C ABI now exposes invocation builder handles and submitted InvocationHandle
  await/cancel/events/free handles for unary submit; live event streaming and
  Axon-backed receipt verification remain incomplete.
- C ABI now exposes schema-shaped typed error JSON for ABI return codes; Python
  and Go profile facades now attach stable `profile`/`source_ref` details and
  execute the shared `error/profile_source_refs` conformance case, while broader
  non-P0 language facade error classes and source refs remain incomplete.
- Backend SDK-only import-ban enforcement now has a shared `backend/import_ban`
  conformance case and executable scanner gate; the sibling EasyNet backend
  still reports raw Axon, generated Axon protobuf, and direct daemon transport
  violations before cutover can be claimed.
- Backend Hub route-family coverage now has a shared
  `backend/hub_route_family_coverage` conformance case, a SPEC 29.2 manifest,
  and an executable validator gate for all 14 Hub route families; real backend
  route source cutover and per-family smokes remain incomplete before cutover
  can be claimed.
- Receipt fetch carrier, projection, causal-ref guardrails, and
  invocation-history list/get/trace carrier builders exist for Rust/C ABI over
  daemon `invocation.history.*` and `invocation.trace.get`; Python now exposes
  the same read models through the Receipt facade and C ABI Runtime Core invoke.
  Axon-backed full verification and broader language-facade cutovers remain
  incomplete.
- Directory read-model carrier/page guardrails, `namespace.resolve`
  carrier/resolved-ref projection guardrails, Identity URA/DescriptorRef
  projection guardrails, and identity signing-key register/list/revoke
  builder/projection guardrails exist for Rust/C ABI; subscribe convenience
  wrappers, signer construction, and language facades remain incomplete.
- Publication ResourceRef/package validation/plugin install/deploy-unpublish
  carrier and lifecycle guardrails exist for Rust/C ABI; Go now executes
  list/show/enable/disable through Runtime Core and C ABI lifecycle projections.
  Plugin/skill lifecycle policy, host binding bridge, backend publication
  cutover, and broader non-P0 language facades remain incomplete.
- Host Binding codec/hash guardrails exist for Rust/C ABI, and Go/Python expose
  conformance-pinned local transports plus SDK-owned lifecycle providers for
  readiness and cleanup state transitions; product host process startup,
  user-code execution, and downstream product cutover remain outside the daemon
  SDK.
- Mission carrier/status/events guardrails exist for Rust/C ABI over
  `mission.run/track/cancel/events`; Go/Python C ABI transports now execute
  run/run-file/track/cancel/events through Runtime Core invoke. Stream-backed
  live adapters, daemon-backed child Invocation execution behavior conformance,
  scheduler/retry policy, and backend automation cutover remain incomplete.
- Events Directory/device/invocation stream carrier guardrails, session stream
  carrier guardrails, DirectoryEvent/drop/terminal projection guardrails, and
  bounded device event history carrier/page guardrails exist for Rust/C ABI over
  daemon-owned abilities; daemon-side live filtering, backend SSE/WebSocket
  fanout, and product cutovers remain incomplete.
- Admin + Gateway carrier/status guardrails exist for Rust/C ABI over daemon
  `agent.list/start/stop/refresh`, `session.list`, `session.create/delete`,
  `federation.revoke`, lifecycle status, agent-record projections,
  hub lifecycle, pairing lifecycle, credential verification, device-session
  page/result projections, and device-admin result projections;
  Go/Python C ABI transports now execute device revoke and device-session
  create/delete, hub join/leave, pairing preflight/create/validate, and
  credential verification through Runtime Core invoke. Go/Python facade seams
  also cover device-session create/list/delete projections. Certificate policy,
  backend trust policy persistence, and product cutovers remain incomplete.
- Surface page carrier/projection guardrails exist for Rust/C ABI over daemon
  `pages.list/publish/get/unpublish/health`; Go/Python facades now expose page
  carriers, typed page records, public page refs, manifests, mutation results,
  SurfaceHealth readiness projections, and Runtime-backed execution paths.
  Backend route serving, browser auth, CDN/cache policy, content-management UX,
  and product cutovers remain incomplete.
- Compatibility carrier/projection guardrails exist for Rust/C ABI over daemon
  `openai.list_models`, `openai.chat_completions`, and file upload/retrieve/delete
  carriers plus file adapter projections over SDK file/resource facts; Go/Python
  facades now expose Runtime-backed execution paths. Product API-key policy,
  quota/rate limits, billing, backend HTTP route shaping, multipart
  upload/storage policy, SSE/WebSocket fanout, and product cutovers remain
  incomplete.
- Directory subscribe convenience methods, Axon-backed receipt verification,
  and full surface status remain schema/conformance declarations only.
- Convenience wrapper carrier/projection guardrails exist for Rust/C ABI over
  file, terminal, remote desktop, browser, and media session DTOs; Go/Python
  facades now expose Runtime-backed record-returning helper execution. Backend
  HTTP/WebSocket bridges, storage policy, concrete stream/bidi adapters, and
  product cutovers remain incomplete.
- Go package exposes Runtime Core feature/version discovery with `SdkEnvironment`
  process root, default daemon discovery/connect policy, explicit
  `DaemonControl` access, local runtime connect, idempotent environment close,
  root client close, and optional
  `easynet_cabi,cgo` C ABI v4 feature-discovery, daemon lifecycle/open-runtime,
  runtime-health, unary invoke, stream/bidi callback, prepare/sign/submit-handle,
  await/cancel/events/free-handle adapters, runtime connection
  state, DaemonHandle lifecycle status/endpoints/start/attach/discover/stop/
  detach/open-runtime/connect-local state seams, runtime health readiness facts,
  schema-backed typed SDK error projection, complete Invocation draft
  construction with inspect/build handle consumption, prepared/signed Invocation DTOs,
  local Ed25519 signer provider over daemon/Axon canonical signing material, unary InvocationResult
  projection, StreamHandle state observation with schema-shaped terminal event
  projection, BidiSession frame ordering/
  half-close/cancel/terminal-close observation, InvocationHandle
  await/cancel/events/close observation, and RuntimeClient
  invoke/invoke-stream/open-bidi/prepare/prepare-builder/submit-signed/close methods behind narrow JSON
  transport seams plus direct daemon UDS unary/stream/bidi transports and Python
  profile clients with stable per-profile error source refs; remaining profile
  conformance action execution and backend source cutover violations remain
  incomplete before backend cutover.
- Go Directory + Identity facade exposes `DirectoryClient` resolve/list
  read-model pages with bounded pagination, Runtime Core-backed directory
  subscription streams, directory subscription state seams, and close state
  seams plus `IdentityClient` descriptor, identity, Axon-delegated
  URA/DescriptorRef helper seams, ResourceRef, signing-key lifecycle,
  signer-handle projection, and close state seams; signing-key live execution
  adapters, concrete daemon carriers, and backend route cutover remain
  incomplete.
- Go Receipt facade exposes `ReceiptClient` fetch/project/verify/causal-ref
  projection, `invocation.history.get` fetch Invocation carrier construction,
  invocation-history list/get/trace read-model methods, optional C ABI v4
  concrete transport for fetch/list/get/trace/project/verify/verify-chain/
  causal-ref over Runtime Core invoke, explicit daemon/Axon projection provider
  seams for Runtime-backed project/verify/verify-chain/causal-ref, and close
  state seams over opaque receipt refs; concrete Axon verification provider
  wiring, receipt URA construction after RFC-007, and backend history/metrics
  cutover remain incomplete.
- Go Publication facade exposes `PublicationClient` resource-ref,
  package-validation, deploy/unpublish Invocation carrier, deploy-result, plugin
  install projection, explicit daemon-local provider seams for Runtime-backed
  package validation and plugin install, published-ability read-model seams,
  complete AbilityImpl lifecycle request execution through Runtime Core and
  optional C ABI v4 carrier/projection, and close state seams; host binding
  bridge, plugin/skill lifecycle policy, and backend publication cutover remain
  incomplete.
- Go Host Binding facade exposes `HostBindingClient` binding DTO, envelope
  decode, item/error/terminal frame encoding, output-hash folding, typed
  readiness/cleanup projections, SDK-owned lifecycle provider/controller state,
  and shared conformance-pinned hash cursor invariants over schema-backed
  transport projections plus close state; product host process startup,
  user-code execution, and downstream product cutover remain incomplete outside
  the daemon SDK.
- Go Mission facade exposes `MissionClient` run/run-file/track/cancel/events
  Invocation carrier builders, C ABI-backed run/run-file/track/cancel/events
  execution through Runtime Core invoke, daemon `MissionStatus` and
  `MissionEventPage` projection seams, and close state seams; concrete live-tail adapters, child Invocation behavior conformance,
  scheduler/retry policy, and backend automation cutover remain incomplete.
- Go Admin + Gateway facade exposes `AdminClient` agent list/start/stop/refresh,
  session-list, hub join/leave, pairing preflight/create/validate, credential
  verification, device-session create/delete, device-revoke Runtime
  Core-backed execution, explicit daemon-owned GatewayStatus provider seams,
  plus
  `GatewayStatus`, `AdminAgentPage`, lifecycle-result, pairing token, device
  credential, credential verification, typed device-session projection seams,
  C ABI-backed device-admin/session result projection, and close state seams;
  certificate policy, backend trust policy persistence, and backend route cutover
  remain incomplete.
- Go Events facade exposes `EventClient` directory/device/session/invocation
  subscription Invocation carrier builders, with session subscriptions requiring
  explicit daemon `session_id` rather than product `session_ura`, Runtime
  Core-backed bounded device event history execution, explicit daemon-owned
  projection provider seams for Runtime-backed directory/drop/terminal frames,
  plus `EventFrame` cursor, resume-token, drop-report, terminal projection
  seams, and close state seams; daemon-side filtering and backend SSE/WebSocket
  cutover remain incomplete.
- Go Surface facade exposes `SurfaceClient` page list/create/delete/manifest
  Invocation carrier builders plus `SurfacePageRecord`, `SurfacePagePage`,
  `SurfaceManifest`, `SurfacePublicPageRef`, and `SurfaceMutationResult`
  projections, plus `SurfaceHealth`/`SurfaceStatus` readiness projections,
  `SurfaceRuntimeTransport` Runtime Core execution, and close state; backend
  route serving, browser auth, CDN/cache policy, content-management UX, and
  backend page-route cutover remain incomplete outside the daemon SDK.
- Go Compatibility facade exposes `CompatibilityClient` list-models, chat,
  stream-chat, and file upload/get/delete Invocation carrier builders plus
  model, chat, stream, file, file-delete projections, `CompatibilityRuntimeTransport`
  Runtime Core execution, and close state; product API-key policy, quota/rate
  limits, billing, backend HTTP route shaping, multipart storage execution,
  SSE/WebSocket fanout, and backend compatibility-route cutover remain
  incomplete outside the daemon SDK.
- Go Wrapper facade exposes `WrapperClient` file, terminal, remote desktop,
  browser, and media session Invocation carrier builders, `WrapperRuntimeTransport`
  Runtime Core execution, transport-backed helper close state, and record
  projections; backend HTTP/WebSocket bridges, storage policy, concrete
  stream/bidi adapters, and product wrapper cutovers remain incomplete outside
  the daemon SDK.
- Python package exposes Runtime Core feature/version discovery with root client close, public
  `SdkEnvironment` process-root factories with default daemon control-path
  resolution over direct control-plane UDS boot/status IPC, private C ABI v4 discovery/daemon lifecycle/open-runtime/runtime health/unary/stream/bidi/prepare-submit handle transport, and direct daemon Axon gRPC-over-UDS unary/server-stream transport plus control-discovery-backed RuntimeConnection endpoint resolution with C ABI-backed or direct daemon handshake, runtime
  direct daemon Axon gRPC-over-UDS handle operations composed through an explicit
  RuntimeTransport delegate, runtime connection state, DaemonHandle lifecycle status/endpoints/invocation-endpoint lookup/start/attach/
  discover/stop/detach/open-runtime/connect-local state seams, runtime health readiness
  facts, SDK-owned `DaemonStartProjection` and
  `DaemonLifecycleFacade` start-wire/status/open-client
  projection, schema-backed typed SDK error projection, complete Invocation draft
  construction with inspect/build handle consumption, `AbilityInvocationClient`
  descriptor-delegated complete tuple build/invoke/stream/bidi facade plus
  generic ability target build/invoke/stream/bidi/prepare/prepare-and-sign helpers and
  explicit submit/await/cancel/events/close-handle observation helpers,
  `InvocationObjectAdapter` tuple-like object Runtime Core facade
  `InvocationDraft` and daemon wire DTO construction, object-bound
  Runtime Core lifecycle delegation from InvocationBuilder through InvocationHandle, prepared/signed Invocation DTOs, signer workflow
  objects over daemon-authorized handles, unary InvocationResult projection plus non-verifying terminal
  receipt projection, StreamHandle state observation with schema-shaped
  terminal event projection, BidiSession frame ordering/
  half-close/cancel/terminal-close observation, InvocationHandle
  await/cancel/events/close observation, DaemonHandle-scoped Runtime/Profile client factories, and
  RuntimeClient invoke/invoke-stream/open-bidi/prepare/prepare-builder/prepare-and-sign/submit-signed/close methods behind narrow
  transport protocols with timeout-aware stream/bidi receive; public `DaemonInvocationTransport` dict/JSON unary,
  stream, bidi, and signed unary prepare/sign/submit/await/free-handle facade
  with RuntimeConnection-owned session lifecycle over C ABI v4, plus
  SDK-owned signed unary signer-boundary errors plus product-neutral
  `InvocationResultAdapter`, `UnaryDispatchPool`, `StreamValueAdapter`, and
  `BidiSessionAdapter` helpers for unary wait/timeout/retire/close transport-pool state,
  stream terminal/timeout/error/payload value projection, and bidi session
  close/cancel/timeout/wire-error lifecycle projection;
  `SdkEnvironment.addressing_client()` and package-level functions for the
  Axon-delegated URA/DescriptorRef helper subset, including SDK-owned
  generic descriptor-ref and target-dispatch cutover tests, stable
  per-profile error source refs; private C ABI v4 profile carrier/projection bridges for
  Receipt, Directory, Publication, Host Binding, Mission, Admin + Gateway, Events,
  Surface, Compatibility, and Wrapper carriers/records; direct daemon
  prepare/submit adapters, live profile execution adapters, remaining EasyRemote repository extraction,
  and remaining profile conformance action execution remain incomplete before
  EasyRemote cutover.
- Python Directory + Identity facade exposes `DirectoryClient` resolve/list
  read-model pages with bounded pagination, list/resolve Invocation carrier
  builders, Directory projection helpers, C ABI-backed directory subscription
  execution through Runtime Core open_stream, directory subscription state seams,
  and close state seams plus `AddressingClient` and `IdentityClient`
  Axon-delegated `parse_ura`, `device_ura`, `agent_ura`,
  `device_agent_ura`, `hub_ura`, `resource_ura`, `device_ability_ura`,
  `owner_ability_ura`, `owner_ura_for_ability`,
  `ability_ura_from_descriptor_ref`, `owner_ability_descriptor_ref`, and
  `canonical_ability_descriptor_ref` helper
  facades plus an `AbilityAddress` projection for owner/subject facts consumed
  by generic host addressing, `IdentityClient` descriptor, identity, ResourceRef,
  signing-key lifecycle and signer-handle projection seams, and close state
  seams; Python now has private C ABI v4 identity projection, profile carrier
  transports, and C ABI-backed resolve/list read-model execution through
  Runtime Core invoke, plus C ABI-backed signing-key register/list/revoke
  execution through daemon identity abilities and signer-handle projection from
  daemon key inventory plus a Python `Ed25519SignatureProvider` for local
  signatures over daemon/Axon-provided canonical signing material; Python now
  also exposes directory buffered-event/drop projection state-machine helpers,
  while actual EasyRemote repository extraction remains incomplete.
- Python Receipt facade exposes `ReceiptClient` fetch/project/verify/causal-ref
  projection, `invocation.history.get` fetch Invocation carrier construction,
  invocation-history list/get/trace read-model methods,
  receipt-derived child `causal_context` adapters, `AbilityInvocationClient`
  child-context helpers for generic host nested calls, typed `ReceiptRef` and
  `ReceiptChain` wrappers that delegate causal-context and continuity projection
  through the client, `ReceiptVerification` cryptographic-assurance guardrails
  that reject summary-only projections as verifier evidence, local SDK
  receipt-summary projection/continuity/causal-ref guardrails, SDK-owned
  `LocalReceiptSummary`/`LocalReceiptSummaryChain` parsing, state projection,
  summary-only verification guardrails, and hash-chain continuity projection,
  C ABI-backed fetch and invocation-history list/get/trace execution through
  Runtime Core invoke, and close state seams over opaque receipt refs.
  EasyRemote receipt summary verification, hash-chain continuity, and
  parent-receipt-anchored Context child dispatch now delegate to SDK Receipt.
  Axon-backed cryptographic verification and receipt URA construction after
  RFC-007 remain incomplete.
- Python Publication facade exposes `PublicationClient` resource-ref,
  package-validation, direct C ABI-backed deploy execution through Runtime Core invoke,
  C ABI-backed plugin install through the daemon plugin installer,
  C ABI-backed deploy-result projection,
  published-ability list/show execution through Runtime Core invoke,
  ability-implementation enable/disable execution through complete C ABI
  carriers and Runtime Core invoke, complete unpublish execution through Runtime Core invoke,
  deploy/show/enable-impl/disable-impl/unpublish Invocation carrier,
  deploy/show/enable-impl/disable-impl/unpublish result, published-ability read-model seams,
  SDK-owned publication host catalogue install/list/list-device/list-user/
  show projection, and close state seams; host binding bridge, EasyRemote
  decorator/package extraction, and broader plugin/skill lifecycle policy
  remain incomplete.
- Python Host Binding facade exposes `HostBindingClient` binding DTO, envelope
  decode, typed cleanup/readiness/lifecycle ownership DTOs,
  item/error/terminal frame encoding, output-hash folding with shared
  conformance-pinned hash cursor invariants over schema-backed and local SDK
  codec transports, daemon host-stream line-protocol projections, an SDK-owned
  lifecycle provider/controller for readiness and cleanup transitions, and a
  `HostStreamFrameWriter` lifecycle helper that delegates all frame/hash
  semantics through the client plus per-call `HostStreamSession` state;
  EasyRemote warm host frame/error/terminal emission now delegates frame and
  output-hash semantics to SDK Host Binding. Product host process startup,
  user-code execution, and downstream product cutover remain incomplete outside
  the daemon SDK.
- Python Consumer boundary audit helpers expose source-tree checks for raw
  FFI/Axon imports, raw C ABI symbols, raw Invocation JSON codecs, and raw
  URA/DescriptorRef helpers, raw host-stream frame/hash codecs, raw
  receipt-chain continuity checks, and raw admin/mission carrier strings, plus
  manifest checks for raw Axon/ABI package dependencies, with shared conformance
  cases for no-raw-FFI, no-raw-invocation-codec, addressing-helper ownership,
  host-stream-codec ownership, receipt-continuity ownership, context-causal
  gates, and admin/mission carrier gates; Runtime transport adapter,
  unary wait/retire lifecycle, stream value projection, invocation,
  bidi session lifecycle, addressing helpers, warm host frame/hash substrate, receipt summary/continuity,
  hosted-agent admin, Context child dispatch, Mission transport/event-page
  extraction, SDK-owned Admin/Mission Daemon profile bridge, page-based
  Mission event access/live-tail plus publication catalogue extraction,
  gateway lifecycle hub-config/fingerprint projection, and MissionPlan
  EAL rendering/child Invocation fact conformance projection, plus
  EasyRemote `sign=True` signed unary dispatch over SDK Runtime Core
  prepare/sign/submit/await/free with an explicit SDK signer now pass static
  gates, and product Invocation direct daemon UDS unary/server-stream/bidi transport plus
  explicit direct-runtime prepare/submit/handle delegation is available through
  the SDK facade, while daemon-owned signer acquisition/keyring policy, daemon-backed
  MissionPlan child Invocation execution behavior, daemon/ABI-backed Server
  pairing/hub lifecycle cutover, and full receipt verification remain
  incomplete.
- Python Mission facade exposes `MissionClient` run/run-file/track/cancel/events
  Invocation carrier builders, C ABI-backed run/run-file/track/cancel execution
  through Runtime Core invoke, C ABI-backed mission events execution through
  Runtime Core invoke, daemon `MissionStatus` and `MissionEventPage`
  projection seams, an SDK-owned `MissionExecutionAdapter` with
  event-page projection, SDK-owned Daemon profile bridge dispatch/projection
  glue, page-based Mission event access, SDK-owned bounded Mission event
  projection tailing, SDK-owned `MissionPlan` EAL rendering and
  child Invocation fact conformance projection, raw mission carrier audit gate for
  `mission.run/track/cancel/events`, and close state seams;
  daemon stream-backed live adapters, daemon-backed child Invocation execution
  behavior conformance, and scheduler/retry policy remain incomplete.
- Python Admin + Gateway facade exposes `AdminClient` agent
  list/start/stop/refresh, session-list, hub join/leave, pairing
  preflight/create/validate, credential verification, device-session
  create/delete, and device-revoke Invocation carrier builders, C ABI-backed
  agent list/start/stop/refresh, session-list, hub join/leave, pairing
  preflight/create/validate, credential verification, device-session
  create/delete, and device-revoke execution through Runtime Core, C ABI-backed
  daemon lifecycle `GatewayStatus` projection, plus
  `AdminAgentPage`, lifecycle-result, pairing token, device credential,
  credential verification, C ABI-backed typed device-session page projection,
  device-admin result projection, and close state seams, with an SDK-owned
  `AgentLifecycleAdapter` add/list/stop/refresh Admin adapter and
  SDK-owned Daemon profile bridge dispatch/projection glue for gateway
  status, hub join/leave, pairing preflight/create/validate, credential
  verification, device-session create/list/delete, and device revoke, SDK-owned
  `GatewayLifecycleFacade` hub-config materialization, lifecycle state, TLS file
  validation, endpoint projection, certificate fingerprint projection, and raw
  admin carrier consumer-boundary audit gate for `agent.start/list/stop/refresh`;
  invocation-builder carrier methods remain fail-closed in the EasyRemote
  product bridge, while certificate policy, backend trust policy persistence,
  and daemon-backed Server pairing/hub lifecycle product cutover remain
  incomplete.
- Python Events facade exposes `EventClient` directory/device/session/invocation
  subscription Invocation carrier builders, C ABI-backed directory/device/session/invocation
  subscription execution through Runtime Core open_stream, plus `EventFrame`
  cursor, resume-token, drop-report, terminal projection seams, and C ABI-backed
  bounded device event history execution through Runtime Core invoke plus close
  state seams; daemon-side filtering, backend SSE/WebSocket fanout, and product
  cutovers remain incomplete.
- Python Surface facade exposes `SurfaceClient` page list/create/delete/manifest/health
  Invocation carrier builders, C ABI-backed page list/create/delete/manifest/health
  execution through Runtime Core, plus `SurfacePageRecord`, `SurfacePagePage`,
  `SurfaceManifest`, `SurfacePublicPageRef`, and `SurfaceMutationResult`
  projection seams, plus `SurfaceHealth`/`SurfaceStatus` readiness seams and
  close state seams; backend route serving, browser auth, CDN/cache policy,
  content-management UX, and product cutovers remain incomplete.
- Python Compatibility facade exposes `CompatibilityClient` list-models, chat,
  stream-chat, and file upload/get/delete Invocation carrier builders plus
  model, chat, stream, file, file-delete projection seams, and close state
  seams; Python concrete C ABI transport now backs unary list-models,
  non-stream chat, and file upload/get/delete execution through Runtime Core,
  while product API-key policy, quota/rate limits, billing, backend HTTP route
  shaping, multipart storage execution, SSE/WebSocket fanout, streaming chat
  adapters, and EasyRemote/Hub compatibility cutovers remain incomplete.
- Python Wrapper facade exposes `WrapperClient` file, terminal, remote desktop,
  browser, and media session Invocation carrier builders, public
  `RuntimeWrapperTransport` carrier-build/runtime-invoke/record-project
  composition, transport-backed helper close state seams, and record
  projections; Python concrete C ABI transport now backs wrapper carrier
  builders plus record-returning file, terminal, remote desktop, browser, and
  media helper execution through Runtime Core, while backend HTTP/WebSocket
  bridges, storage policy, concrete stream/bidi adapters, and product wrapper
  cutovers remain incomplete.
- Go and Python stream/bidi facades now expose schema-shaped Runtime Core
  terminal projections backed by shared conformance expectations; C ABI terminal
  projection adapters, bounded backpressure conformance, P1 language facades,
  and strict Go/Python parity remain incomplete.
- Go/Python SDK parity now has a shared machine-checked matrix gate at
  `sdk/conformance/sdk-parity-matrix.json` with status values limited to
  `unsupported`, `seam`, `provider-backed`, and `cutover-ready`; the gate
  records current capability gaps and rejects product-specific capability rows
  so external product cutover work does not become language SDK structure.
- Go and Python now consume shared SDK conformance cases and fixtures for
  local facade/projection actions covering Runtime Core
  Invocation/health, Directory + Identity read-model/projection behavior,
  Mission carrier/status behavior, Publication ResourceRef/package/carrier
  behavior, Admin + Gateway carrier/status behavior, Events directory-stream
  behavior, Surface page carrier/projection behavior, Compatibility OpenAI
  carrier/projection behavior, Host Binding codec/hash behavior, Receipt
  projection, and Wrapper record projection. The shared conformance runner now
  validates Go/Python action-adapter reports against every required shared case,
  while non-P0 language adapter reports and external product cutover smokes
  remain incomplete.

## Capability States

| State | Meaning |
| --- | --- |
| unsupported | No public SDK object or declared shipped facade support exists for this language. |
| seam | Public DTOs, clients, carriers, or state objects exist, but product workflows still need lower-layer or facade-local execution to cut over. |
| provider-backed | The public facade has a daemon, C ABI, Runtime Core, or explicit provider delegate path covered by shared conformance evidence. |
| cutover-ready | The first-class consumer can remove lower-layer product code and the required import, route, or facade gates pass. |

No current language is `cutover-ready` across every declared consumer-facing
capability; consumer repository cutover evidence remains outside the SDK
profile model.

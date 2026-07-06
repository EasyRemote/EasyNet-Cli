# Python Daemon SDK

Python is the P0 facade for EasyRemote and local automation. It may use the C
ABI internally, but EasyRemote product code must not own ctypes loaders, raw
handles, Invocation JSON codecs, or host-stream wire/hash semantics.

Current status: Runtime Core discovery/daemon-lifecycle/connection/health/
errors/connect-local lifecycle composition/invocation-draft/unary/stream/bidi/handle/prepare-submit plus
Directory + Identity, Receipt, Publication, Host Binding, Mission,
Admin + Gateway, Events multi-stream subscription, Surface page seam, and Compatibility
OpenAI adapter seam, and Convenience Wrapper execution seam
partial. The package exposes typed
feature/version discovery with root client close, public `SdkEnvironment`
process-root factories with default daemon control-path resolution, direct control-plane UDS boot/status IPC over
`control.json`/`control.sock`, private C ABI v4 discovery, daemon
lifecycle/open-runtime, identity projection, runtime
health/unary/stream/bidi/prepare-submit handle transports, and direct daemon
Axon gRPC-over-UDS unary/server-stream transport plus
control-discovery-backed RuntimeConnection endpoint resolution with C ABI-backed
handshake, profile
carrier/projection transports for Receipt, Directory, Publication, Host
Binding, Mission, Admin + Gateway, Events, Surface, Compatibility, and
Wrapper carrier/projection records, runtime connection
state, runtime health readiness facts, DaemonHandle lifecycle
status/endpoints/invocation-endpoint lookup/start/attach/discover/stop/detach/open-runtime/connect-local
state seams, SDK-owned `DaemonStartProjection` and
`DaemonLifecycleFacade` start-wire/status/open-client projection,
schema-backed SDK error projection with stable error classes and profile
source-ref accessors, complete
Invocation draft construction, AbilityInvocationClient descriptor-delegated
complete tuple build/invoke/stream/bidi convenience facade plus generic ability
target build/invoke/stream/bidi/prepare/prepare-and-sign helpers and explicit
submit/await/cancel/events/close-handle observation helpers, invocation object
adapter for building SDK `InvocationDraft` values and daemon wire
dicts without raw product codecs, prepared/signed Invocation DTOs, unary
InvocationResult and non-verifying terminal receipt projection, StreamHandle state
observation with schema-shaped terminal event projection, BidiSession frame
ordering, half-close, cancel, terminal-frame projection, and terminal-close observation, object-bound
Runtime Core lifecycle delegation from InvocationBuilder through InvocationHandle, signer workflow
objects over daemon-authorized handles, InvocationHandle await/cancel/events/close
observation, SDK-owned signed unary prepare/sign/submit/await/free-handle
transport path with explicit signer-boundary errors, product-neutral
`InvocationResultAdapter`, `UnaryDispatchPool`, `StreamValueAdapter`, and
`BidiSessionAdapter` runtime transport helpers for unary wait/timeout/retire/
close, stream terminal/timeout/error/payload projection, and bidi session
close/cancel/timeout/wire-error lifecycle projection,
DaemonHandle-scoped Runtime/Profile client factories, and RuntimeClient
invoke/invoke-stream/open-bidi/prepare/prepare-and-sign/submit-signed/close
methods behind narrow transport protocols with timeout-aware stream/bidi receive,
plus public
DaemonInvocationTransport dict/JSON unary, stream, and bidi facade with
RuntimeConnection-owned session lifecycle over C ABI v4 plus explicit direct
daemon Axon gRPC-over-UDS unary/server-stream connection, DirectoryClient resolve/list read-model pages,
C ABI-backed resolve/list read-model execution through Runtime Core invoke,
list/resolve Invocation carrier builders, directory
projection helpers, C ABI-backed directory subscription execution through
Runtime Core open_stream, directory subscription state seams, buffered-event/drop
projection helpers, and close state seams plus
AddressingClient and package-level Axon-delegated `parse_ura`, `device_ura`,
`agent_ura`, `device_agent_ura`, `hub_ura`, `resource_ura`,
`device_ability_ura`, `owner_ability_ura`, `owner_ura_for_ability`,
`ability_ura_from_descriptor_ref`, `owner_ability_descriptor_ref`, and
`canonical_ability_descriptor_ref`, and `project_descriptor_ref` helpers plus an
`AbilityAddress` projection for generic host callee/subject ownership facts,
IdentityClient descriptor/resource projection, C ABI-backed signing-key
register/list/revoke execution through daemon identity abilities, and
C ABI-backed signer-handle projection from daemon key inventory with
SDK-owned signer workflow acquisition, signer-handle provenance/policy-proof guardrails, plus
`Ed25519SignatureProvider` for local signatures over daemon/Axon-provided
canonical signing material.
It also exposes ReceiptClient fetch/project/verify/causal-ref projection,
receipt-derived child `causal_context` adapters, `AbilityInvocationClient`
child-context helpers for generic host nested calls, C ABI-backed fetch plus
invocation-history list/get/trace read-model execution through Runtime Core
invoke, and close state seams over opaque receipt refs plus typed
ReceiptRef/ReceiptChain facades that
delegate causal-context and continuity projection back through ReceiptClient,
plus `ReceiptVerification` cryptographic-assurance guardrails that reject
summary-only projections as verifier evidence, and SDK-owned
`LocalReceiptSummary`/`LocalReceiptSummaryChain` parsing, state projection,
summary-only verification guardrails, and hash-chain continuity projection,
plus PublicationClient resource-ref,
package-validation, direct C ABI-backed deploy execution through Runtime Core invoke,
C ABI-backed deploy-result projection,
published-ability list execution through Runtime Core invoke,
published-ability show execution through Runtime Core invoke,
complete unpublish execution through Runtime Core invoke,
deploy/show/enable-impl/disable-impl/unpublish Invocation carrier,
deploy/show/enable-impl/disable-impl/unpublish result, plugin
install projection, published-ability read-model seams, SDK-owned
`PublicationCatalogFacade` publication host catalogue install/list/list-device/
list-user/show projection, and close state seams. HostBindingClient
exposes binding DTO, typed cleanup/readiness/lifecycle ownership DTOs, envelope
decode, item/error/terminal frame encoding, output-hash folding seams, and a
HostStreamFrameWriter lifecycle helper that delegates frame/hash semantics
through the client plus per-call HostStreamSession state seams. Python also
ships `LocalHostBindingTransport` for product-host frame/hash facade execution
against the shared Host Binding conformance fixtures. The package also exposes
Consumer boundary audit helpers that reject raw FFI/Axon imports, raw
Invocation JSON codecs, raw URA/DescriptorRef helpers, raw host-stream
frame/hash codecs, raw admin/mission carrier strings, and raw lower-layer
Axon/ABI dependencies in consumer manifests.
MissionClient exposes run/run-file/track/cancel
Invocation carrier builders, C ABI-backed run/run-file/track/cancel execution
through Runtime Core invoke, plus MissionStatus and MissionEventPage projection
seams, an SDK-owned `MissionExecutionAdapter` over typed Mission profile
requests, SDK-owned `DaemonProfileBridge` dispatch/projection glue,
page-based Mission event access, SDK-owned bounded Mission event
projection tailing, Runtime Core-backed Mission event streams, SDK-owned
`MissionPlan` EAL rendering and complete child Invocation fact conformance
projection, and close state seams. AdminClient
exposes agent list/start/stop/refresh, session-list, hub join/leave, pairing
preflight/create/validate, credential verification, device-session
create/delete, and device-revoke Invocation carrier builders, C ABI-backed
agent list/start/stop/refresh, session-list, hub join/leave, pairing
preflight/create/validate, credential verification, device-session
create/delete, and device-revoke execution through Runtime Core invoke,
C ABI-backed gateway lifecycle status projection, plus GatewayStatus,
AdminAgentPage, lifecycle-result, pairing token, device credential, credential
verification, and C ABI-backed typed device-session page/result projection plus close state seams,
with an SDK-owned `AgentLifecycleAdapter` over typed Admin profile
requests for add/list/stop/refresh, SDK-owned `DaemonProfileBridge`
dispatch/projection glue for gateway status, hub join/leave, pairing
preflight/create/validate, credential verification, device-session
create/list/delete, and device revoke, and SDK-owned `GatewayLifecycleFacade`
hub-config materialization, lifecycle state, TLS file validation, endpoint
projection, and certificate fingerprint projection.
EventClient exposes directory/device/session/invocation subscription
Invocation carriers, C ABI-backed directory/device/session/invocation
subscription execution through Runtime Core open_stream, C ABI-backed bounded
device event history execution through Runtime Core invoke, and EventFrame
cursor/resume/drop-report/terminal projection seams plus close state seams. SurfaceClient
exposes page list/create/delete/manifest Invocation carriers, C ABI-backed
page list/create/delete/manifest/health execution through Runtime Core invoke, plus
SurfacePageRecord, SurfacePagePage, SurfaceManifest, SurfacePublicPageRef, and
SurfaceMutationResult projection seams plus SurfaceHealth/SurfaceStatus readiness
seams and close state seams. CompatibilityClient exposes
OpenAI-compatible list-models/chat/stream-chat and file upload/get/delete
Invocation carriers, including C ABI-backed unary list-models, non-stream chat,
and file upload/get/delete execution through Runtime Core invoke, plus model,
chat, stream, file, and file-delete projection seams and close state seams.
WrapperClient exposes
file, terminal, remote desktop, browser, and media session Invocation carrier
builders, including C ABI-backed record-returning file, terminal, remote desktop,
browser, and media helper execution through Runtime Core invoke, plus public
`RuntimeWrapperTransport` composition for carrier-build/runtime-invoke/stream/bidi/
record-project execution,
transport-backed helper close state seams, and record projections. EasyRemote
signed unary dispatch now flows through Runtime Core
prepare/sign/submit/await/free when a daemon-authorized SDK signer is supplied.
Product Invocation direct daemon UDS unary, server-stream, bidi transport, and
explicit direct-runtime prepare/submit/handle delegation are available through
the SDK facade with signer policy-proof validation. Live daemon keyring signing
execution policy, Axon-backed receipt verification, receipt URA
construction, ability implementation lifecycle adapters that require daemon/ABI
lifecycle result contracts, warm host process execution
and cleanup execution adapters,
certificate policy, backend trust policy persistence, Events daemon filtering/live adapters,
backend rendering/auth/cache cutover, Compatibility API-key/quota/HTTP/SSE,
multipart storage execution, and product cutovers, wrapper backend HTTP/WebSocket bridges,
profile-specific stream execution adapters, daemon-backed MissionPlan child
Invocation execution behavior, and daemon/ABI-backed Server pairing/hub lifecycle
product cutover remain incomplete. See
`../SDK_PARITY.md` before claiming package stability.

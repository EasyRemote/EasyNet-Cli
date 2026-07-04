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
process-root factories, private C ABI v4 discovery, daemon
lifecycle/open-runtime, identity projection, and runtime
health/unary/stream/bidi/prepare-submit handle transports plus C ABI-backed
RuntimeConnection connector, profile
carrier/projection transports for Receipt, Directory, Publication, Host
Binding, Mission, Admin + Gateway, Events, Surface, Compatibility, and
Wrapper carrier/projection records, runtime connection
state, runtime health readiness facts, DaemonHandle lifecycle
status/endpoints/start/attach/discover/stop/detach/open-runtime/connect-local
state seams, schema-backed SDK error projection, complete
Invocation draft construction, AbilityInvocationClient descriptor-delegated
complete tuple build/invoke/stream/bidi convenience facade, prepared/signed Invocation DTOs, unary
InvocationResult and non-verifying terminal receipt projection, StreamHandle state observation, BidiSession frame
ordering, half-close, cancel, and terminal-close observation, signer workflow
objects over daemon-authorized handles, InvocationHandle await/cancel/events/close
observation, and RuntimeClient
invoke/invoke-stream/open-bidi/prepare/prepare-and-sign/submit-signed/close
methods behind narrow transport protocols with timeout-aware stream/bidi receive,
plus public
DaemonInvocationTransport dict/JSON unary, stream, and bidi facade over
RuntimeClient/C ABI v4, DirectoryClient resolve/list read-model pages,
C ABI-backed resolve/list read-model execution through Runtime Core invoke,
list/resolve Invocation carrier builders, directory
projection helpers, directory subscription state seams, and close state seams plus
AddressingClient and package-level Axon-delegated `parse_ura`, `owner_ability_ura`,
`owner_ura_for_ability`, `owner_ability_descriptor_ref`, and
`canonical_ability_descriptor_ref` helpers,
IdentityClient descriptor/resource projection, signing-key lifecycle, and
signer-handle seams.
It also exposes ReceiptClient fetch/project/verify/causal-ref projection,
receipt-derived child `causal_context` adapters, C ABI-backed fetch execution
through Runtime Core invoke, and close state seams
over opaque receipt refs, plus PublicationClient resource-ref,
package-validation, direct C ABI-backed deploy execution through Runtime Core invoke,
C ABI-backed deploy-result projection,
published-ability list execution through Runtime Core invoke,
deploy/unpublish Invocation carrier, deploy-result, plugin
install projection, published-ability read-model seams, and close state seams. HostBindingClient
exposes binding DTO, envelope decode, item/error/terminal frame encoding, and
output-hash folding seams plus close state seams. MissionClient exposes run/run-file/track/cancel
Invocation carrier builders, C ABI-backed run/run-file/track/cancel execution
through Runtime Core invoke, plus MissionStatus and MissionEventPage projection
seams and close state seams. AdminClient
exposes agent list/start/stop/refresh and session-list Invocation carrier
builders, C ABI-backed agent list/start/stop/refresh execution through Runtime
Core invoke, plus GatewayStatus, AdminAgentPage, lifecycle-result, pairing
token, device credential, credential verification, and typed device-session
projection seams plus close state seams. EventClient exposes directory/device/session/invocation subscription
Invocation carriers, device event history pages, and EventFrame
cursor/resume/drop-report/terminal projection seams plus close state seams. SurfaceClient
exposes page list/create/delete/manifest Invocation carriers, C ABI-backed
page list/create/delete/manifest execution through Runtime Core invoke, plus
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
browser, and media helper execution through Runtime Core invoke,
transport-backed helper close state seams, and record projections. Direct daemon
UDS transport, directory subscription live adapters, local signer implementations,
Axon-backed receipt verification, receipt URI construction, publication
show/unpublish and plugin lifecycle live adapters, host-binding execution
adapters, mission event streams, Admin gateway live status carriers, concrete
Admin trust/session carriers, certificate policy, Events daemon filtering/live adapters, concrete surface health
carriers, backend rendering/auth/cache cutover, Compatibility API-key/quota/HTTP/SSE,
multipart storage execution, and product cutovers, wrapper backend HTTP/WebSocket bridges,
profile-specific stream/bidi execution adapters, and the actual EasyRemote
repository cutover remain incomplete. See
`../SDK_PARITY.md` before claiming package stability.

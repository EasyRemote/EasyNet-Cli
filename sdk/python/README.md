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
health/unary/stream/bidi/prepare-submit handle transports, profile
carrier/projection transports for Receipt, Directory, Publication, Host
Binding, Mission, Admin + Gateway, Events, Surface, Compatibility, and
Wrapper carrier/projection records, runtime connection
state, runtime health readiness facts, DaemonHandle lifecycle
status/endpoints/start/attach/discover/stop/detach/open-runtime/connect-local
state seams, schema-backed SDK error projection, complete
Invocation draft construction, prepared/signed Invocation DTOs, unary
InvocationResult projection, StreamHandle state observation, BidiSession frame
ordering, half-close, cancel, and terminal-close observation, InvocationHandle
await/cancel/events/close observation, and RuntimeClient
invoke/invoke-stream/open-bidi/prepare/submit-signed/close
methods behind narrow transport protocols, plus public
DaemonInvocationTransport dict/JSON unary, stream, and bidi facade over
RuntimeClient/C ABI v4, DirectoryClient resolve/list read-model pages,
C ABI-backed resolve/list read-model execution through Runtime Core invoke,
list/resolve Invocation carrier builders, directory
projection helpers, directory subscription state seams, and close state seams plus
AddressingClient Axon-delegated `parse_ura`, `owner_ability_ura`,
`owner_ura_for_ability`, `owner_ability_descriptor_ref`, and
`canonical_ability_descriptor_ref` helpers,
IdentityClient descriptor/resource projection, signing-key lifecycle, and
signer-handle seams.
It also exposes ReceiptClient fetch/project/verify/causal-ref projection,
C ABI-backed fetch execution through Runtime Core invoke, and close state seams
over opaque receipt refs, plus PublicationClient resource-ref,
package-validation, deploy/unpublish Invocation carrier, deploy-result, plugin
install projection, published-ability read-model seams, and close state seams. HostBindingClient
exposes binding DTO, envelope decode, item/error/terminal frame encoding, and
output-hash folding seams plus close state seams. MissionClient exposes run/run-file/track/cancel
Invocation carrier builders plus MissionStatus and MissionEventPage projection
seams and close state seams. AdminClient
exposes agent list/start/stop/refresh and session-list Invocation carrier
builders plus GatewayStatus, AdminAgentPage, lifecycle-result, pairing token,
device credential, credential verification, and typed device-session projection
seams plus close state seams. EventClient exposes directory/device/session/invocation subscription
Invocation carriers, device event history pages, and EventFrame
cursor/resume/drop-report/terminal projection seams plus close state seams. SurfaceClient
exposes page list/create/delete/manifest Invocation carriers plus
SurfacePageRecord, SurfacePagePage, SurfaceManifest, SurfacePublicPageRef, and
SurfaceMutationResult projection seams plus SurfaceHealth/SurfaceStatus readiness
seams and close state seams. CompatibilityClient exposes
OpenAI-compatible list-models/chat/stream-chat and file upload/get/delete
Invocation carriers, including C ABI-backed concrete file carrier builders,
plus model, chat, stream, file, and file-delete projection seams and close state
seams. WrapperClient exposes
file, terminal, remote desktop, browser, and media session Invocation carrier
builders, including C ABI-backed concrete carrier builders,
transport-backed helper close state seams, and record projections. Direct daemon
UDS transport, directory subscription live adapters, local signer implementations,
Axon-backed receipt verification, receipt URI construction, live publication/host-binding/mission
execution adapters, mission event streams, concrete Admin trust/session carriers,
certificate policy, Events daemon filtering/live adapters, concrete surface health
live adapters, backend rendering/auth/cache cutover, Compatibility API-key/quota/HTTP/SSE,
multipart storage execution, and product cutovers, wrapper backend HTTP/WebSocket bridges,
profile-specific stream/bidi execution adapters, and the actual EasyRemote
repository cutover remain incomplete. See
`../SDK_PARITY.md` before claiming package stability.

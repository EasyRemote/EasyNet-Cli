# Go Daemon SDK

Go is the P0 facade for the EasyNet backend/Hub. It must expose daemon
lifecycle, Runtime Core, Directory + Identity, Receipt, Events, Admin +
Gateway, Surface, Compatibility, and selected wrapper profiles without
importing Axon packages or generated Axon protobufs in public APIs.

Current status: Runtime Core discovery/daemon-lifecycle/connection/health/
errors/connect-local lifecycle composition/invocation-draft/unary/stream/bidi/handle/prepare-submit plus
Directory + Identity, Receipt, Publication, Host Binding, Mission,
Admin + Gateway, Events multi-stream subscription, Surface page seam, and Compatibility
OpenAI adapter seam, and Convenience Wrapper execution seam
partial. The package exposes typed
feature/version discovery, runtime connection state, runtime health readiness
facts, DaemonHandle lifecycle status/endpoints/start/attach/discover/stop/
detach/open-runtime/connect-local state seams, schema-backed SDK error projection, complete
Invocation draft construction, prepared/signed Invocation DTOs, unary
InvocationResult projection, StreamHandle state observation, BidiSession frame
ordering, half-close, cancel, and terminal-close observation, InvocationHandle
await/cancel/events observation, and RuntimeClient
invoke/invoke-stream/open-bidi/prepare/submit-signed
methods behind narrow JSON transport seams, plus DirectoryClient resolve/list
read-model pages and directory subscription state seams plus IdentityClient descriptor/resource projection,
signing-key lifecycle, and signer-handle seams.
It also exposes ReceiptClient fetch/project/verify/causal-ref projection seams
over opaque receipt refs, plus PublicationClient resource-ref,
package-validation, deploy/unpublish Invocation carrier, deploy-result, plugin
install projection, and published-ability read-model seams. HostBindingClient
exposes binding DTO, envelope decode, item/error/terminal frame encoding, and
output-hash folding seams. MissionClient exposes run/run-file/track/cancel
Invocation carrier builders plus MissionStatus and MissionEventPage projection
seams. AdminClient
exposes agent list/start/stop/refresh and session-list Invocation carrier
builders plus GatewayStatus, AdminAgentPage, lifecycle-result, pairing token,
device credential, credential verification, and typed device-session projection
seams. EventClient exposes directory/device/session/invocation subscription
Invocation carriers, device event history pages, and EventFrame
cursor/resume/drop-report/terminal projection seams. SurfaceClient
exposes page list/create/delete/manifest Invocation carriers plus
SurfacePageRecord, SurfacePagePage, SurfaceManifest, SurfacePublicPageRef, and
SurfaceMutationResult projection seams plus SurfaceHealth/SurfaceStatus readiness
seams. CompatibilityClient exposes
OpenAI-compatible list-models/chat/stream-chat and file upload/get/delete
Invocation carriers plus model, chat, stream, file, and file-delete projection
seams. WrapperClient exposes
file, terminal, remote desktop, browser, and media session Invocation carrier
builders, transport-backed helper seams, and record projections. Concrete daemon
process spawn/default C ABI or UDS transport, directory live transport adapters, local signer implementations,
Axon-backed receipt verification, concrete publication/host-binding/mission
carriers, mission event streams, concrete Admin trust/session carriers,
certificate policy, Events daemon filtering/live adapters, concrete surface health
carriers, backend rendering/auth/cache cutover, Compatibility API-key/quota/HTTP/SSE,
multipart storage execution, and product cutovers, wrapper backend HTTP/WebSocket bridges,
concrete bidi adapters, and backend cutover gates remain incomplete. See
`../SDK_PARITY.md` before claiming package stability.

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
feature/version discovery with root client close and optional `easynet_cabi,cgo`
C ABI v4 feature-discovery, daemon lifecycle/open-runtime, runtime-health,
unary invoke, stream/bidi callback, prepare/sign/submit-handle,
await/cancel/events/free-handle adapters, runtime connection state, runtime health readiness
facts, DaemonHandle lifecycle status/endpoints/start/attach/discover/stop/
detach/open-runtime/connect-local state seams, schema-backed SDK error projection
with stable error classes and profile source-ref accessors, complete Invocation
draft construction, prepared/signed Invocation DTOs, unary
InvocationResult projection, local Ed25519 signer provider over daemon/Axon
canonical signing material, StreamHandle state observation with schema-shaped
terminal event projection, BidiSession frame ordering, half-close, cancel,
terminal-frame projection, and terminal-close observation, InvocationHandle
await/cancel/events/close observation, and RuntimeClient
invoke/invoke-stream/open-bidi/prepare/submit-signed/close
methods behind narrow JSON transport seams, plus DirectoryClient resolve/list
read-model pages, Runtime Core-backed directory subscription streams, directory
subscription state seams, and close state seams plus
IdentityClient descriptor/resource projection, Axon-delegated
URA/DescriptorRef helper seams, signing-key lifecycle, signer workflow
acquisition, and signer-handle provenance/policy guardrails.
It also exposes ReceiptClient fetch/project/verify/verify-chain/causal-ref projection,
invocation-history list/get/trace read-model methods, explicit daemon/Axon
projection provider seams for Runtime-backed project/verify/verify-chain/causal-ref,
optional C ABI v4 concrete transport over Runtime Core invoke, and close state
seams over opaque receipt refs,
plus PublicationClient resource-ref,
package-validation, deploy/unpublish Invocation carrier, deploy-result, plugin
install projection, explicit daemon-local provider seams for Runtime-backed
package validation and plugin install, published-ability read-model seams, and close state seams. HostBindingClient
exposes binding DTO, envelope decode, item/error/terminal frame encoding,
output-hash folding seams, and hash cursor invariant guardrails plus close state
seams. MissionClient exposes run/run-file/track/cancel
Invocation carrier builders plus MissionStatus and MissionEventPage projection
seams, bounded Mission event tailing, MissionPlan EAL rendering, complete child
Invocation fact conformance, and close state seams. AdminClient
exposes agent list/start/stop/refresh and session-list Invocation carrier
builders, explicit daemon-owned GatewayStatus provider seams, GatewayStatus,
AdminAgentPage, lifecycle-result, pairing token, device credential, credential
verification, and typed device-session projection seams plus close state seams.
EventClient exposes directory/device/session/invocation subscription
Invocation carriers, device event history pages, explicit daemon-owned
projection provider seams for Runtime-backed directory/drop/terminal frames,
and EventFrame cursor/resume/drop-report/terminal projection seams plus close state seams. SurfaceClient
exposes page list/create/delete/manifest Invocation carriers plus
SurfacePageRecord, SurfacePagePage, SurfaceManifest, SurfacePublicPageRef, and
SurfaceMutationResult projection seams plus SurfaceHealth/SurfaceStatus readiness
seams and close state seams. CompatibilityClient exposes
OpenAI-compatible list-models/chat/stream-chat and file upload/get/delete
Invocation carriers plus model, chat, stream, file, and file-delete projection
seams and close state seams. WrapperClient exposes
file, terminal, remote desktop, browser, and media session Invocation carrier
builders, transport-backed helper close state seams, and record projections. Direct UDS transport,
Axon-backed receipt verification,
concrete publication/host-binding/mission
carriers, daemon stream-backed mission event adapters, concrete Admin trust/session carriers,
certificate policy, Events daemon filtering/live adapters, concrete surface health
carriers, backend rendering/auth/cache cutover, Compatibility API-key/quota/HTTP/SSE,
multipart storage execution, and product cutovers, wrapper backend HTTP/WebSocket bridges,
and backend cutover gates remain incomplete. See
`../SDK_PARITY.md` before claiming package stability.

# Python Daemon SDK

Python is the P0 facade for EasyRemote and local automation. It may use the C
ABI internally, but EasyRemote product code must not own ctypes loaders, raw
handles, Invocation JSON codecs, or host-stream wire/hash semantics.

Current status: Runtime Core discovery/daemon-lifecycle/connection/health/
errors/invocation-draft/unary/stream/bidi/handle/prepare-submit plus
Directory + Identity, Receipt, Publication, Host Binding, Mission, and
Admin + Gateway seam
partial. The package exposes typed
feature/version discovery, runtime connection state, runtime health readiness
facts, DaemonHandle lifecycle status/endpoints/start/attach/discover/stop/
detach/open-runtime state seams, schema-backed SDK error projection, complete
Invocation draft construction, prepared/signed Invocation DTOs, unary
InvocationResult projection, StreamHandle state observation, BidiSession frame
ordering, half-close, cancel, and terminal-close observation, InvocationHandle
await/cancel/events observation, and RuntimeClient
invoke/invoke-stream/open-bidi/prepare/submit-signed
methods behind narrow transport protocols, plus DirectoryClient resolve/list
read-model pages and IdentityClient descriptor/resource projection seams.
It also exposes ReceiptClient fetch/project/verify/causal-ref projection seams
over opaque receipt refs, plus PublicationClient resource-ref,
package-validation, deploy/unpublish Invocation carrier, deploy-result, plugin
install projection, and published-ability read-model seams. HostBindingClient
exposes binding DTO, envelope decode, item/error/terminal frame encoding, and
output-hash folding seams. MissionClient exposes run/run-file/track/cancel
Invocation carrier builders and MissionStatus projection seams. AdminClient
exposes agent list/start/stop/refresh and session-list Invocation carrier
builders plus GatewayStatus, AdminAgentPage, and lifecycle-result projection
seams. Concrete daemon
process spawn/local transport, directory subscriptions, signer key lifecycle,
Axon-backed receipt verification, concrete publication/host-binding/mission
carriers, mission event streams, pairing/credential/device-session Admin
flows, Events, Surface,
Compatibility, wrappers, concrete bidi adapters, and EasyRemote cutover gates
remain incomplete. See `../SDK_PARITY.md` before claiming package stability.

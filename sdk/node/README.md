# Node/TypeScript Daemon SDK

Node/TypeScript is a P1 facade for desktop tools, extension hosts, and local
developer tooling. It must project the same object graph with promises,
`AsyncIterable` streams, `AbortSignal` cancellation, and explicit close/cancel
operations.

Current status: Runtime Core, Health, Directory + Identity, Receipt,
Publication, Host Binding, Mission, Admin + Gateway, Events, Surface,
Compatibility, and Wrappers seams.
`index.js` and `index.d.ts` expose feature discovery, typed errors, profile
source-ref accessors, Invocation draft construction, RuntimeClient dispatch seams, PreparedInvocation,
SigningMaterial, InvocationSignature, SignedInvocation, stream/bidi lifecycle
handles, bounded retained stream/bidi history with typed terminal backpressure
projection, async iteration, `AbortSignal` cancellation, HealthClient runtime health/diagnostics seams,
InvocationHandle await/cancel/events/free seams,
DirectoryClient read-model resolve/list/subscribe seams, IdentityClient URA/DescriptorRef projection seams,
ReceiptClient fetch/projection/verification/causal-ref seams,
PublicationClient resource/package/deploy/unpublish/read-model/lifecycle seams,
HostBindingClient host-stream codec/hash/lifecycle seams, MissionClient
run/track/cancel/event carrier and projection seams, AdminClient gateway,
agent lifecycle, pairing, and device-session carrier/projection seams, EventClient
directory/device/session/invocation carrier, stream, projection, and device
history seams, SurfaceClient page carrier/projection/health seams,
CompatibilityClient OpenAI-compatible model/chat/file carrier and projection
seams, and WrapperClient file/terminal/remote desktop/browser/media projection
seams over injected transports or the local generic Host Binding transport.

`RuntimeClient.prepare` returns daemon/Axon-provided canonical signing material
as a non-submit-ready `PreparedInvocation`; callers attach an existing caller
signature to produce `SignedInvocation`, and `RuntimeClient.submitSigned`
accepts only that submit-ready state. Node does not construct canonical signing
bytes, acquire signer handles, or perform local daemon signing.

This package has no daemon transport provider, C ABI bridge, local daemon
signing provider, or package-stability claim yet. Its shared conformance
adapter report covers only declared Runtime Core, Health, Directory + Identity,
Errors, MEMC, Receipt, Publication, Host Binding, Mission, Admin + Gateway,
Events, Surface, Compatibility, and Wrappers seam
cases, including Runtime Core stream/bidi backpressure bounds; it must not claim
provider-backed or cutover-ready status.

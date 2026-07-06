# Node/TypeScript Daemon SDK

Node/TypeScript is a P1 facade for desktop tools, extension hosts, and local
developer tooling. It must project the same object graph with promises,
`AsyncIterable` streams, `AbortSignal` cancellation, and explicit close/cancel
operations.

Current status: Runtime Core, Directory + Identity, Receipt, Publication, and
Host Binding seams.
`index.js` and `index.d.ts` expose feature discovery, typed errors, Invocation
draft construction, RuntimeClient dispatch seams, stream/bidi lifecycle handles,
async iteration, `AbortSignal` cancellation, DirectoryClient read-model
resolve/list/subscribe seams, IdentityClient URA/DescriptorRef projection
seams, ReceiptClient fetch/projection/verification/causal-ref seams,
PublicationClient resource/package/deploy/unpublish/read-model/lifecycle seams,
and HostBindingClient host-stream codec/hash/lifecycle seams over injected
transports or the local generic Host Binding transport.

This package has no daemon transport provider, C ABI bridge, or
package-stability claim yet. Its shared conformance adapter report covers only
declared Runtime Core, Directory + Identity, MEMC, Receipt, Publication, and
Host Binding seam cases; it must not claim provider-backed or cutover-ready
status.

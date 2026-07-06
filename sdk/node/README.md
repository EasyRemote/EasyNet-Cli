# Node/TypeScript Daemon SDK

Node/TypeScript is a P1 facade for desktop tools, extension hosts, and local
developer tooling. It must project the same object graph with promises,
`AsyncIterable` streams, `AbortSignal` cancellation, and explicit close/cancel
operations.

Current status: Runtime Core and Directory + Identity seams. `index.js` and
`index.d.ts` expose feature discovery, typed errors, Invocation draft
construction, RuntimeClient dispatch seams, stream/bidi lifecycle handles,
async iteration, `AbortSignal` cancellation, DirectoryClient read-model
resolve/list/subscribe seams, and IdentityClient URA/DescriptorRef projection
seams over injected transports.

This package has no daemon transport provider, C ABI bridge, conformance action
adapter report, or package-stability claim yet. It must not claim
provider-backed or cutover-ready status.

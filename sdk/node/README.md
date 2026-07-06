# Node/TypeScript Daemon SDK

Node/TypeScript is a P1 facade for desktop tools, extension hosts, and local
developer tooling. It must project the same object graph with promises,
`AsyncIterable` streams, `AbortSignal` cancellation, and explicit close/cancel
operations.

Current status: Runtime Core seam. `index.js` and `index.d.ts` expose feature
discovery, typed errors, Invocation draft construction, RuntimeClient dispatch
seams, stream/bidi lifecycle handles, async iteration, `AbortSignal`
cancellation, and explicit close/cancel over injected transports.

This package has no daemon transport provider and no profile clients yet. It
must not claim provider-backed, cutover-ready, or package-stable status.

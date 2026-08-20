# Intent

The canonical Runtime Core stream facade exposed Axon's wire-level `chunk`
label through the C ABI and both direct SDK runtimes. The Go and Python live
smoke tests consume the public contract as `data` for a non-terminal event,
so the projection boundary was split even though all three paths describe the
same stream lifecycle.

This slice makes each stream producer emit the product-neutral SDK event
vocabulary: `data` before the verified terminal boundary and `terminal` at
that boundary. It leaves Bidi frame labels outside this server-stream contract.

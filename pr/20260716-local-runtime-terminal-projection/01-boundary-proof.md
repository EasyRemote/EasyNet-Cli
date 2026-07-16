# Boundary Proof

`InvocationHandle::finalized()` is Axon's canonical terminal proof: it closes
the receipt chain, requires one terminal receipt, verifies the callee signature,
and checks receipt/state agreement. The local RPC adapter owns only conversion
of that verified terminal output into JSON or a daemon error string.

The adapter must not call `wait()` and search events for terminality. Stream
and bidi frame iteration remain separate lifecycle surfaces and are outside
this RPC-only change.

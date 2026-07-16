# Intent

Converge exact daemon stream route lifecycle after the LocalRuntime cutover.

The concrete product use case is `federation.subscribe_directory` and
`federation.subscribe_directory_v2`: subscribers must receive snapshot/delta
frames through Axon's admitted stream path, and the stream must terminate when
the daemon-owned route surface is gone. Runtime registration must not turn a
daemon product stream into an unbounded orphan.

Expected effect: architecture convergence and lifecycle correctness.

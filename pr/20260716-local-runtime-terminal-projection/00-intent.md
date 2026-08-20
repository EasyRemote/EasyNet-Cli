# Intent

The daemon-local RPC adapter reconstructed terminal state from an Axon event
snapshot. That made direct local calls a second terminal-state consumer with a
different proof boundary than Kernel dispatch.

The adapter must consume Axon's finalized invocation projection and decode its
verified terminal payload. This keeps event history observational only.

# Intent

Kernel dispatch previously reconstructed a daemon-local terminal outcome from
an event snapshot. That created a second terminal authority and collapsed
Axon cancellation and timeout states into failure.

The kernel now consumes Axon's finalized lifecycle projection. Its Receipt is
a presentation record that cites the verified terminal receipt hash and
callee signature when Axon admission occurred.

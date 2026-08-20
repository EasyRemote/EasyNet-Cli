# Intent

Converge resolver-selected invocation descriptor binding onto the selected
route's control-plane publication facts.

Concrete use case: when `namespace.resolve` selects a local route from the
live ability publication snapshot, unary/stream/bidi dispatch must stamp the
descriptor-bound ability ref selected by that resolver/control-plane path. A
stale `LocalRuntime` ability option may prove that execution code remains
installed, but it must not be the source of descriptor version/hash/action for
resolver-selected dispatch.

Expected effect: architecture convergence and proof-chain cleanup.

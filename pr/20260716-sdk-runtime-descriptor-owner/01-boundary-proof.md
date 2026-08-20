# Boundary Proof

Addressing owns URA parsing and descriptor-bound subject projection. It does
not own the selected descriptor binding. `RuntimeClient.resolve_descriptor_ref`
is the sole SDK authority for a runtime descriptor because the provider selects
the registered version, hash, action, and geometry.

The shared lowering state machine is:

1. validate complete caller-controlled context;
2. resolve the descriptor through Runtime Core with the exact call mode;
3. project the descriptor-bound subject through addressing;
4. build the immutable invocation draft.

There is no local descriptor-mint fallback when the runtime resolver is absent.

# Boundary Proof

The DeviceLifecycle ability catalog owns `ability.publish` as a daemon-local
ability. The handler owns argument validation, manifest normalization, atomic
write, and the returned publish envelope. Mission and curator flows should call
that ability rather than duplicating manifest-write semantics.

The verified boundary is:

1. build the production registry with Device authority;
2. materialize a real local Agent root;
3. invoke `ability.publish` through the same local RPC dispatch table as
   production;
4. assert the returned envelope names the public ability; and
5. assert the manifest exists on disk at the returned path.

This is a coverage/proof slice, not a new compatibility path.

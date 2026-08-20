# Intent

Go and Python exposed the same `RuntimeAbilityClient` capability but used two
different descriptor-binding authorities. Go asked `RuntimeClient` to resolve a
registered descriptor for its call mode; Python assembled a descriptor ref from
addressing data and reused that RPC-shaped path for streams.

This slice converges Python on the runtime-owned resolver. Descriptor version,
hash, action, and call mode now come from the same provider boundary in both
facades.

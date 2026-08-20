Architecture
============

Root abstraction
----------------

`CallMode` is part of the descriptor identity used by routing and descriptor
proofs. Treating it as a defaultable enum makes RPC a silent fallback path.

Boundary decision
-----------------

Remove `Default` from the enum. Code that needs RPC must say
`CallMode::Rpc`. This preserves current public behavior while preventing new
generic default construction from manufacturing an RPC descriptor.

The first compile failure identified owner-projection `AbilityCallableSummary`
as a second implicit RPC read model. That summary now uses an explicit
constructor, and missing `callable_summary` fails deserialization instead of
being tolerated as a lossy proto-shaped compatibility row.

Layering
--------

This is a core runtime descriptor contract. SDKs and product facades should
consume explicit call-mode facts rather than infer a product-friendly default.

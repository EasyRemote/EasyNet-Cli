Architecture
============

Root abstraction
----------------

`OwnerKind` is the typed runtime owner model used by the ability registry.
`AuthorityScope.owner_projection` stores its persisted control-plane marker.
Those two abstractions must be inverses over one grammar.

Boundary decision
-----------------

Move the `OwnerKind` projection inverse from `hub` to `authority`, matching the
runtime authority scope state machine. Rejecting `hub` avoids a second owner
grammar that would preserve product Hub vocabulary inside core runtime facts.

Layering
--------

Core runtime owns `OwnerKind` and `AuthorityScope`. Product surfaces may keep
Hub-oriented labels until migrated in separate CLI/product slices.

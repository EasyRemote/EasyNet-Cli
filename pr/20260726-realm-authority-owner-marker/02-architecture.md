Architecture
============

Root abstraction
----------------

`AuthorityScope.owner_projection` is an internal runtime governance key. It is
not a CLI deployment mode and not a product label. Encoding the realm Authority
plane as `hub` couples core admission facts to a historical product concept.

Boundary decision
-----------------

Move the owner-projection state machine to the generic `authority` marker for
realm Authority scopes. The parser rejects retired `hub` markers instead of
normalizing them, because normalization would preserve a compatibility layer.

Layering
--------

Core runtime authority binding owns the marker grammar. Product-facing
commands, trust files, and legacy route names are separate surfaces and must be
migrated in dedicated slices.

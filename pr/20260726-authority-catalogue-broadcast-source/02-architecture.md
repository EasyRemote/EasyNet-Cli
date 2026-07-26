Architecture
============

Root abstraction
----------------

`meta.list_abilities` projects canonical descriptors to SDK and product
consumers. Its `source` value is part of the runtime catalogue DTO consumers
see, so it must use canonical runtime vocabulary.

Boundary decision
-----------------

Replace `hub:broadcast` with `authority:broadcast` for realm-scope broadcast
rows. Keep wire names that are not part of this DTO in place until a dedicated
federation-wire migration.

Layering
--------

Core catalogue projection owns source metadata. Federation transport may still
decode legacy-named wire fields below this layer.

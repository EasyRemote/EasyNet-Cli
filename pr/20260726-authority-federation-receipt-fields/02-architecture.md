Architecture
============

Layering
--------

- `receipt_contract.rs` owns the daemon federation receipt DTOs.
- Federation wrappers produce the canonical DTO field names.
- Session prelude/heartbeat consumers deserialize the same canonical DTOs and
  update `AuthorityPublishedAbilityStore`.

Boundary proof
--------------

This is a producer/consumer in-repository protocol migration. The retired Hub
field names are removed from the canonical DTOs instead of preserved as serde
aliases, because aliases would keep a second receipt model alive.

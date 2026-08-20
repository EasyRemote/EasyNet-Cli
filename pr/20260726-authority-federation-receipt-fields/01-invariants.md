Invariants
==========

- Missing ability publication facts remain terminal protocol errors.
- Empty ability snapshots and diffs are valid only when explicitly serialized.
- The device read model still validates every descriptor before caching it.
- Revision-only heartbeat diffs still advance the observed revision.
- No dual-field compatibility path may accept both old and new field names.

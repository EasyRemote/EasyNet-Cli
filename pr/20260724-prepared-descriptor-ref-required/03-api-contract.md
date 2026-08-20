# API Contract

Public surface remains structurally unchanged:

- `PreparedInvocation.descriptor_ref` / `DescriptorRef()` / `descriptorRef()`
  remains present.
- Existing valid prepare responses continue to decode and sign.
- Invalid prepare responses missing top-level `descriptor_ref` now fail with
  `INVALID_ARGUMENT`.

This is not a compatibility layer removal at the wire schema level; it is a
validation tightening that prevents SDKs from synthesizing canonical facts.

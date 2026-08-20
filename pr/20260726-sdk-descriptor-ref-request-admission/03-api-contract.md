# API Contract

No public method names change.

Go:

- `RuntimeClient.ResolveDescriptorRef(ctx, RuntimeDescriptorRefRequest)` now fails locally when `callee_ura`, `ability`, or `call_mode` are blank.
- When `Provider` is set, `CallerURA` and `SubjectURA` are required and must not contain the all-zero principal placeholder.

Python:

- `RuntimeClient.resolve_descriptor_ref(...)` now fails locally when `callee_ura`, `ability`, or `call_mode` are blank.
- When `provider` is set, `caller_ura` and `subject_ura` are required and must not contain the all-zero principal placeholder.

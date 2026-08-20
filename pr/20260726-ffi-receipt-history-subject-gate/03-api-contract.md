Public contract
===============

For FFI `runtime_resolve_descriptor_ref_json`:

- `provider` remains optional for generic descriptor catalog lookup.
- `provider: "ability_descriptor"` remains restricted to catalogue abilities.
- `provider: "receipt_history"` remains restricted to receipt-history abilities.
- `provider: "receipt_history"` now also requires `subject_ura`.
- The receipt-history `subject_ura` must be a canonical user-owned
  runtime-state read resource subject.

Error contract
==============

- Missing `subject_ura` is `INVALID_ARGUMENT` at SDK/request stage.
- Device subject, retired session subject, malformed URA, and all-zero user
  placeholder are `INVALID_ARGUMENT` at SDK/request stage.
- These errors occur before catalog lookup or remote route state is consulted.

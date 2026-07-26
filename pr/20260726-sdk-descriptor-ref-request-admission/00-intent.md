# Intent

## Goal

Make Go and Python RuntimeClient descriptor-ref resolution reject incomplete runtime descriptor requests before provider transport.

## Non-goals

- Do not change daemon C ABI or Rust provider wire shape.
- Do not remove generic descriptor catalogue lookup.
- Do not add product-specific descriptor lifecycle or EasyNet/EasyRemote naming.

## Acceptance criteria

- Go and Python RuntimeClient both require `callee_ura`, `ability`, and `call_mode`.
- Provider-backed descriptor resolution requires explicit `caller_ura` and `subject_ura`.
- All-zero principal placeholders are rejected before transport.
- Existing high-level RuntimeAbility and Receipt providers continue to pass through the same provider-backed path.

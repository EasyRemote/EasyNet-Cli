# Intent

## Goal

Close the FFI descriptor resolver gap where `provider: "ability_descriptor"` accepts catalogue-read requests without proving the canonical catalogue subject. The Go and Python SDK providers already lower runtime descriptor catalogue reads through the realm authority subject; the native FFI resolver must enforce the same model instead of allowing device/user subjects to flow into daemon routing and fail later as route, signer, or authority errors.

## Non-goals

- Do not add EasyNet- or EasyRemote-specific catalogue concepts to the SDK or FFI API.
- Do not add compatibility aliases for legacy device-owned catalogue reads.
- Do not change public descriptor request or response fields.
- Do not mask missing route/signature state with fallback descriptor synthesis.

## Acceptance criteria

- Explicit `ability_descriptor` provider requests require `subject_ura`.
- The subject must be a canonical realm authority URA matching the callee realm.
- Device, user, resource, malformed, empty, all-zero, and cross-realm subjects fail before daemon route/admission I/O.
- Existing Go/Python provider behavior remains aligned with the FFI native boundary.
- SPEC v2 gate covers the new FFI invariant.

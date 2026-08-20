# Node Profile Error Source Refs Intent

Implement Node/TypeScript profile-originated SDK error provenance so shipped
Node profile seams converge with the Go/Python error model.

## Scope

- Add stable Node package source references for SDK profiles.
- Expose `SDKError.profile()`, `SDKError.sourceRef()`, and `SDKError.errorClass()`.
- Attach `profile` and `source_ref` details to Node profile validation errors.
- Preserve existing error schema fields and existing details.
- Declare Node evidence for `error/profile_source_refs` only after tests cover
  package helper, accessors, and detail preservation.

## Out Of Scope

- No top-level error schema changes.
- No legacy error-code aliases.
- No daemon provider or C ABI error transport changes.

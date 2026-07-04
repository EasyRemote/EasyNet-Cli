# Go Profile Error Source Refs Intent

Implement stable per-profile source references for Go SDK profile errors,
aligned with the Python SDK profile error metadata slice.

This slice keeps the existing `SDKError` structure, error codes, stages, retry
hints, and public client APIs unchanged. It only fills `SDKError.Details` with:

- `profile`: the owning SDK profile.
- `source_ref`: a stable Go SDK profile source reference.

Existing detail keys must be preserved.

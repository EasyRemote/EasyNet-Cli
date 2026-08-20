# Python Profile Error Source Refs Intent

Implement per-profile SDK error source references for the Python SDK profiles
required by `docs/spec/daemon-sdk-requirements-v1.md`.

The Python SDK already exposes typed `SDKError` values with stable code, stage,
retry, source, invocation id, receipt URA, and details fields. This slice should
make profile-originated errors traceable without changing the daemon protocol or
the shared error schema:

- Profile validation errors keep their existing `stage`.
- `details.profile` identifies the SDK profile that produced the error.
- `details.source_ref` gives a stable profile-scoped source reference.
- Existing reason, method, and transport details are preserved.

This must not introduce daemon-side behavior, direct Axon/protobuf exposure, or
new URA parsing logic in profile clients.

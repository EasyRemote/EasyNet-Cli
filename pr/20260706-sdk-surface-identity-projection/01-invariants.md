# Invariants

- Do not modify `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not introduce a package-level Surface URA helper.
- Do not parse or concatenate URA grammar in the Surface profile when an
  Identity facade projection is available.
- Preserve existing public Surface DTOs and request/response shape.
- Keep Surface product semantics scoped to pages/surface DTO projection; Axon
  still owns canonical URA grammar through the daemon/Identity boundary.
- Tests must prove `BuildURA(kind=resource)` is used when daemon output omits a
  resource ref.

# Invariants

- Do not modify `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not construct `device.<node_id>` or any other owner id inside Publication.
- Do not catch `TypeError` to support an older resource URA builder signature.
- Keep `sdk_resource_ura(owner_ura, path)` as the no-custom-addressing path.
- Keep custom addressing restricted to `resource_ura(owner_ura, path)`.
- Tests must prove legacy three-argument resource builders are rejected.

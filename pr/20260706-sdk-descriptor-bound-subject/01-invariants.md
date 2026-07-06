# Invariants

- Do not modify `docs/spec/daemon-sdk-requirements-v1.md`.
- Keep the Go SDK import boundary clean: no `easynet.run/axon`, protobuf, daemon internals, or C ABI imports.
- Provide Go and Python parity.
- Keep the projection generic: no EasyNet backend naming, no product-specific subject lifecycle.
- Preserve URA terminology only.
- Do not implement URA grammar in Go/Python; delegate to Identity
  `build_ura(kind=resource)`.
- Do not change transport-backed identity behavior except adding the resource
  path field to the existing request DTO.

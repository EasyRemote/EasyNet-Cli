Intent

Converge AccessControl ability route names onto one provider route manifest.
The same route list currently exists as handwritten constants in the Go SDK,
Python SDK, and daemon governance ability names. This slice makes the manifest
the editable source of truth and generates language-local constants from it.

Scope

- Add an AccessControl route manifest under `provider_routes/`.
- Generate Go SDK, Python SDK, and daemon Rust route constants from the
  manifest.
- Preserve existing package-private Go names, Python module constants, and
  daemon governance constants as compatibility aliases.
- Do not change AccessControl request/response DTOs, provider behavior,
  admission policy semantics, descriptor metadata, or public SDK APIs.

Expected effect

- Architecture convergence: one authority/policy/admission route source feeds
  every runtime facade.
- Product consistency: Go and Python SDK providers invoke the same route table.
- Product acceleration: future access-control route additions update one
  manifest and fail deterministic generation checks when a binding is stale.

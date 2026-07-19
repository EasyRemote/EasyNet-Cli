# Intent

## Goal

Implement the remaining canonical runtime convergence work required by
`docs/spec/canonical-runtime-convergence-v2.md` for the SDK helper/facade matrix.

## Non-goals

- Do not add product-specific concepts to canonical SDK roots.
- Do not expose plugin sidecar helpers as canonical runtime root APIs.
- Do not enable language templates that would hand-roll the sidecar JSON frame
  contract.
- Do not preserve legacy helper paths unless the SPEC explicitly requires an
  edge adapter.

## Acceptance Criteria

- Provider-scoped sidecar helper contract is explicit.
- Python and Go helpers remain provider-backed or cutover-ready only under the
  EasyNet provider namespace.
- Rust, Node, Java, and C/C++ helper/template states are recorded as seams or
  unsupported until provider-backed helpers exist.
- `plugin init` only exposes languages with provider-backed helpers.
- Automated gates reject naked sidecar frame parsing in plugin templates.
- Canonical convergence gates remain green.

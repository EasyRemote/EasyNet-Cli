# Evidence

## Source exploration

- `sdk/conformance/canonical-public-api.json` exposes `RuntimeHost`, `RuntimeHandle` and related members in the canonical inventory.
- `sdk/conformance/canonical-public-api.json` classifies daemon/control compatibility names under its non-canonical surface.
- `sdk/SDK_INTERFACE_SPEC.md` still listed `DaemonControl -> DaemonHandle` as part of the canonical object graph.
- `sdk/SDK_PARITY.md` still described the machine-readable source as only `sdk-parity-matrix.json`, while current gates generate/validate the matrix from `canonical-public-api.json`.
- `docs/spec/daemon-sdk-requirements-v1.md` already states the stronger provider boundary and runner-owned evidence semantics.

## Intended convergence

The docs should not create a second lifecycle vocabulary. Public canonical docs use `RuntimeHost`; daemon-named compatibility remains documented as provider/source-compatibility surface only.

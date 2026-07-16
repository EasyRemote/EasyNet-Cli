# SDK Provider Boundary Docs Plan

## Goal

Converge SDK architecture documents around one boundary: the SDK defines the canonical runtime model, while the EasyNet provider ABI binds that model to `easynet-daemon`.

## Root fork

`docs/spec/daemon-sdk-requirements-v1.md` has moved toward `RuntimeHost` and explicit provider ABI ownership, but `sdk/SDK_INTERFACE_SPEC.md` still lists `DaemonControl -> DaemonHandle` as canonical SDK graph. That leaves two public documentation authorities for the same lifecycle surface.

## Boundary decision

- Canonical SDK docs should name `RuntimeHost -> RuntimeHandle` in the public object graph.
- `Daemon*` names are source-compatibility/provider aliases, not canonical runtime concepts.
- The `easynet_*` C ABI v5 is an EasyNet provider ABI. Its generic property is operation-family shape, not product neutrality.
- Capability-state evidence comes from the canonical public API concept schema and runner-owned live conformance, not committed report claims.

## Edit sequence

1. Keep the normative requirement update in `docs/spec/daemon-sdk-requirements-v1.md`.
2. Align `sdk/SDK_INTERFACE_SPEC.md` object graph and provider/C ABI wording.
3. Align `sdk/SDK_PARITY.md` with the canonical-public-api source and seven-language matrix.
4. Verify public API, parity, scaffold, architecture and project-structure gates.

# EasyNet-Cli Project Structure

This repository follows `docs/spec/project-structure-v1.md` and stages the
Daemon SDK layout from `docs/spec/daemon-sdk-requirements-v1.md`.

## Stable Roots

| Path | Owner |
| --- | --- |
| `src/daemon/` | daemon process lifecycle, Invocation runtime, identity, resources, plugins, and Axon adapter boundary |
| `src/ffi/` | C ABI handles, error mapping, and Runtime Core projection |
| `include/easynet_cli.h` | binding-facing C ABI contract |
| `sdk/` | public Daemon SDK docs, schemas, conformance assets, and language facades |
| `schemas/` | daemon/control-plane protocol files outside the SDK JSON projection |
| `tests/scripts/` | shell guard script contract tests |
| `tools/scripts/` | repository guard and release helper scripts |
| `packaging/` | release and installer shape |

## SDK Roots

The current SDK stage contains:

- `sdk/README.md`
- `sdk/SDK_INTERFACE_SPEC.md`
- `sdk/SDK_PARITY.md`
- `sdk/CONFORMANCE_SUITE.md`
- `sdk/schemas/`
- `sdk/conformance/cases/`
- `sdk/conformance/fixtures/`
- `sdk/conformance/runner/`
- `sdk/go/`, `sdk/python/`, `sdk/node/`, `sdk/java/`, `sdk/swift/`, `sdk/rust/`

`sdk/rust/` is limited to provider/runtime SDK packages and must not own
EasyNet product behavior or daemon lifecycle. `sdk/c/` remains a migration
target, not an active root. Native Rust product behavior and product-specific
projections live with their semantic owners under `src/daemon/`; generic C ABI
projection lives under `src/ffi/`.
The C binding contract lives in `include/easynet_cli.h` and
`docs/spec/ffi-abi-v7.md` with exact export allowlists
`include/easynet_cli.exports.v7` and `include/easynet_cli.exports.v8`.

## Structural Rules

- Product code must not depend on raw Axon SDK/proto/runtime paths for EasyNet
  daemon product flows.
- Language SDK directories may own idiomatic facade code only; daemon lifecycle,
  canonical Invocation material, receipt semantics, and stream/bidi state
  machines remain owned by Rust daemon SDK core or Axon delegation.
- Examples and gallery code may import SDK packages; SDK packages must not
  import examples or gallery code.
- Release packaging must ship the current C ABI header and current ABI spec
  together.

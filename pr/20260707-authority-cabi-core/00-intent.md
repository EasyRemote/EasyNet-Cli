# Authority C ABI Core Intent

## Objective

Move authority metadata signing material and wire metadata materialization into
the EasyNet-Cli Rust daemon SDK core and expose it through `libeasynet_cli`.

The target dependency direction remains:

```text
Axon semantics -> EasyNet-Cli daemon/core -> C ABI -> Go/Python SDK facades
```

Language SDKs may request signing material and pass signatures back. They must
not own canonical authority payload construction or daemon admission metadata
wire shape.

## Non-Goals

- Do not add private-key signing to the C ABI.
- Do not make Go/Python import Axon packages.
- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not retain a legacy authority path once concrete facade transports are
  ready.

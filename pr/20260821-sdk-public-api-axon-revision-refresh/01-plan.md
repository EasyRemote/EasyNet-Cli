# SDK public API Axon revision refresh

Date: 2026-08-21

## Problem

After Axon `codex/lifecycle-authority-binding-refresh` advanced to
`028c35585d21c953e8fc41bfbda1f4128d163ce8`, the EasyNet-Cli SDK canonical
public API manifest still recorded the prior Axon source revision
`e6e84299c9a0ea52c5f1fa04c2a46d1119a0096a`.

`check-sdk-cutover-readiness.sh` correctly rejected the stale manifest with:

```text
sdk_concepts: inventory_source_revision_mismatch:rust:
expected=e6e84299c9a0ea52c5f1fa04c2a46d1119a0096a:
actual=028c35585d21c953e8fc41bfbda1f4128d163ce8
```

## Boundary invariants

- The SDK public API manifest is generated evidence, not hand-authored API
  design.
- The SDK remains product-neutral; this refresh must not introduce EasyNet,
  EasyRemote, daemon, device, or plugin concepts into the canonical runtime
  model.
- The change must be limited to source-revision attestation unless the owner
  generator detects a real API shape delta.

## Implementation

Run the owner generator:

```bash
source sdk/conformance/toolchain_path.sh
resolve_sdk_toolchain_path "$PWD"
source sdk/conformance/python_toolchain.sh
resolve_sdk_python_toolchain "$PWD" pytest
"$SDK_CONFORMANCE_PYTHON" sdk/conformance/rebuild_public_api_model.py --write
```

## Expected effect

- `sdk/conformance/canonical-public-api.json` records Axon revision
  `028c35585d21c953e8fc41bfbda1f4128d163ce8`.
- `check-sdk-canonical-public-api.sh` passes.
- Full `check-sdk-cutover-readiness.sh` passes.

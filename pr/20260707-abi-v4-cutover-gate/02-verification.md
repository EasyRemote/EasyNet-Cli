# Verification

Status: Passed.

Commands run:

```sh
bash tools/scripts/check-ffi-abi-v4-header.sh --self-test
bash tools/scripts/check-ffi-abi-v4-header.sh
bash tools/scripts/check-sdk-cutover-readiness.sh --self-test
bash tools/scripts/check-sdk-completion-audit.sh
git diff --check
```

Notes:

- `check-sdk-completion-audit.sh` now visibly runs `FFI ABI v4 header` through
  `check-sdk-cutover-readiness.sh`.
- `check-sdk-cutover-readiness.sh --self-test` includes the FFI ABI v4 header
  self-test.
- The daemon SDK SPEC gate text now names ABI v4 as the active
  header/export/version check.

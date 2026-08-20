# Verification

Planned commands:

```text
bash tools/scripts/check-sdk-completion-audit.sh --self-test
bash tools/scripts/check-sdk-completion-audit.sh
bash tools/scripts/check-sdk-scaffold.sh
git diff --check
```

Expected result: the completion audit passes only when aggregate cutover
readiness passes and the Go/Python matrix stays provider-backed or stronger for
every capability row.

# Verification

Planned commands:

```text
bash tools/scripts/check-sdk-conformance-reports.sh --self-test
bash tools/scripts/check-sdk-conformance-reports.sh
bash tools/scripts/check-sdk-cutover-readiness.sh --self-test
bash tools/scripts/check-sdk-cutover-readiness.sh
bash tools/scripts/check-sdk-scaffold.sh
git diff --check
```

Expected result: all commands pass, and a missing report record fails in the
self-test before any cutover-ready claim can be accepted.

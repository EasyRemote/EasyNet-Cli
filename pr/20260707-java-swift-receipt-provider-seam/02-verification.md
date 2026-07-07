# Verification

Run after implementation:

```sh
bash tools/scripts/check-java-sdk-seam.sh
bash tools/scripts/check-swift-sdk-seam.sh
bash tools/scripts/check-sdk-conformance-reports.sh
bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-completion-audit.sh
git diff --check
```

Expected result: all commands pass.

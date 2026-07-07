# Verification

Run:

```sh
bash tools/scripts/check-java-sdk-seam.sh
bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-ura-naming.sh
git diff --check
```

Expected result: all commands pass. Java remains a Runtime Core seam and is not
added to provider-backed conformance reports.

Observed result:

- `bash tools/scripts/check-java-sdk-seam.sh`: passed.
- `bash tools/scripts/check-sdk-scaffold.sh`: passed.
- `bash tools/scripts/check-sdk-ura-naming.sh`: passed.
- `bash tools/scripts/check-sdk-parity-matrix.sh`: passed.
- `bash tools/scripts/check-sdk-conformance-reports.sh`: passed.
- `git diff --check`: passed.

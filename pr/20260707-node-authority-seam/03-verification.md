# Verification

Run:

```sh
npm test --prefix sdk/node
bash tools/scripts/check-node-sdk-seam.sh
bash tools/scripts/check-sdk-ura-naming.sh
bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-parity-matrix.sh
bash tools/scripts/check-sdk-conformance-reports.sh
cargo run --bin sdk-conformance-runner -- --language node --adapter-report sdk/conformance/runner/node-action-adapter-report.json --format jsonl
git diff --check
bash tools/scripts/check-sdk-completion-audit.sh
```

Expected result: Node authority seam tests pass, Node remains undeclared for
the provider-backed `authority/mutual_exclusion` case, and all aggregate gates
remain clean.

Observed result:

- `npm test --prefix sdk/node`: passed, including Node authority metadata
  projection and mutual-exclusion tests.
- `bash tools/scripts/check-node-sdk-seam.sh`: passed.
- `bash tools/scripts/check-sdk-ura-naming.sh`: passed.
- `bash tools/scripts/check-sdk-scaffold.sh`: passed.
- `bash tools/scripts/check-sdk-parity-matrix.sh`: passed.
- `bash tools/scripts/check-sdk-conformance-reports.sh`: passed.
- `cargo run --bin sdk-conformance-runner -- --language node --adapter-report sdk/conformance/runner/node-action-adapter-report.json --format jsonl`:
  passed declared Node seam cases and left `authority/mutual_exclusion`
  undeclared for Node.
- `git diff --check`: passed.
- `bash tools/scripts/check-sdk-completion-audit.sh`: passed.

# Verification

Run after implementation:

```sh
npm test --prefix sdk/node
bash tools/scripts/check-node-sdk-seam.sh
bash tools/scripts/check-sdk-ura-naming.sh
bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-parity-matrix.sh
bash tools/scripts/check-sdk-conformance-reports.sh
cargo run --bin sdk-conformance-runner -- --language node --adapter-report sdk/conformance/runner/node-action-adapter-report.json --format jsonl
git diff --check
```

Expected evidence:

- Node tests cover Events carrier delegation, stream binding, event projection,
  drop report projection, terminal projection, device history pages, and session
  `session_id` enforcement.
- Node conformance report passes declared Events cases.

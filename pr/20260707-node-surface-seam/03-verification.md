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

- Node tests cover Surface carrier delegation, page/manifest/ref/mutation/health
  projection, status aliasing, validation failure paths, and no descriptor
  construction in the facade.
- Node conformance report passes `surface/page_carriers`.

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

- Node tests cover Compatibility carriers, model/chat/file projections, stream
  normalization, provider nickname rejection, and unary `stream: true`
  rejection.
- Node conformance report passes `compatibility/openai_carrier_projection`.

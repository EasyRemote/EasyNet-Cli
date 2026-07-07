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
```

Expected result: all commands pass. Node is declared for
`stream/backpressure_bound` once retained stream/bidi history stays bounded and
overflow is projected as a typed terminal backpressure state.

Observed result:

- `npm test --prefix sdk/node`: passed, including StreamHandle and BidiSession
  bounded-history overflow tests.
- `bash tools/scripts/check-node-sdk-seam.sh`: passed.
- `bash tools/scripts/check-sdk-ura-naming.sh`: passed.
- `bash tools/scripts/check-sdk-scaffold.sh`: passed.
- `bash tools/scripts/check-sdk-parity-matrix.sh`: passed.
- `bash tools/scripts/check-sdk-conformance-reports.sh`: passed.
- `cargo run --bin sdk-conformance-runner -- --language node --adapter-report sdk/conformance/runner/node-action-adapter-report.json --format jsonl`:
  passed `stream/backpressure_bound` for Node.
- `git diff --check`: passed.

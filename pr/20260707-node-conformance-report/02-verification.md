# Verification

Required commands:

```text
npm test --prefix sdk/node
cargo run --bin sdk-conformance-runner -- --language node --adapter-report sdk/conformance/runner/node-action-adapter-report.json --format jsonl
bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-cutover-readiness.sh
```

Expected evidence:

1. Node unit tests still prove Runtime Core async state machines plus Directory
   + Identity seam behavior over injected transports.
2. The shared conformance runner passes MEMC plus Node-declared Receipt cases
   and skips undeclared P0/provider-backed cases.
3. Scaffold validation requires the new Node report.
4. Aggregate readiness remains green for P0 Go/Python/C ABI and product
   boundary gates.

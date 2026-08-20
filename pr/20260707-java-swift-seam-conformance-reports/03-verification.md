# Verification

## Commands

- `bash tools/scripts/check-sdk-conformance-reports.sh`
- `cargo run --quiet --bin sdk-conformance-runner -- --root /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli --language java --adapter-report sdk/conformance/runner/java-action-adapter-report.json --format json`
- `cargo run --quiet --bin sdk-conformance-runner -- --root /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli --language swift --adapter-report sdk/conformance/runner/swift-action-adapter-report.json --format json`
- `bash tools/scripts/check-java-sdk-seam.sh`
- `bash tools/scripts/check-swift-sdk-seam.sh`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-ura-naming.sh`

## Expected Evidence

- Java and Swift action-adapter reports validate through `sdk-conformance-runner`.
- Java and Swift seam tests remain the evidence source for their reports.
- The Go/Python parity matrix stays a P0 provider-backed matrix and does not become a product or non-P0 transport claim.

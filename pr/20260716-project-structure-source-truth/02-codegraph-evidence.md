# CodeGraph-style evidence

Commands used before edits:

```sh
find src/bin -maxdepth 1 -type f | sort
rg -n "real-publish-smoke|sdk-conformance-runner|verify-voice-contract" \
  Cargo.toml docs/spec/project-structure-v1.md tools tests src/bin
bash tools/scripts/check-project-structure-v1.sh
```

Findings:

- `src/bin/real-publish-smoke.rs` is absent and no longer appears in
  `Cargo.toml`.
- `src/bin/verify-voice-contract.rs` exists and is exercised by
  `tools/scripts/check-voice-call-product-contract.sh`.
- `sdk-conformance-runner` is a workspace crate at
  `tools/sdk-conformance-runner`, not a `src/bin` entry.
- `provider_routes` is consumed by generated Rust/Go/Python route constants
  and SDK tests.
- `ability-descriptors/system/federation` contains grouped federation
  descriptors and is not a flat descriptor violation.
- local ignored `__pycache__` directories made the gate fail against generated
  output; they were removed from the working tree before verification.

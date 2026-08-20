# Swift Async Sequence Seam Verification

## Commands

```bash
bash tools/scripts/check-swift-sdk-seam.sh
bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-conformance-reports.sh
bash tools/scripts/check-sdk-ura-naming.sh
git diff --check
```

## Expected Evidence

- Swift tests consume `StreamHandle` and `BidiSession` with `for try await`.
- Terminal stream/bidi items are yielded once before iteration ends.
- The Swift seam remains dependency-free and provider-injected.

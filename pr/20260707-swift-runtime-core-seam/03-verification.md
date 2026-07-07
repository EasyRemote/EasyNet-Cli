# Swift Runtime Core Seam Verification

## Commands

```bash
bash tools/scripts/check-swift-sdk-seam.sh
bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-ura-naming.sh
git diff --check
```

## Expected Coverage

- Swift sources compile with warnings treated as errors.
- Runtime Core seam tests execute feature discovery, typed errors, complete Invocation draft construction, injected runtime dispatch, stream bounded history, bidi bounded history, and closed-state rejection.
- Static guard rejects legacy address spelling and public Axon/protobuf symbols in `sdk/swift`.
- Shared SDK scaffold requires the Swift seam files and runs the Swift seam guard.

## Non-Claims

- No Swift action-adapter report exists.
- No Swift Package Manager metadata exists.
- No daemon or C ABI transport exists.
- No provider-backed or cutover-ready Swift capability is claimed.

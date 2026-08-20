# Swift Package Manager Seam Verification

## Commands

```bash
bash tools/scripts/check-swift-sdk-seam.sh
bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-ura-naming.sh
git diff --check
```

## Expected Evidence

- `check-swift-sdk-seam` runs `swift test` from `sdk/swift`.
- XCTest imports `EasyNetDaemonSDK` as a public package target.
- Static guard rejects legacy address spelling and public protocol-wire symbols in `sdk/swift`.
- Shared scaffold requires `Package.swift`, the XCTest file, and the Swift seam guard.

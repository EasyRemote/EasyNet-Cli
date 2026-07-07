# Swift Runtime Core Seam Plan

## Goal

Move `sdk/swift` from placeholder to a P1 Runtime Core seam without changing the SDK architecture direction. The seam must expose generic runtime concepts only and must not claim daemon-provider, package-stable, or product-cutover support.

## Scope

- Add dependency-free Swift value types and clients for typed SDK errors, feature discovery, Invocation draft construction, injected runtime transport dispatch, and bounded stream/bidi state.
- Add a Swift seam guard to the shared scaffold check.
- Update SDK status documentation to report Swift as `seam`, not provider-backed.
- Leave Swift Package Manager metadata, C ABI/daemon transports, profile clients, and action-adapter reports unsupported.

## Verification

- `bash tools/scripts/check-swift-sdk-seam.sh`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-sdk-ura-naming.sh`
- `git diff --check`

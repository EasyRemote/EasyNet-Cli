# Swift Package Manager Seam Plan

## Goal

Convert the Swift Runtime Core seam into a real Swift Package Manager package while keeping its capability state at `seam`.

## Scope

- Add `sdk/swift/Package.swift` for the `EasyNetDaemonSDK` library.
- Move the temporary direct-run Swift test entrypoint into an XCTest package test target.
- Update the Swift seam guard to run `swift test` as the package verification boundary.
- Update SDK status documentation and scaffold checks so Swift Package Manager metadata is no longer listed as missing.

## Non-Scope

- No daemon or C ABI transport provider.
- No profile clients beyond Runtime Core seam objects.
- No provider-backed Swift transport report.
- No provider-backed or cutover-ready Swift claim.

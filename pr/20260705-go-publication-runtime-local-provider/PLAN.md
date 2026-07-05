# Go Publication Runtime Local Provider

## Goal

Implement Go Publication runtime package validation and plugin install through an explicit daemon-local provider while preserving Runtime Core invocation ownership for ability publication carriers.

## Boundary Proof

- `PublicationRuntimeTransport` continues to own Runtime Core invocation lowering for deploy/list/show/enable/disable/unpublish operations.
- Package validation and plugin install remain daemon-local implementation-resource operations supplied by an explicit provider.
- The C ABI Publication transport already implements the provider shape and remains the Rust-owned package/plugin path.
- Go runtime transport validates provider output against existing Publication DTO constructors before returning it.
- No plugin manifest parser, package installer, filesystem policy, or host runtime execution is introduced in Go.

## Invariants

- Runtime publication ValidatePackage and InstallPlugin fail closed without a configured provider.
- Provider output must decode as the public PackageValidation or PluginInstallResult DTO.
- Existing request JSON and public client methods remain unchanged.
- Ability publication Runtime invocation behavior remains untouched.
- No retired address terminology is introduced in touched files.

## Verification

- `go test -count=1 ./...` in `sdk/go`.
- `go test -count=1 -tags easynet_cabi ./...` in `sdk/go`.
- `cargo fmt --check`.
- `bash tools/scripts/check-sdk-scaffold.sh`.
- `git diff --check`.
- Retired address terminology scan over touched files.

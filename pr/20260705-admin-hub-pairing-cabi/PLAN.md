# Admin Hub Pairing C ABI Contract

## Goal

Complete the Admin + Gateway hub lifecycle and pairing/device-credential C ABI contract across Rust, Go, and Python facades.

## Boundary Proof

- Rust daemon contract owns carrier construction and projection semantics.
- C ABI exports schema-backed carrier/projection functions only; Runtime Core remains the execution path.
- Go and Python C ABI transports call the exported contract instead of preserving unsupported stubs.
- Product onboarding, certificate authority policy, browser sessions, and backend auth remain outside the SDK.

## Invariants

- Hub join/leave and pairing/trust methods lower to complete Invocation carriers before dispatch.
- Projection functions require daemon result context and validate hub/device identifiers before returning typed DTOs.
- Language facades do not hand-build carrier semantics independently of the Rust contract.
- No retired address terminology is introduced in touched files.

## Verification

- `cargo test admin_gateway --lib`
- `cargo test admin_project --lib`
- `go test -count=1 ./...` in `sdk/go`
- `go test -count=1 -tags easynet_cabi ./...` in `sdk/go`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_cabi.py`
- `cargo fmt --check`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`
- Retired address terminology scan over touched files.

# Go Events Runtime Projection Provider

## Goal

Implement Go Events runtime directory-event, drop-report, and terminal projection through an explicit daemon-owned projection provider while preserving Runtime Core stream ownership.

## Boundary Proof

- `EventsRuntimeTransport` continues to own Runtime Core Invocation lowering and stream-handle ownership for subscriptions.
- Event frame projection remains daemon-owned and is supplied by an explicit provider.
- The C ABI Events transport already implements the provider shape and remains the Rust-owned projection path.
- Go runtime transport validates provider output against the existing `EventFrame` DTO before returning it.
- No backend subscriber registry, browser delivery policy, or facade-side live fan-out is introduced.

## Invariants

- Runtime Events projection methods fail closed without a configured provider.
- Provider output must decode as the public `EventFrame` DTO.
- Existing request JSON and public client methods remain unchanged.
- Runtime stream and device-history behavior remain untouched.
- No retired address terminology is introduced in touched files.

## Verification

- `go test -count=1 -run 'TestRuntimeEvents' ./...` from `sdk/go` - passed.
- `go test -count=1 ./...` from `sdk/go` - passed.
- `go test -count=1 -tags easynet_cabi ./...` from `sdk/go` - passed.
- `bash tools/scripts/check-sdk-scaffold.sh` - passed.
- `git diff --check` - passed.

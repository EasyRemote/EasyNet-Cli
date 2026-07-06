# Verification

Run:

1. `go test ./...` from `sdk/go`
2. `bash tools/scripts/check-sdk-ura-naming.sh`
3. `bash tools/scripts/check-sdk-scaffold.sh`
4. `git diff --check`

Acceptance:

1. Public Go SDK no longer aliases Axon bridge DTO types.
2. Invoke-remote marshal/unmarshal/decode still delegates to Axon.
3. Authority raw metadata canonicalization still delegates to Axon.
4. Constants remain delegated to Axon rather than copied locally.
5. Legacy `ability` input is not emitted by the facade.

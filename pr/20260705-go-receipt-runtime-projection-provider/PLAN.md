# Go Receipt Runtime Projection Provider

## Goal

Implement Go Receipt runtime Project, Verify, VerifyChain, and CausalRef through an explicit daemon/Axon projection provider while preserving Runtime Core invocation ownership for fetch/history reads.

## Boundary Proof

- `ReceiptRuntimeTransport` continues to own Runtime Core invocation lowering for fetch and invocation-ledger history reads.
- Receipt projection, conservative verification, chain continuity, and causal-ref construction remain daemon/Axon-owned and are supplied by an explicit provider.
- The C ABI Receipt transport already implements the provider shape and remains the Rust-owned projection path.
- Go runtime transport validates provider output against existing Receipt DTO constructors before returning it.
- No cryptographic verification algorithm, ledger parser, backend receipt database, or product audit policy is introduced in Go.

## Invariants

- Runtime receipt Project, Verify, VerifyChain, and CausalRef fail closed without a configured provider.
- Provider output must decode as the public Receipt DTO for the called operation.
- Existing request JSON and public client methods remain unchanged.
- Fetch/history Runtime invocation behavior remains untouched.
- No retired address terminology is introduced in touched files.

## Verification

- `go test -count=1 ./...` in `sdk/go`.
- `go test -count=1 -tags easynet_cabi ./...` in `sdk/go`.
- `cargo fmt --check`.
- `bash tools/scripts/check-sdk-scaffold.sh`.
- `git diff --check`.
- Retired address terminology scan over touched files.

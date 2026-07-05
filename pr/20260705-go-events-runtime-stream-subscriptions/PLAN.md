# Go Events Runtime Stream Subscriptions

## Goal

Implement Go Events runtime subscriptions as real Runtime Core stream openings while preserving the existing Events DTO shape.

## Boundary Proof

- Events subscription carrier construction remains in `EventsRuntimeTransport`.
- Runtime stream ownership remains in Runtime Core via `RuntimeClient.InvokeStream`.
- `EventStream` gains private handle ownership and public observation methods without changing its JSON fields.
- C ABI Events subscription methods remain explicitly unsupported because they do not own stream opening.
- Hub, pairing, and daemon lifecycle policy are out of scope.

## Invariants

- Subscription requests still build complete Invocation drafts with caller, callee, descriptor, subject, nonce, causal context, and args.
- The returned Events object exposes state and stream id, and does not hide stream resource lifetime.
- Stream event ordering, terminal state, cancel, and close semantics are delegated to `StreamHandle`.
- No retired address terminology is introduced in touched files.

## Verification

- `go test -count=1 ./...` in `sdk/go`.
- `go test -count=1 -tags easynet_cabi ./...` in `sdk/go`.
- `git diff --check`.
- Retired address terminology scan over touched files.

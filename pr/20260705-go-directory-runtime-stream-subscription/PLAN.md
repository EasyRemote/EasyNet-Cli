# Go Directory Runtime Stream Subscription

## Goal

Implement Go Directory runtime subscriptions as real Runtime Core streams while preserving the existing Directory subscription JSON projection.

## Boundary Proof

- Directory carrier construction remains in `DirectoryRuntimeTransport`.
- Runtime stream opening and terminal behavior remain owned by `RuntimeClient.InvokeStream` and `StreamHandle`.
- `DirectorySubscription` stays JSON-compatible and gains private runtime handle ownership plus public observation methods.
- C ABI Directory subscription behavior is out of scope for this slice.
- No daemon lifecycle, Hub policy, backend fan-out, or product subscriber registry behavior is introduced.

## Invariants

- Subscription requests still lower to a complete Invocation draft before dispatch.
- Snapshot/live ordering and cursor monotonicity continue to use `DirectorySubscription` validation.
- Each runtime-opened subscription has explicit `Next`, `Cancel`, and `Close` operations.
- Runtime stream handles are released from the transport registry on successful cancel or close.
- No retired address terminology is introduced in touched files.

## Verification

- `go test -count=1 ./...` in `sdk/go`.
- `go test -count=1 -tags easynet_cabi ./...` in `sdk/go`.
- `cargo fmt --check`.
- `bash tools/scripts/check-sdk-scaffold.sh`.
- `git diff --check`.
- Retired address terminology scan over touched files.

# Authority Conformance Plan

## Goal

Promote the SDK authority metadata facade from local unit coverage to a shared
Go/Python conformance gate without moving authority canonicalization, signing,
or admission into language facades.

## Boundary Proof

- Go/Python tests consume one shared metadata fixture and one shared case file.
- The shared case asserts projection and mutual exclusion only.
- Authority metadata values remain daemon/Axon opaque wire values.
- Minting/signing/verification remains a future daemon/Axon-owned transport
  slice, not part of this conformance slice.

## Invariants

1. Both languages decode the same delegated-authority and session-authority
   metadata values into typed SDK projections.
2. Both languages preserve existing Invocation metadata when attaching one
   authority value.
3. Both languages reject an Invocation draft that carries both delegated and
   session authority metadata.
4. The SDK parity matrix must fail if this case is missing from the
   `complete_invocation_draft` capability or from action-adapter reports.

## Verification

- `go test ./...` in `sdk/go`.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`.
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`.
- `tools/scripts/check-sdk-scaffold.sh`.
- Product boundary scans remain separate from this SDK conformance slice.

## Remaining After This Slice

- Daemon/Axon-owned authority minting transport.
- EasyNet backend raw Axon/direct daemon transport cutover.
- RFC-007 receipt URA construction.
- Events filtering/live adapters and trust/certificate policy.

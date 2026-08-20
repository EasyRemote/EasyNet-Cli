# SDK Cutover Readiness Gate

## Objective

Add a single EasyNet-Cli gate that audits whether the daemon SDK can claim
consumer cutover readiness across the in-repo SDK evidence plus the sibling
EasyRemote and EasyNet backend repositories. The gate must make remaining
external product blockers reproducible without moving product-owned behavior
into the SDK.

## Boundary Proof

- EasyNet-Cli SDK owns facade parity, conformance fixtures, and static boundary
  gates for consumers.
- EasyRemote and EasyNet backend own their product facades and route migration.
- The new gate only orchestrates existing evidence and reports failures; it does
  not weaken backend/EasyRemote restrictions and does not mark any capability
  `cutover-ready` by itself.

## Invariants

- Passing the aggregate gate requires Go/Python parity, daemon Invocation
  migration cleanliness, EasyRemote raw-lower-layer bans, backend SDK-only bans,
  and backend route-family coverage manifest validity.
- Failing the aggregate gate must preserve the underlying script's exit code
  class and output enough context to fix the product boundary violation.
- Backend root resolution must accept both the backend module root and the
  EasyNet monorepo root containing `backend/go.mod`.
- Self-tests must not require the sibling product repositories.
- The gate must not modify files outside EasyNet-Cli.

## Implementation Plan

1. Refactor backend SDK-only boundary root resolution into explicit module-root
   discovery.
2. Add a top-level `check-sdk-cutover-readiness.sh` script that composes SDK,
   daemon, EasyRemote, backend import, and backend route-family gates.
3. Add deterministic self-tests for root discovery and aggregate failure
   reporting.
4. Register the script in SDK scaffold checks.
5. Run targeted self-tests, scaffold, parity, and diff hygiene before commit.

## Remaining Outside This Slice

- Actual EasyNet backend raw Axon/protobuf/direct daemon transport migration.
- Backend route smoke tests against live product handlers.
- Product-owned SSE/WebSocket/storage/quota/billing/certificate policy cutovers.
- RFC-007 receipt URA construction.

## Verification Results

- `bash tools/scripts/check-backend-sdk-only-boundary.sh --self-test` passed.
- `bash tools/scripts/check-backend-route-family-coverage.sh --self-test` passed.
- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test` passed.
- `bash tools/scripts/check-sdk-cutover-readiness.sh` failed as expected on the
  current sibling EasyNet backend repository because it still contains raw Axon
  imports, generated `internal/pb/axon/v1` packages/imports, and direct
  `internal/daemon_grpc` transport usage. The same aggregate run showed SDK
  scaffold, SDK parity matrix, daemon Invocation migration, EasyRemote boundary,
  and backend route-family coverage passing.

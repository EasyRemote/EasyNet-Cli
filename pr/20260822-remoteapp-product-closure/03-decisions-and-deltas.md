# Decisions and Deltas — RemoteApp Product Closure

## 2026-08-22 — User Service projection conflict must not kill Device session

Decision:

- `service/<user>.pages` remains a user-scoped Service owner projection.
- The Hub read model currently selects one live projection row per
  `owner_ura`.
- Equal generation/revision projection conflicts are read-model selection
  outcomes, not authority failures.
- Device-native RemoteApp abilities remain SystemAgent-owned and must not be
  taken offline by a non-selected user Service projection.

Implementation delta:

- `federation.advertise_abilities` responses now carry an optional projection
  upsert `outcome`.
- Strict projection callers still require `ack=true` and exact `count`.
- User-scoped Service owner prelude degrades only when the admitted write is a
  read-model rejection such as `ignored_stale` or `rejected_conflict`.
- Admission, signer delegation, descriptor integrity, transport errors, and
  acknowledged count mismatches still fail closed.

Product effect:

- Cross-device RemoteApp smoke should no longer report the caller Device as
  offline merely because another host already owns the selected Pages Service
  projection.
- This does not claim product completion for real OS capture, input injection,
  audio/video, NAT/relay, or frontend end-to-end RemoteApp lifecycle.

## 2026-08-22 — Cross-device smoke must produce bounded environment evidence

Decision:

- Cross-device RemoteApp evidence must be terminal and inspectable even when
  the local Docker or filesystem environment is not ready.
- A Docker probe hang and insufficient report filesystem space are environment
  failures, not RemoteApp feature failures.

Implementation delta:

- The cross-device smoke now checks report filesystem free space before child
  E2Es.
- The Docker readiness probe uses a bounded `docker info` timeout.
- Each child E2E step runs under a bounded timeout and writes a failed step
  report on timeout/failure.

Product effect:

- Future `--run` attempts will either produce cross-device product evidence or
  a structured failed report explaining why the environment could not execute
  the product path.
- This closes an evidence-chain seam only; it does not complete real capture,
  input, audio/video, relay, or frontend lifecycle coverage.

## 2026-08-22 — Product closure state must be machine-readable

Decision:

- RemoteApp product completion cannot be inferred from Markdown prose, local
  source gates, or synthetic reports.
- The eight product requirements are now represented as an explicit readiness
  matrix with row ids, current evidence, required evidence, and non-claims.
- The matrix is allowed to contain only `partial` or `incomplete` rows until
  real authoritative product evidence exists for every requirement.

Implementation delta:

- Added `docs/design/remoteapp-product-readiness-matrix.json`.
- Extended `check-remoteapp-product-closure-audit.sh` to reject missing matrix
  rows, empty evidence, unsupported statuses, and premature
  `product_complete=true`.
- Extended the audit script tests to mutate the matrix and prove those failure
  modes are caught.

Product effect:

- Future RemoteApp work has a concrete acceptance ledger for real OS capture,
  input, audio/video, multi-window tracking, recovery, network fallback,
  frontend lifecycle, and cross-device E2E.
- This does not implement the missing product capabilities; it prevents
  architectural drift and false product-complete claims while those
  capabilities are built.

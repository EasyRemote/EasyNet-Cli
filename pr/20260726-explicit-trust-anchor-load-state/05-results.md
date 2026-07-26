# Results

Implemented explicit realm trust-anchor load state.

## Refactoring

- Removed the `RealmTrustAnchor::load_or_empty` compatibility helper.
- Added `RealmTrustAnchorLoadState::{Loaded, Missing}`.
- Moved missing-file policy decisions to explicit callers:
  - daemon first boot logs `realm_trust_anchor_missing_first_run` and starts
    with an empty in-memory anchor;
  - daemon reload fails closed and preserves the current shared trust cell;
  - CLI read projections render missing trust storage as empty display state;
  - Mission remote-child receipt verification fails when trust storage is
    missing;
  - canonical receipt resolver preserves missing, empty, loaded, and malformed
    trust-source states separately.

## Tests

- `cargo test daemon::trust::anchor::tests --lib`
- `cargo test daemon::boot::invocation::trust::tests --lib`
- `cargo test support::platform::local_daemon_grpc::tests::canonical_receipt_resolver_preserves --lib --features axon-pb`
- `cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

## Gate

Added `check_realm_trust_anchor_explicit_load_state_contract` to SPEC v2 so the
old empty-fallback helper and missing-file-as-empty test cannot return.

# Verification

Completed:

- `cargo fmt --check`
- `cargo test canonical_receipt_resolver_preserves_malformed_realm_trust_source --lib`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph explore --max-files 3 -p . RealmReceiptTrustSource CanonicalRuntimeReceiptResolver unavailable_detail`
- `rg -n "realm trust anchor is empty or unavailable|realm_trust: Option<crate::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver>|\\.ok\\(\\)\\s*\\.filter\\(\\|anchor\\| !anchor\\.is_empty\\(\\)\\)|RealmReceiptTrustSource|canonical_receipt_resolver_preserves_malformed_realm_trust_source" src/support/platform/local_daemon_grpc.rs tools/scripts/check-canonical-runtime-convergence-v2.sh`

Evidence:

- The canonical receipt resolver now stores `RealmReceiptTrustSource` instead of an optional resolver.
- Malformed realm trust anchors are surfaced as `LoadFailed` details during signer resolution.
- The retired `empty or unavailable` trust-source message appears only in the SPEC self-test fixture and forbidden-token list.
- SPEC v2 and legacy architecture gates pass.

# Verification

- `cargo fmt --check`
- `git diff --check`
- `cargo test realm_from_ura --features axon-pb`
- `cargo test federated_user_resolver --features axon-pb`
- `cargo test resolved_device_record_keeps_cross_tenant_realm_and_abilities --features axon-pb`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph index .`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "parse_realm_from_ura" --limit 20`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "parse_realm_from_user_ura" --limit 20`
- `rg -n "parse_realm_from_ura|parse_realm_from_user_ura|duplicated rather than re-exported|federated fallback|register_device_pubkey::parse_realm_from_ura" src -S || true`

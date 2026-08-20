#!/usr/bin/env bash
# standalone-hub-recovery-e2e.sh — live Hub PrincipalLifecycle recovery edges

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

if [[ "${1:-}" == "--self-test" ]]; then
  bash -n "$0"
  test -f "$REPO_ROOT/tests/hub_ura_tls_join_cli_e2e.rs"
  grep -q "tls-alice-recover-replay" "$REPO_ROOT/tests/hub_ura_tls_join_cli_e2e.rs"
  grep -q "tls-bob-recover-deleted" "$REPO_ROOT/tests/hub_ura_tls_join_cli_e2e.rs"
  grep -q "tls-admin-wrong-delete-grant" "$REPO_ROOT/tests/hub_ura_tls_join_cli_e2e.rs"
  grep -q "tls-bob-delete-wrong-grant" "$REPO_ROOT/tests/hub_ura_tls_join_cli_e2e.rs"
  grep -q "CARGO_BIN_EXE_easynet-keyring" "$REPO_ROOT/tests/hub_ura_tls_join_cli_e2e.rs"
  grep -q "replayed recovery key must not be projected into RuntimeTrust" "$REPO_ROOT/tests/hub_ura_tls_join_cli_e2e.rs"
  grep -q "deleted-principal recovery key must not be projected into RuntimeTrust" "$REPO_ROOT/tests/hub_ura_tls_join_cli_e2e.rs"
  echo "standalone-hub-recovery-e2e self-test ok"
  exit 0
fi

echo "[standalone-hub-recovery-e2e] running live Hub TCP+TLS recovery edge E2E..."
(
  cd "$REPO_ROOT"
  cargo test --features axon-pb --test hub_ura_tls_join_cli_e2e -- --nocapture
)

echo "[standalone-hub-recovery-e2e] PASS"

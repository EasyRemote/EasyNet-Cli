#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-kernel-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

run_script() {
  (
    cd "$SB"
    bash tools/scripts/check-kernel-boundary.sh
  ) >/tmp/check-kernel-boundary.out 2>&1
}

expect_fail() {
  local label="$1"
  set +e
  run_script
  local rc=$?
  set -e
  [[ "$rc" == "1" ]] || {
    cat /tmp/check-kernel-boundary.out >&2 || true
    fail "$label should exit 1 (got $rc)"
  }
}

expect_pass() {
  local label="$1"
  run_script || {
    cat /tmp/check-kernel-boundary.out >&2 || true
    fail "$label should pass"
  }
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p \
  "$SB/tools/scripts" \
  "$SB/src/daemon/control" \
  "$SB/src/daemon/execution" \
  "$SB/src/daemon/federation" \
  "$SB/src/daemon/invocation"
cp "$SCRIPT" "$SB/tools/scripts/check-kernel-boundary.sh"

cat > "$SB/src/daemon/control/mod.rs" <<'EOF'
pub fn control_plane() {}
EOF
cat > "$SB/src/daemon/invocation/mod.rs" <<'EOF'
pub fn invocation_plane() {}
EOF
cat > "$SB/src/daemon/execution/mod.rs" <<'EOF'
pub fn run() {}
EOF

expect_pass "clean final layout"

mkdir -p "$SB/src/runtime"
expect_fail "retired src/runtime root"
rm -rf "$SB/src/runtime"

cat > "$SB/src/daemon/invocation/mod.rs" <<'EOF'
use crate::runtime::session::Session;

pub fn invocation_plane(_: Session) {}
EOF
expect_fail "retired crate::runtime namespace"
cat > "$SB/src/daemon/invocation/mod.rs" <<'EOF'
pub fn invocation_plane() {}
EOF

cat > "$SB/src/daemon/control/mod.rs" <<'EOF'
use crate::ffi::client::FfiClient;

pub fn control_plane(_: FfiClient) {}
EOF
expect_fail "daemon control importing ffi edge"
cat > "$SB/src/daemon/control/mod.rs" <<'EOF'
pub fn control_plane() {}
EOF

cat > "$SB/src/daemon/execution/mod.rs" <<'EOF'
use crate::daemon::federation::client::FederationClient;

pub fn run(_client: FederationClient) {}
EOF
expect_fail "execution importing concrete gateway"
cat > "$SB/src/daemon/execution/mod.rs" <<'EOF'
pub fn run() {}
EOF

mkdir -p "$SB/src/daemon/kernel"
expect_fail "retired daemon kernel root"
rm -rf "$SB/src/daemon/kernel"

expect_pass "final layout after cleanup"

echo "test_check_kernel_boundary.sh: all cases passed"

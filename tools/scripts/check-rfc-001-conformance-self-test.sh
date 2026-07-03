#!/usr/bin/env bash
# Self-test for check-rfc-001-conformance.sh (EasyNet-Cli).
#
# Builds a temporary fixture tree mimicking the CLI src/ layout,
# injects known-violation strings, runs the conformance script
# against the fixture (via RFC001_FIXTURE_ROOT), and asserts catches.
#
# Run: ./check-rfc-001-conformance-self-test.sh
# Exit 0 = all assertions pass.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CONFORMANCE_SCRIPT="$SCRIPT_DIR/check-rfc-001-conformance.sh"

if [[ ! -x "$CONFORMANCE_SCRIPT" ]]; then
  echo "FAIL: conformance script not executable: $CONFORMANCE_SCRIPT"
  exit 1
fi

FIXTURE_ROOT="$(mktemp -d -t rfc001-cli-XXXXXX)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

assertions_run=0
assertions_failed=0

assert() {
  local label="$1"
  local actual="$2"
  local expected="$3"
  assertions_run=$((assertions_run + 1))
  if [[ "$actual" == "$expected" ]]; then
    printf "  [PASS] %-60s\n" "$label"
  else
    printf "  [FAIL] %-60s expected=%s actual=%s\n" "$label" "$expected" "$actual"
    assertions_failed=$((assertions_failed + 1))
  fi
}

run_conformance() {
  local phase="$1"
  RFC001_PHASE="$phase" RFC001_FIXTURE_ROOT="$FIXTURE_ROOT" \
    "$CONFORMANCE_SCRIPT" 2>&1
}

count_violations() {
  echo "$1" | grep -E "^Total flagged occurrences:" | awk '{print $4}'
}

exit_code() {
  local phase="$1"
  set +e
  RFC001_PHASE="$phase" RFC001_FIXTURE_ROOT="$FIXTURE_ROOT" \
    "$CONFORMANCE_SCRIPT" >/dev/null 2>&1
  local rc=$?
  set -e
  echo "$rc"
}

# ──────────────────────────────────────────────────────────
# Case 1: clean fixture, no system.* / no MCP / no register-tool.
# ──────────────────────────────────────────────────────────
echo "Case 1: clean fixture"
mkdir -p "$FIXTURE_ROOT/src/daemon/invocation"
mkdir -p "$FIXTURE_ROOT/src/daemon/federation"
cat > "$FIXTURE_ROOT/src/daemon/invocation/clean.rs" << 'EOF'
pub fn clean() {}
EOF

output="$(run_conformance baseline)"
total="$(count_violations "$output")"
assert "Case 1 — baseline counts 0 violations" "$total" "0"
assert "Case 1 — baseline exit code = 0"      "$(exit_code baseline)" "0"
assert "Case 1 — enforce exit code = 0"       "$(exit_code enforce)"  "0"

# ──────────────────────────────────────────────────────────
# Case 2: register_runtime_local_mcp_tool — caught.
# ──────────────────────────────────────────────────────────
echo
echo "Case 2: inject register_runtime_local_mcp_tool"
cat > "$FIXTURE_ROOT/src/daemon/federation/publish.rs" << 'EOF'
pub fn publish() {
    register_runtime_local_mcp_tool();
}
EOF

output="$(run_conformance baseline)"
total="$(count_violations "$output")"
# Should catch register_runtime_local_mcp_tool AND the MCP keyword
# (since 'mcp' appears in both the function name and as a word).
if [[ "$total" -lt 1 ]]; then
  printf "  [FAIL] %-60s expected >=1\n" "Case 2 — at least 1 violation"
  assertions_failed=$((assertions_failed + 1))
else
  printf "  [PASS] %-60s\n" "Case 2 — at least 1 violation total ($total)"
fi
assertions_run=$((assertions_run + 1))
assert "Case 2 — enforce exit code = 1" "$(exit_code enforce)" "1"

# ──────────────────────────────────────────────────────────
# Case 3: system.* ability name — caught.
# ──────────────────────────────────────────────────────────
echo
echo "Case 3: inject system.skill.list literal"
cat > "$FIXTURE_ROOT/src/daemon/invocation/skill_list.rs" << 'EOF'
const SKILL_LIST_ABILITY: &str = "system.skill.list";
EOF

output="$(run_conformance baseline)"
if echo "$output" | grep -q "WARN.*system\.skill\.\* / system\.session"; then
  printf "  [PASS] %-60s\n" "Case 3 — system.* rule fired"
else
  printf "  [FAIL] %-60s\n" "Case 3 — system.* rule did not fire"
  assertions_failed=$((assertions_failed + 1))
fi
assertions_run=$((assertions_run + 1))

# ──────────────────────────────────────────────────────────
# Case 4: MCP keyword in non-edge file — caught.
# ──────────────────────────────────────────────────────────
echo
echo "Case 4: MCP outside mcp-profile module is flagged"
cat > "$FIXTURE_ROOT/src/daemon/invocation/mcp_in_dispatch.rs" << 'EOF'
// MCP-aware kernel code — should be flagged.
fn dispatch_mcp() {}
EOF

output="$(run_conformance baseline)"
if echo "$output" | grep -q "WARN.*MCP keyword in CLI src"; then
  printf "  [PASS] %-60s\n" "Case 4 — MCP-in-non-edge rule fired"
else
  printf "  [FAIL] %-60s\n" "Case 4 — MCP-in-non-edge rule did not fire"
  assertions_failed=$((assertions_failed + 1))
fi
assertions_run=$((assertions_run + 1))

# ──────────────────────────────────────────────────────────
# Case 5: MCP inside mcp-profile module — ALLOWED (excluded).
# ──────────────────────────────────────────────────────────
echo
echo "Case 5: MCP inside src/daemon/ability/catalog/profiles/mcp.rs is allowed"
mkdir -p "$FIXTURE_ROOT/src/daemon/ability/catalog/profiles"
# Remove the violating file from Case 4 first.
rm "$FIXTURE_ROOT/src/daemon/invocation/mcp_in_dispatch.rs"
# Add a file under the allowed path.
cat > "$FIXTURE_ROOT/src/daemon/ability/catalog/profiles/mcp.rs" << 'EOF'
// mcp-profile Agent implementation — MCP keyword expected here.
fn handle_mcp_request() {}
EOF

output="$(run_conformance baseline)"
# The rule "MCP keyword in CLI src" excludes daemon/ability/catalog/profiles/mcp.rs.
# But we still have system.skill.list from Case 3 + register_* from Case 2.
# Verify the MCP rule itself reports 0.
mcp_line="$(echo "$output" | grep "MCP keyword in CLI src" || true)"
if echo "$mcp_line" | grep -qF "[ ok ]"; then
  printf "  [PASS] %-60s\n" "Case 5 — MCP allowed inside profile projection"
else
  printf "  [FAIL] %-60s\n" "Case 5 — MCP rule should be ok; line: $mcp_line"
  assertions_failed=$((assertions_failed + 1))
fi
assertions_run=$((assertions_run + 1))

# ──────────────────────────────────────────────────────────
# Case 6: REMOVED-RFC-001 file excluded.
# ──────────────────────────────────────────────────────────
echo
echo "Case 6: REMOVED-RFC-001 marker excluded from scan"
before="$(count_violations "$(run_conformance baseline)")"
cat > "$FIXTURE_ROOT/src/daemon/invocation/REMOVED-RFC-001-archive.rs" << 'EOF'
// Historical: register_runtime_local_mcp_tool, system.skill.list, MCP, etc.
EOF
after="$(count_violations "$(run_conformance baseline)")"
assert "Case 6 — REMOVED-RFC-001 file excluded" "$after" "$before"

# ──────────────────────────────────────────────────────────
# Case 7: final-forbidden source root presence detected.
# ──────────────────────────────────────────────────────────
echo
echo "Case 7: final-forbidden src/facade/ root presence"
mkdir -p "$FIXTURE_ROOT/src/facade/mcp"
output="$(run_conformance baseline)"
if echo "$output" | grep -qE "WARN.*src/facade/ final-forbidden source root.*exists"; then
  printf "  [PASS] %-60s\n" "Case 7 — final-forbidden src/facade/ detected"
else
  printf "  [FAIL] %-60s\n" "Case 7 — final-forbidden src/facade/ not detected"
  assertions_failed=$((assertions_failed + 1))
fi
assertions_run=$((assertions_run + 1))
if [[ "$(exit_code enforce)" == "1" ]]; then
  printf "  [PASS] %-60s\n" "Case 7 — enforce mode rejects final-forbidden root"
else
  printf "  [FAIL] %-60s\n" "Case 7 — enforce mode did not reject final-forbidden root"
  assertions_failed=$((assertions_failed + 1))
fi
assertions_run=$((assertions_run + 1))

# ──────────────────────────────────────────────────────────
# Summary
# ──────────────────────────────────────────────────────────
echo
echo "==================================================================="
echo "Self-test: $assertions_run assertions, $assertions_failed failed"
if [[ "$assertions_failed" -gt 0 ]]; then
  exit 1
fi
echo "All assertions passed."
exit 0

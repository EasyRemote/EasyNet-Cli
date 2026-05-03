#!/usr/bin/env bash
# tests/pages_cli.sh — Matrix B (CLI surface) for the Pages reference system.
#
# Runs the prebuilt `easynet pages …` binary against a running daemon
# and asserts each subcommand's exit code + stdout shape. This catches
# CLI-shape regressions that the in-process pages_unit.rs cannot
# (clap arg parsing, JSON-vs-human output, exit codes).
#
# Pre-conditions:
#   - daemon running with EASYNET_PAGES_PORT=8787 and EASYNET_PAGES_USER
#   - `easynet` binary built at target/debug/easynet
#   - /tmp/easynet-pages-cli-fixture/{hello-world.html,style.css} present
#     (created by this script)
#
# Conformance: RFC-006-B v0.6 §6.3 Matrix B (C1-C10).
#
# Author: Silan Hu <silan.hu@u.nus.edu>

set -uo pipefail

EASYNET="${EASYNET:-/Users/macbook.silan.tech/Documents/Github/EasyNet-Cli/target/debug/easynet}"
USER_ID="${EASYNET_PAGES_USER:-alice-cli}"
FIXTURE="/tmp/easynet-pages-cli-fixture"
PROJECT="paperscli"

export EASYNET_PAGES_USER="$USER_ID"

PASS=0
FAIL=0

# Per-case helper: print pass / fail with reason.
ok() { echo "  [PASS] $1"; PASS=$((PASS+1)); }
ko() { echo "  [FAIL] $1: $2"; FAIL=$((FAIL+1)); }

setup() {
  rm -rf "$FIXTURE"
  mkdir -p "$FIXTURE"
  cat > "$FIXTURE/hello-world.html" <<'EOF'
<!doctype html>
<html><head><link rel="stylesheet" href="style.css"></head>
<body><h1>CLI test</h1></body></html>
EOF
  echo 'h1 { color: red; }' > "$FIXTURE/style.css"
  # ensure clean slate
  "$EASYNET" pages delete "$PROJECT" --force >/dev/null 2>&1 || true
}

teardown() {
  "$EASYNET" pages delete "$PROJECT" --force >/dev/null 2>&1 || true
  rm -rf "$FIXTURE"
}

# C1 — create exits 0 + stdout has project_uri + url_root
c1_create() {
  local out
  out=$("$EASYNET" pages create "$PROJECT" --folder "$FIXTURE" 2>&1)
  local rc=$?
  if [[ $rc -ne 0 ]]; then ko C1 "exit=$rc out=$out"; return; fi
  if [[ "$out" == *"project_uri"* && "$out" == *"url_root"* ]]; then
    ok "C1 create"
  else
    ko C1 "missing project_uri or url_root in stdout: $out"
  fi
}

# C2 — create --json emits valid JSON
c2_create_json() {
  # already published from C1; recreate
  "$EASYNET" pages delete "$PROJECT" --force >/dev/null 2>&1 || true
  local out
  out=$("$EASYNET" pages create "$PROJECT" --folder "$FIXTURE" --json 2>&1)
  if [[ $? -ne 0 ]]; then ko C2 "exit nonzero: $out"; return; fi
  if echo "$out" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert "project_uri" in d and "url_root" in d' 2>/dev/null; then
    ok "C2 create --json"
  else
    ko C2 "stdout is not valid JSON with required keys: $out"
  fi
}

# C3 — list shows the row
c3_list() {
  local out
  out=$("$EASYNET" pages list 2>&1)
  if [[ "$out" == *"$PROJECT"* ]]; then
    ok "C3 list"
  else
    ko C3 "no '$PROJECT' in list output: $out"
  fi
}

# C4 — list --json
c4_list_json() {
  local out
  out=$("$EASYNET" pages list --json 2>&1)
  if echo "$out" | python3 -c 'import json,sys; d=json.load(sys.stdin); assert isinstance(d.get("projects"), list)' 2>/dev/null; then
    ok "C4 list --json"
  else
    ko C4 "stdout is not valid JSON: $out"
  fi
}

# C5 — show prints detail
c5_show() {
  local out
  out=$("$EASYNET" pages show "$PROJECT" 2>&1)
  if [[ "$out" == *"project_uri"* && "$out" == *"$PROJECT"* ]]; then
    ok "C5 show"
  else
    ko C5 "show stdout missing detail: $out"
  fi
}

# C6 — url prints exactly the URL + newline
c6_url() {
  local out
  out=$("$EASYNET" pages url "$PROJECT" 2>&1)
  if [[ "$out" == http://*"$PROJECT"*"$USER_ID"*"pages"* ]]; then
    ok "C6 url"
  else
    ko C6 "url stdout wrong: $out"
  fi
}

# C7 — delete without --force fails
c7_delete_no_force() {
  local out
  out=$("$EASYNET" pages delete "$PROJECT" 2>&1)
  local rc=$?
  if [[ $rc -ne 0 && "$out" == *"force"* ]]; then
    ok "C7 delete refuses without --force"
  else
    ko C7 "delete should have failed without --force: rc=$rc out=$out"
  fi
}

# C8 — delete --force
c8_delete_force() {
  local out
  out=$("$EASYNET" pages delete "$PROJECT" --force 2>&1)
  local rc=$?
  if [[ $rc -eq 0 && "$out" == *"Unpublished"* ]]; then
    ok "C8 delete --force"
  else
    ko C8 "delete --force failed: rc=$rc out=$out"
  fi
}

# C9 — create with non-existent folder fails clearly
c9_missing_folder() {
  local out
  out=$("$EASYNET" pages create badpath --folder /tmp/easynet-does-not-exist-$$ 2>&1)
  local rc=$?
  if [[ $rc -ne 0 && "$out" == *"does not exist"* ]]; then
    ok "C9 missing folder rejected"
  else
    ko C9 "expected rejection: rc=$rc out=$out"
  fi
}

# C10 — visibility=private rejected with clear message
c10_private_visibility() {
  local out
  out=$("$EASYNET" pages create "$PROJECT" --folder "$FIXTURE" --visibility private 2>&1)
  local rc=$?
  if [[ $rc -ne 0 && "$out" == *"private"* && "$out" == *"not yet supported"* ]]; then
    ok "C10 visibility=private rejected"
  else
    ko C10 "expected rejection: rc=$rc out=$out"
  fi
}

main() {
  echo "=== Matrix B — CLI surface ==="
  setup
  c1_create
  c2_create_json
  c3_list
  c4_list_json
  c5_show
  c6_url
  c7_delete_no_force
  c8_delete_force
  c9_missing_folder
  c10_private_visibility
  teardown
  echo ""
  echo "=== summary: $PASS passed, $FAIL failed ==="
  [[ $FAIL -eq 0 ]]
}

main

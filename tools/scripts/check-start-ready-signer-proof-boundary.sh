#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

START="src/cli/commands/start.rs"
WATCHER="src/cli/commands/start_boot_watcher.rs"
BOOT_INVOCATION="src/daemon/boot/invocation/mod.rs"

[[ -f "$START" ]] || fail "missing $START"
[[ -f "$WATCHER" ]] || fail "missing $WATCHER"
[[ -f "$BOOT_INVOCATION" ]] || fail "missing $BOOT_INVOCATION"

if ! rg -n 'ready_capability_flags: Vec<String>' "$WATCHER" >/dev/null; then
  fail "BootProgressOutcome must carry daemon Ready capability flags"
fi

if ! rg -n 'has_ready_capability_flag' "$WATCHER" >/dev/null; then
  fail "BootProgressOutcome must expose typed ready capability lookup"
fi

if ! rg -n 'outcome\.ready_capability_flags = disc\.capability_flags\.clone\(\)' "$WATCHER" >/dev/null; then
  fail "start boot watcher must capture capability flags from control discovery on Ready"
fi

if ! rg -n 'fn validate_device_runtime_readiness' "$START" >/dev/null; then
  fail "device start must validate daemon Ready signer capability and caller signer custody before success"
fi

if ! rg -n 'PAIRED_USER_RUNTIME_SIGNER' "$START" >/dev/null; then
  fail "device start must require paired_user_runtime_signer readiness"
fi

if ! rg -n 'RuntimeCallerSignerReadinessProbe' "$START" >/dev/null; then
  fail "device start must route signer readiness through an explicit probe boundary"
fi

if ! rg -n 'prove_runtime_caller_signer_custody' "$START" >/dev/null; then
  fail "device start must prove active caller signer custody, not only inspect Ready flags"
fi

if rg -n 'fn validate_device_ready_capabilities' "$START" >/dev/null; then
  fail "retired device-ready flag-only validator is still present"
fi

if ! rg -n 'fn register_paired_user_runtime_signer_if_bound' "$BOOT_INVOCATION" >/dev/null; then
  fail "daemon boot must centralize paired User runtime signer readiness"
fi

if ! rg -n 'ensure_user_runtime_signing_identity' "$BOOT_INVOCATION" >/dev/null; then
  fail "daemon boot must ensure paired User managed signing identity"
fi

if ! rg -n 'prove_user_runtime_signing_projection_custody' "$BOOT_INVOCATION" >/dev/null; then
  fail "daemon boot must prove projection-bound paired User signer custody before Ready"
fi

python3 - "$BOOT_INVOCATION" <<'PY'
import pathlib
import sys

text = pathlib.Path(sys.argv[1]).read_text()
fn = text.find("fn register_paired_user_runtime_signer_if_bound")
if fn == -1:
    raise SystemExit("missing register_paired_user_runtime_signer_if_bound")
body = text[fn:text.find("\nfn ", fn + 1) if text.find("\nfn ", fn + 1) != -1 else len(text)]
ensure = body.find("ensure_user_runtime_signing_identity")
prove = body.find("prove_user_runtime_signing_projection_custody")
register = body.find(".register_user_pubkey(")
flag = text.find("PAIRED_USER_RUNTIME_SIGNER")
call = text.find("register_paired_user_runtime_signer_if_bound(")
if ensure == -1 or prove == -1 or register == -1:
    raise SystemExit("paired User signer boot gate must ensure, prove, and register")
if not (ensure < prove < register):
    raise SystemExit("paired User signer custody proof must bind the ensured projection before trust registration")
if call == -1 or flag == -1 or not (call < flag):
    raise SystemExit("daemon boot must register/prove paired User signer before publishing Ready flag")
PY

python3 - "$START" <<'PY'
import pathlib
import sys

text = pathlib.Path(sys.argv[1]).read_text()
validate = text.find("validate_device_runtime_readiness(&boot, &creds)")
save = text.find("save_runtime_projection_after_ready(&mut daemon_handle")
welcome = text.find('console::style("Welcome,")')
stop = text.find("daemon_handle.stop()")

if validate == -1:
    raise SystemExit("missing validate_device_runtime_readiness call")
if save == -1:
    raise SystemExit("missing save_runtime_projection_after_ready")
if welcome == -1:
    raise SystemExit("missing Welcome surface")
if not (validate < save < welcome):
    raise SystemExit("ready signer validation must precede projection persistence and Welcome")
if stop == -1 or not (validate < stop < save):
    raise SystemExit("fresh daemon must be stopped when signer readiness validation fails")
PY

for required_test in \
  start_runtime_readiness_accepts_paired_user_signer_custody \
  start_runtime_readiness_accepts_device_only_unbound_credentials \
  start_runtime_readiness_rejects_missing_paired_user_signer_flag \
  start_runtime_readiness_rejects_missing_credential_user_ura \
  start_runtime_readiness_rejects_failed_signer_custody_proof
do
  if ! rg -n "$required_test" "$START" >/dev/null; then
    fail "start ready signer proof missing test: $required_test"
  fi
done

echo "check-start-ready-signer-proof-boundary: ok"

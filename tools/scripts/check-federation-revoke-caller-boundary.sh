#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  echo "check-federation-revoke-caller-boundary: $*" >&2
  exit 1
}

REMOTE="src/daemon/invocation/routing/remote_invoke.rs"
DEVICE="src/cli/commands/groups/device.rs"
STOP="src/cli/commands/stop.rs"
RESET="src/cli/commands/reset.rs"
JOIN="src/cli/commands/join.rs"
SELFCMD="src/cli/commands/groups/selfcmd.rs"

[[ -f "$REMOTE" ]] || fail "missing remote invoke module"

python3 - "$REMOTE" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text()

signature = re.search(
    r"pub fn invoke_federation_revoke\(\s*agent_ura: &str,\s*reason: &str,\s*caller_ura: &str,\s*\)",
    text,
    re.S,
)
if not signature:
    raise SystemExit("invoke_federation_revoke must require explicit caller_ura")

start = text.find("pub fn invoke_federation_revoke(")
end = text.find("\nfn canonical_federation_revoke_caller(", start)
if start == -1 or end == -1:
    raise SystemExit("cannot extract invoke_federation_revoke body")
body = text[start:end]

required = [
    "canonical_federation_revoke_caller(caller_ura, &local_daemon_ura)",
    'local_daemon_federation_signer(&caller_ura, "federation.revoke")',
    "ProtoEnvelope::from_target(\n        caller_ura.as_str(),",
]
for needle in required:
    if needle not in body:
        raise SystemExit(f"invoke_federation_revoke missing boundary: {needle}")

for forbidden in [
    'local_daemon_federation_signer(&local_daemon_ura, "federation.revoke")',
    "ProtoEnvelope::from_target(\n        local_daemon_ura.as_str(),",
]:
    if forbidden in body:
        raise SystemExit(f"invoke_federation_revoke still reselects ambient caller: {forbidden}")

validator = text[end:text.find("\n#[cfg(test)]", end)]
for needle in [
    "caller != local",
    "does not match active local daemon",
    "URAKind::Device | crate::core::ura::URAKind::Authority",
]:
    if needle not in validator:
        raise SystemExit(f"canonical_federation_revoke_caller missing invariant: {needle}")
PY

if rg -n '_ = caller_ura' "$DEVICE" "$STOP" "$RESET" "$JOIN" "$SELFCMD" >/dev/null; then
  fail "production revoke caller must not be accepted and ignored"
fi

python3 - "$DEVICE" "$STOP" "$RESET" "$JOIN" "$SELFCMD" <<'PY'
import pathlib
import sys

checks = {
    "src/cli/commands/groups/device.rs": "target_ura, reason, caller_ura",
    "src/cli/commands/stop.rs": '&caller_ura,\n                "device shutdown",\n                &caller_ura',
    "src/cli/commands/reset.rs": 'device_ura,\n        "device-reset",\n        device_ura',
    "src/cli/commands/join.rs": 'device_ura,\n        "device-rejoin",\n        device_ura',
    "src/cli/commands/groups/selfcmd.rs": '&identity.device_ura,\n                "self uninstall",\n                &identity.device_ura',
}
for raw in sys.argv[1:]:
    path = pathlib.Path(raw)
    text = path.read_text()
    needle = checks[str(path)]
    if needle not in text:
        raise SystemExit(f"{path}: federation.revoke call must pass explicit caller_ura")
PY

echo "check-federation-revoke-caller-boundary: OK"

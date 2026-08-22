#!/usr/bin/env bash
# Shared helpers for RemoteApp lifecycle E2E harnesses.
#
# The helpers keep public lifecycle probes aligned with the runtime boundary:
# callers invoke full Ability URAs resolved from the committed ability catalog
# and carry the session approval receipt as causal context for session-bound
# calls.

remoteapp_resolve_rpc_ability_ura() {
  local catalog_path="$1"
  local ability_name="$2"
  python3 - "$catalog_path" "$ability_name" <<'PY'
import json
import sys

catalog_path, ability_name = sys.argv[1:3]
with open(catalog_path, encoding="utf-8") as f:
    rows = json.load(f)
if not isinstance(rows, list):
    raise SystemExit("ability list --format json must return an array")
candidates = [
    row for row in rows
    if row.get("name") == ability_name
    and row.get("call_mode") == "rpc"
    and isinstance(row.get("ability_ura"), str)
    and row["ability_ura"].startswith("easynet:///r/")
]
if len(candidates) != 1:
    sample = [
        {
            "name": row.get("name"),
            "call_mode": row.get("call_mode"),
            "ability_ura": row.get("ability_ura"),
            "descriptor_ref": row.get("descriptor_ref"),
        }
        for row in rows
        if row.get("name") == ability_name
    ]
    raise SystemExit(
        f"{ability_name} rpc Ability URA must resolve exactly once; got {len(candidates)} sample={sample}"
    )
print(candidates[0]["ability_ura"])
PY
}

remoteapp_session_approval_causal_context_json() {
  local create_session_path="$1"
  python3 - "$create_session_path" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    response = json.load(f)
receipt = (
    response
    .get("session", {})
    .get("consent", {})
    .get("approval_receipt")
)
if not isinstance(receipt, dict):
    raise SystemExit("create_session response missing session.consent.approval_receipt")
receipt_hash = receipt.get("receipt_hash")
receipt_ura = receipt.get("receipt_ura")
if not isinstance(receipt_hash, str) or len(receipt_hash) != 64:
    raise SystemExit("session approval receipt_hash must be 64 hex characters")
if not isinstance(receipt_ura, str) or not receipt_ura.startswith("easynet:///r/"):
    raise SystemExit("session approval receipt_ura must be a canonical EasyNet URA")
print(json.dumps({
    "form": "scalar",
    "receipt_hash_hex": receipt_hash,
    "receipt_ura": receipt_ura,
}, separators=(",", ":")))
PY
}


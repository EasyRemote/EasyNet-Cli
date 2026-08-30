#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-attestation-trust.py"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

openssl genpkey -algorithm ED25519 -out "$TMP_DIR/key-1.pem" >/dev/null 2>&1
openssl pkey -in "$TMP_DIR/key-1.pem" -pubout -out "$TMP_DIR/key-1-public.pem" >/dev/null 2>&1
openssl genpkey -algorithm ED25519 -out "$TMP_DIR/key-2.pem" >/dev/null 2>&1
openssl pkey -in "$TMP_DIR/key-2.pem" -pubout -out "$TMP_DIR/key-2-public.pem" >/dev/null 2>&1

python3 - "$TMP_DIR" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
now = 2_000_000_000_000
key = lambda name: (root / name).read_text()
bundle = {
    "schema": "easynet.remoteapp.attestation-trust.v3",
    "generation": 1,
    "updated_at_ms": now,
    "keys": [{
        "keyid": "campaign-1",
        "signer_ura": "easynet:///r/test/service/remoteapp-campaign-authority",
        "roles": ["campaign_authority"],
        "public_key_pem": key("key-1-public.pem"),
        "not_before_ms": now - 1000,
        "not_after_ms": now + 100000,
        "revoked_at_ms": None,
    }],
}
(root / "trust.json").write_text(json.dumps(bundle), encoding="utf-8")
successor = {
    "keyid": "observer-2",
    "signer_ura": "easynet:///r/test/service/remoteapp-observer",
    "roles": ["observer_runner"],
    "domains": ["input_injection"],
    "platforms": ["linux"],
    "public_key_pem": key("key-2-public.pem"),
    "not_before_ms": now + 1,
    "not_after_ms": now + 100000,
    "revoked_at_ms": None,
}
(root / "successor.json").write_text(json.dumps(successor), encoding="utf-8")
PY

python3 "$SCRIPT" validate --trust-bundle "$TMP_DIR/trust.json" \
  --at-ms 2000000000000 >"$TMP_DIR/validate.json"
grep -q '"campaign-1": "active"' "$TMP_DIR/validate.json"

python3 "$SCRIPT" rotate --current "$TMP_DIR/trust.json" \
  --new-key-record "$TMP_DIR/successor.json" --output "$TMP_DIR/rotated.json" \
  --updated-at-ms 2000000000001
python3 - "$TMP_DIR/rotated.json" <<'PY'
import json
import sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
assert value["generation"] == 2
assert [row["keyid"] for row in value["keys"]] == ["campaign-1", "observer-2"]
PY

python3 "$SCRIPT" revoke --current "$TMP_DIR/rotated.json" --keyid observer-2 \
  --output "$TMP_DIR/revoked.json" --revoked-at-ms 2000000000010
python3 - "$TMP_DIR/revoked.json" <<'PY'
import json
import sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
assert value["generation"] == 3
assert value["keys"][1]["revoked_at_ms"] == 2000000000010
PY
python3 "$SCRIPT" validate --trust-bundle "$TMP_DIR/revoked.json" \
  --at-ms 2000000000020 >"$TMP_DIR/revoked-status.json"
grep -q 'signing key is revoked' "$TMP_DIR/revoked-status.json"

if python3 "$SCRIPT" rotate --current "$TMP_DIR/rotated.json" \
    --new-key-record "$TMP_DIR/successor.json" --output "$TMP_DIR/duplicate.json" \
    --updated-at-ms 2000000000002 >"$TMP_DIR/duplicate.out" 2>&1; then
  echo "duplicate trust key rotation was accepted" >&2
  exit 1
fi
grep -q "already exists" "$TMP_DIR/duplicate.out"

echo "test_remoteapp_attestation_trust: ok"

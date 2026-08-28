#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FINALIZER="$ROOT/tools/scripts/remoteapp-product-finalize.py"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

fail() {
  printf 'test_remoteapp_product_finalization: %s\n' "$1" >&2
  exit 1
}

for authority in campaign observer completion; do
  openssl genpkey -algorithm ED25519 -out "$TMP_DIR/$authority-private.pem" >/dev/null 2>&1
  openssl pkey -in "$TMP_DIR/$authority-private.pem" -pubout \
    -out "$TMP_DIR/$authority-public.pem" >/dev/null 2>&1
done

python3 - "$TMP_DIR" <<'PY'
import json
from pathlib import Path
import sys

root = Path(sys.argv[1])
campaign_id = "11111111-1111-4111-8111-111111111111"
run_id = "22222222-2222-4222-8222-222222222222"
digest = "sha256:" + "a" * 64
source = {"git_commit": "1" * 40, "repository": "EasyNet-Cli"}
build = {"receipt_verifier_sha256": "sha256:" + "b" * 64}
domain_ids = (
    "browser_transport_resume",
    "frontend_product_flow",
    "browser_lifecycle",
    "cross_device_smoke",
    "cross_device_remoteapp",
    "cross_platform_capture",
    "input_injection",
    "media_adaptation",
    "multi_window_tracking",
    "network_fallback",
    "session_timeout_window",
    "session_timeout_application",
    "session_cancel_window",
    "session_cancel_application",
    "permission_revoke_window",
    "permission_revoke_application",
    "session_resume_window",
    "session_resume_application",
    "crash_restart_recovery",
)

def trust(completion_signer="easynet:///r/localhost/authority/product-completion"):
    rows = []
    for keyid, signer, role in (
        ("campaign", "easynet:///r/localhost/authority/campaign", "campaign_authority"),
        ("observer", "easynet:///r/localhost/authority/observer", "observer_runner"),
        ("completion", completion_signer, "product_completion_authority"),
    ):
        row = {
            "keyid": keyid,
            "signer_ura": signer,
            "roles": [role],
            "public_key_pem": (root / f"{keyid}-public.pem").read_text(),
            "not_before_ms": 0,
            "not_after_ms": 10_000,
            "revoked_at_ms": None,
        }
        if role == "observer_runner":
            row["domains"] = list(domain_ids)
            row["platforms"] = ["macos"]
        rows.append(row)
    return {
        "schema": "easynet.remoteapp.attestation-trust.v3",
        "generation": 1,
        "updated_at_ms": 0,
        "keys": rows,
    }

candidate = {
    "schema": "easynet.remoteapp.product-completion-candidate.v1",
    "script": "tools/scripts/remoteapp-product-completion-e2e.sh",
    "status": "passed",
    "mode": "check",
    "reason": "all product evidence passed; completion authority signature pending",
    "evidence_origin": "live_runner",
    "product_complete_eligible": True,
    "product_complete_claim": False,
    "finalization_state": "completion_signature_pending",
    "contract_fixture_mode": False,
    "campaign_verified": True,
    "campaign_verification": {
        "schema": "easynet.remoteapp.campaign-bundle.v2",
        "status": "verified",
        "campaign_id": campaign_id,
        "run_id": run_id,
        "campaign_sha256": digest,
        "campaign_signer_ura": "easynet:///r/localhost/authority/campaign",
        "campaign_keyid": "campaign",
        "issued_at_ms": 1_000,
        "expires_at_ms": 5_000,
        "source": source,
        "build": build,
        "domains": {
            domain_id: {
                "signer_ura": "easynet:///r/localhost/authority/observer",
                "keyid": "observer",
            }
            for domain_id in domain_ids
        },
        "all_receipts_verified": True,
        "replay_ledger_reserved": False,
    },
    "required_evidence_count": len(domain_ids),
    "checks": [
        {
            "id": domain_id,
            "status": "passed",
            "evidence_origin": "live_runner",
            "errors": [],
        }
        for domain_id in domain_ids
    ],
    "errors": [],
}
(root / "candidate.json").write_text(json.dumps(candidate, indent=2, sort_keys=True) + "\n")
(root / "trust.json").write_text(json.dumps(trust(), indent=2, sort_keys=True) + "\n")
(root / "same-custody-trust.json").write_text(
    json.dumps(
        trust("easynet:///r/localhost/authority/campaign"),
        indent=2,
        sort_keys=True,
    ) + "\n"
)
PY

write_attestation() {
  local candidate="$1"
  local issued_at_ms="$2"
  local decision="$3"
  local output="$4"
  local stem="$5"
  python3 - "$candidate" "$issued_at_ms" "$decision" "$TMP_DIR/$stem-payload.json" "$TMP_DIR/$stem-message.bin" <<'PY'
import base64
import hashlib
import json
from pathlib import Path
import sys

candidate_path = Path(sys.argv[1])
candidate_body = candidate_path.read_bytes()
candidate = json.loads(candidate_body)
verification = candidate["campaign_verification"]
payload = {
    "schema": "easynet.remoteapp.product-completion-attestation.v1",
    "candidate_sha256": "sha256:" + hashlib.sha256(candidate_body).hexdigest(),
    "campaign_id": verification["campaign_id"],
    "run_id": verification["run_id"],
    "campaign_sha256": verification["campaign_sha256"],
    "issued_at_ms": int(sys.argv[2]),
    "decision": sys.argv[3],
    "source": verification["source"],
    "build": verification["build"],
}
payload_body = json.dumps(payload, separators=(",", ":"), sort_keys=True).encode()
payload_type = b"application/vnd.easynet.remoteapp.product-completion.v1+json"
message = b" ".join((
    b"DSSEv1",
    str(len(payload_type)).encode(),
    payload_type,
    str(len(payload_body)).encode(),
    payload_body,
))
Path(sys.argv[4]).write_bytes(payload_body)
Path(sys.argv[5]).write_bytes(message)
PY
  openssl pkeyutl -sign -inkey "$TMP_DIR/completion-private.pem" -rawin \
    -in "$TMP_DIR/$stem-message.bin" -out "$TMP_DIR/$stem-signature.bin" >/dev/null 2>&1
  python3 - "$TMP_DIR/$stem-payload.json" "$TMP_DIR/$stem-signature.bin" "$output" <<'PY'
import base64
import json
from pathlib import Path
import sys

payload = Path(sys.argv[1]).read_bytes()
signature = Path(sys.argv[2]).read_bytes()
envelope = {
    "payloadType": "application/vnd.easynet.remoteapp.product-completion.v1+json",
    "payload": base64.b64encode(payload).decode(),
    "signatures": [{"keyid": "completion", "sig": base64.b64encode(signature).decode()}],
}
Path(sys.argv[3]).write_text(json.dumps(envelope, indent=2, sort_keys=True) + "\n")
PY
}

python3 - "$FINALIZER" "$TMP_DIR" <<'PY'
import importlib.util
import json
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location("finalizer", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
root = Path(sys.argv[2])
result = module.prepare_completion_signing_material(
    root / "candidate.json",
    root / "prepared-statement.json",
    root / "prepared-pae.bin",
    now_ms=2_000,
)
assert result["status"] == "signing_material_prepared"
assert (root / "prepared-statement.json").read_bytes() not in {
    b"",
    (root / "prepared-pae.bin").read_bytes(),
}
incomplete = json.loads((root / "candidate.json").read_text())
incomplete["checks"].pop()
(root / "incomplete-candidate.json").write_text(
    json.dumps(incomplete, indent=2, sort_keys=True) + "\n"
)
try:
    module.prepare_completion_signing_material(
        root / "incomplete-candidate.json",
        root / "incomplete-statement.json",
        root / "incomplete-pae.bin",
        now_ms=2_000,
    )
except ValueError as error:
    assert "checks do not cover every product domain" in str(error)
else:
    raise AssertionError("signing preparation accepted an incomplete product matrix")
PY
openssl pkeyutl -sign -inkey "$TMP_DIR/completion-private.pem" -rawin \
  -in "$TMP_DIR/prepared-pae.bin" -out "$TMP_DIR/prepared-signature.bin" >/dev/null 2>&1
python3 - "$FINALIZER" "$TMP_DIR" <<'PY'
import importlib.util
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location("finalizer", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
root = Path(sys.argv[2])
result = module.assemble_completion_attestation(
    root / "candidate.json",
    root / "prepared-statement.json",
    root / "prepared-signature.bin",
    "completion",
    root / "trust.json",
    root / "attestation.json",
    now_ms=2_100,
)
assert result["status"] == "completion_attestation_assembled"
assert result["completion_keyid"] == "completion"
(root / "invalid-signature.bin").write_bytes(b"\0" * 64)
try:
    module.assemble_completion_attestation(
        root / "candidate.json",
        root / "prepared-statement.json",
        root / "invalid-signature.bin",
        "completion",
        root / "trust.json",
        root / "invalid-attestation.json",
        now_ms=2_100,
    )
except ValueError as error:
    assert "no valid trusted signature" in str(error)
else:
    raise AssertionError("attestation assembly accepted an invalid KMS signature")
try:
    module.assemble_completion_attestation(
        root / "candidate.json",
        root / "prepared-statement.json",
        root / "prepared-signature.bin",
        "campaign",
        root / "trust.json",
        root / "wrong-role-attestation.json",
        now_ms=2_100,
    )
except ValueError as error:
    assert "not trusted for role" in str(error)
else:
    raise AssertionError("attestation assembly accepted a campaign-authority key")
PY

python3 - "$FINALIZER" "$TMP_DIR" <<'PY'
import importlib.util
import json
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location("finalizer", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
root = Path(sys.argv[2])
report = module.finalize_candidate(
    root / "candidate.json",
    root / "attestation.json",
    root / "trust.json",
    root / "replay",
    root / "final.json",
    now_ms=2_100,
)
assert report["schema"] == "easynet.remoteapp.product-completion-report.v1"
assert report["product_complete_claim"] is True
assert report["finalization_state"] == "completion_authority_verified"
assert report["completion_keyid"] == "completion"
assert report["replay_ledger_reserved"] is True
ledger = json.loads(next((root / "replay").iterdir()).read_text())
assert ledger["completion_statement_sha256"] == report["completion_statement_sha256"]
assert ledger["final_report_sha256"].startswith("sha256:")
verification = module.verify_final_report(
    root / "final.json",
    root / "trust.json",
    root / "replay",
    now_ms=20_000,
)
assert verification["status"] == "verified"
assert verification["final_report_sha256"] == ledger["final_report_sha256"]

# Exact retry is the recovery path for a crash after replay reservation.
retry = module.finalize_candidate(
    root / "candidate.json",
    root / "attestation.json",
    root / "trust.json",
    root / "replay",
    root / "final.json",
    now_ms=2_100,
)
assert retry == report

tampered = json.loads((root / "final.json").read_text())
tampered["reason"] = "untrusted rewrite"
(root / "tampered-final-report.json").write_text(
    json.dumps(tampered, indent=2, sort_keys=True) + "\n"
)
try:
    module.verify_final_report(
        root / "tampered-final-report.json",
        root / "trust.json",
        root / "replay",
        now_ms=2_100,
    )
except ValueError as error:
    assert "canonical authorized projection" in str(error)
else:
    raise AssertionError("standalone final-report verifier accepted a mutated claim")

ledger_path = next((root / "replay").iterdir())
ledger_body = ledger_path.read_text()
tampered_ledger = json.loads(ledger_body)
tampered_ledger["final_report_sha256"] = "sha256:" + "0" * 64
ledger_path.write_text(json.dumps(tampered_ledger, indent=2, sort_keys=True) + "\n")
try:
    module.verify_final_report(
        root / "final.json",
        root / "trust.json",
        root / "replay",
        now_ms=20_000,
    )
except ValueError as error:
    assert "replay record does not match" in str(error)
else:
    raise AssertionError("standalone verifier accepted a mutated replay record")
finally:
    ledger_path.write_text(ledger_body)
PY

python3 - "$TMP_DIR/candidate.json" "$TMP_DIR/tampered-candidate.json" <<'PY'
import json
from pathlib import Path
import sys

candidate = json.loads(Path(sys.argv[1]).read_text())
candidate["checks"][0]["diagnostic"] = "injected"
Path(sys.argv[2]).write_text(json.dumps(candidate, indent=2, sort_keys=True) + "\n")
PY
if python3 - "$FINALIZER" "$TMP_DIR" 2>"$TMP_DIR/tampered.err" <<'PY'
import importlib.util
from pathlib import Path
import sys
spec = importlib.util.spec_from_file_location("finalizer", sys.argv[1])
module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
root = Path(sys.argv[2])
module.finalize_candidate(root / "tampered-candidate.json", root / "attestation.json", root / "trust.json", root / "tampered-replay", root / "tampered-final.json", now_ms=2100)
PY
then
  fail "finalizer accepted a candidate changed after completion signing"
fi
grep -q "candidate_sha256 does not match candidate" "$TMP_DIR/tampered.err" || \
  fail "candidate mutation did not fail at the exact-byte signature binding"

write_attestation "$TMP_DIR/candidate.json" 2000 reject "$TMP_DIR/reject-attestation.json" reject
if python3 - "$FINALIZER" "$TMP_DIR" 2>"$TMP_DIR/reject.err" <<'PY'
import importlib.util
from pathlib import Path
import sys
spec = importlib.util.spec_from_file_location("finalizer", sys.argv[1])
module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
root = Path(sys.argv[2])
module.finalize_candidate(root / "candidate.json", root / "reject-attestation.json", root / "trust.json", root / "reject-replay", root / "reject-final.json", now_ms=2100)
PY
then
  fail "finalizer accepted a signed non-completion decision"
fi
grep -q "decision does not match candidate" "$TMP_DIR/reject.err" || \
  fail "non-completion decision did not fail at decision validation"

if python3 - "$FINALIZER" "$TMP_DIR" 2>"$TMP_DIR/custody.err" <<'PY'
import importlib.util
from pathlib import Path
import sys
spec = importlib.util.spec_from_file_location("finalizer", sys.argv[1])
module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
root = Path(sys.argv[2])
module.finalize_candidate(root / "candidate.json", root / "attestation.json", root / "same-custody-trust.json", root / "custody-replay", root / "custody-final.json", now_ms=2100)
PY
then
  fail "finalizer accepted completion authority sharing the campaign signer identity"
fi
grep -q "independent key and signer identity" "$TMP_DIR/custody.err" || \
  fail "shared completion custody did not fail at authority separation"

write_attestation "$TMP_DIR/candidate.json" 2050 product_complete \
  "$TMP_DIR/replay-attestation.json" replay
if python3 - "$FINALIZER" "$TMP_DIR" 2>"$TMP_DIR/replay.err" <<'PY'
import importlib.util
from pathlib import Path
import sys
spec = importlib.util.spec_from_file_location("finalizer", sys.argv[1])
module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
root = Path(sys.argv[2])
module.finalize_candidate(root / "candidate.json", root / "replay-attestation.json", root / "trust.json", root / "replay", root / "second-final.json", now_ms=2100)
PY
then
  fail "finalizer accepted a different completion statement for a consumed campaign"
fi
grep -q "already been consumed" "$TMP_DIR/replay.err" || \
  fail "alternate completion statement did not fail at campaign replay"

printf 'test_remoteapp_product_finalization: ok\n'

#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/remoteapp-evidence-provenance.py"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

printf '%s\n' '{"status":"passed","evidence_origin":"live_runner"}' >"$TMP_DIR/live.json"
printf '%s\n' '{"status":"passed","evidence_origin":"contract_self_test"}' >"$TMP_DIR/self-test.json"
printf '%s\n' '{"status":"passed"}' >"$TMP_DIR/missing.json"
printf '%s\n' '{"status":"passed","product_complete_claim":false}' >"$TMP_DIR/report.json"

python3 "$SCRIPT" verify --mode run --evidence "$TMP_DIR/live.json"
python3 "$SCRIPT" verify --mode self-test --evidence "$TMP_DIR/self-test.json"

if python3 "$SCRIPT" verify --mode run --evidence "$TMP_DIR/self-test.json" \
    >"$TMP_DIR/wrong.stdout" 2>"$TMP_DIR/wrong.stderr"; then
  echo "run mode accepted contract self-test provenance" >&2
  exit 1
fi
grep -q "evidence_origin must be live_runner" "$TMP_DIR/wrong.stderr"

if python3 "$SCRIPT" verify --mode run --evidence "$TMP_DIR/missing.json" \
    >"$TMP_DIR/missing.stdout" 2>"$TMP_DIR/missing.stderr"; then
  echo "run mode accepted missing provenance" >&2
  exit 1
fi
grep -q "observed None" "$TMP_DIR/missing.stderr"

python3 "$SCRIPT" project-report --mode run \
  --evidence "$TMP_DIR/live.json" --report "$TMP_DIR/report.json"
python3 - "$TMP_DIR/report.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
assert report["status"] == "passed"
assert report["evidence_origin"] == "live_runner"
assert report["product_complete_claim"] is False
PY

openssl genpkey -algorithm ED25519 -out "$TMP_DIR/campaign-private.pem" >/dev/null 2>&1
openssl pkey -in "$TMP_DIR/campaign-private.pem" -pubout \
  -out "$TMP_DIR/campaign-public.pem" >/dev/null 2>&1
openssl genpkey -algorithm ED25519 -out "$TMP_DIR/observer-private.pem" >/dev/null 2>&1
openssl pkey -in "$TMP_DIR/observer-private.pem" -pubout \
  -out "$TMP_DIR/observer-public.pem" >/dev/null 2>&1
mkdir -p "$TMP_DIR/campaign-root"
printf '%s\n' '{"status":"passed","script":"domain-e2e"}' \
  >"$TMP_DIR/campaign-root/domain-report.json"
printf '%s\n' '{}' >"$TMP_DIR/campaign-root/domain-receipts.json"

python3 - "$TMP_DIR" <<'PY'
import base64
import hashlib
import json
import pathlib
import time

root = pathlib.Path(__import__("sys").argv[1])
now = int(time.time() * 1000)
digest = lambda body: "sha256:" + hashlib.sha256(body).hexdigest()
build = {
    "runtime_sha256": "sha256:" + "1" * 64,
    "remote_desktop_plugin_sha256": "sha256:" + "2" * 64,
    "frontend_bundle_sha256": "sha256:" + "3" * 64,
    "receipt_verifier_sha256": "sha256:" + "7" * 64,
}
source = {"git_commit": "a" * 40, "dirty": False}
campaign = {
    "schema": "easynet.remoteapp.campaign.v2",
    "campaign_id": "11111111-1111-4111-8111-111111111111",
    "run_id": "22222222-2222-4222-8222-222222222222",
    "challenge_nonce": base64.b64encode(b"n" * 32).decode(),
    "issued_at_ms": now - 1000,
    "expires_at_ms": now + 60000,
    "source": source,
    "build": build,
    "receipt_signers": [{
        "signer_ura": "easynet:///r/test/device/provider",
        "ed25519_public_key_b64": base64.b64encode(b"k" * 32).decode(),
    }],
    "required_domains": ["domain"],
}
payload = json.dumps(campaign, sort_keys=True, separators=(",", ":")).encode()
payload_type = "application/vnd.easynet.remoteapp.campaign.v2+json"
pae = f"DSSEv1 {len(payload_type.encode())} {payload_type} {len(payload)} ".encode() + payload
(root / "campaign.payload").write_bytes(payload)
(root / "campaign.pae").write_bytes(pae)
(root / "campaign-meta.json").write_text(
    json.dumps({"now": now, "source": source, "build": build, "digest": digest(payload)}),
    encoding="utf-8",
)
(root / "trust.json").write_text(json.dumps({
    "schema": "easynet.remoteapp.attestation-trust.v3",
    "generation": 1,
    "updated_at_ms": now - 2000,
    "keys": [
        {
            "keyid": "campaign-key",
            "signer_ura": "easynet:///r/test/service/remoteapp-campaign-authority",
            "roles": ["campaign_authority"],
            "public_key_pem": (root / "campaign-public.pem").read_text(),
            "not_before_ms": now - 10000,
            "not_after_ms": now + 600000,
            "revoked_at_ms": None,
        },
        {
            "keyid": "observer-key",
            "signer_ura": "easynet:///r/test/service/remoteapp-observer",
            "roles": ["observer_runner"],
            "domains": ["domain"],
            "platforms": ["linux"],
            "public_key_pem": (root / "observer-public.pem").read_text(),
            "not_before_ms": now - 10000,
            "not_after_ms": now + 600000,
            "revoked_at_ms": None,
        },
    ],
}), encoding="utf-8")
PY
openssl pkeyutl -sign -rawin -inkey "$TMP_DIR/campaign-private.pem" \
  -in "$TMP_DIR/campaign.pae" -out "$TMP_DIR/campaign.sig"

python3 - "$TMP_DIR" <<'PY'
import base64
import hashlib
import json
import pathlib

root = pathlib.Path(__import__("sys").argv[1])
meta = json.loads((root / "campaign-meta.json").read_text())
report = (root / "campaign-root/domain-report.json").read_bytes()
arguments = json.dumps(
    {"session_id": "rd-signed-campaign"},
    sort_keys=True,
    separators=(",", ":"),
).encode()
receipt_proof_set = {
    "schema": "easynet.remoteapp.receipt-proof-set.v2",
    "campaign": {
        "campaign_id": "11111111-1111-4111-8111-111111111111",
        "run_id": "22222222-2222-4222-8222-222222222222",
        "challenge_nonce_b64": base64.b64encode(b"n" * 32).decode(),
        "domain_id": "domain",
        "caller_device_ura": "easynet:///r/test/device/caller",
        "provider_device_ura": "easynet:///r/test/device/provider",
    },
    "proofs": [
        {
            "proof_id": "create-session",
            "invocation_ura": "easynet:///r/test/invocation/01",
            "descriptor_ref": "easynet:///r/test/ability/system-agent.device.remote_desktop.create_session@1.0.0#" + "4" * 64,
            "subject_ura": "easynet:///r/test/resource/window.1",
            "caller_ura": "easynet:///r/test/user/caller",
            "callee_ura": "easynet:///r/test/agent/device.provider.remote-desktop",
            "session_id": "rd-signed-campaign",
            "arguments_b64": base64.b64encode(arguments).decode(),
            "encoding": "prost.base64",
            "admission_receipt_b64": base64.b64encode(b"admission-1").decode(),
            "terminal_receipt_b64": base64.b64encode(b"terminal-1").decode(),
        },
        {
            "proof_id": "end-session",
            "invocation_ura": "easynet:///r/test/invocation/02",
            "descriptor_ref": "easynet:///r/test/ability/system-agent.device.remote_desktop.end_session@1.0.0#" + "5" * 64,
            "subject_ura": "easynet:///r/test/resource/window.1",
            "caller_ura": "easynet:///r/test/user/caller",
            "callee_ura": "easynet:///r/test/agent/device.provider.remote-desktop",
            "session_id": "rd-signed-campaign",
            "arguments_b64": base64.b64encode(arguments).decode(),
            "encoding": "prost.base64",
            "admission_receipt_b64": base64.b64encode(b"admission-2").decode(),
            "terminal_receipt_b64": base64.b64encode(b"terminal-2").decode(),
        },
    ],
}
(root / "campaign-root/domain-receipts.json").write_text(
    json.dumps(receipt_proof_set, sort_keys=True), encoding="utf-8"
)
receipt_proof = (root / "campaign-root/domain-receipts.json").read_bytes()
attestation = {
    "schema": "easynet.remoteapp.live-attestation.v2",
    "campaign_sha256": meta["digest"],
    "domain_id": "domain",
    "run_id": "22222222-2222-4222-8222-222222222222",
    "started_at_ms": meta["now"],
    "completed_at_ms": meta["now"] + 1,
    "source": meta["source"],
    "build": meta["build"],
    "producer": {
        "signer_ura": "easynet:///r/test/service/remoteapp-observer",
        "role": "observer_runner",
        "key_id": "observer-key",
        "platform": "linux",
    },
    "topology": {
        "caller_device_ura": "easynet:///r/test/device/caller",
        "provider_device_ura": "easynet:///r/test/device/provider",
    },
    "bindings": {
        "receipt_proof": {
            "path": "domain-receipts.json",
            "sha256": "sha256:" + hashlib.sha256(receipt_proof).hexdigest(),
            "size_bytes": len(receipt_proof),
        },
    },
    "evidence": {
        "path": "domain-report.json",
        "sha256": "sha256:" + hashlib.sha256(report).hexdigest(),
        "size_bytes": len(report),
    },
    "artifacts": [],
}
payload = json.dumps(attestation, sort_keys=True, separators=(",", ":")).encode()
payload_type = "application/vnd.easynet.remoteapp.live-attestation.v2+json"
pae = f"DSSEv1 {len(payload_type.encode())} {payload_type} {len(payload)} ".encode() + payload
(root / "attestation.payload").write_bytes(payload)
(root / "attestation.pae").write_bytes(pae)
PY
openssl pkeyutl -sign -rawin -inkey "$TMP_DIR/observer-private.pem" \
  -in "$TMP_DIR/attestation.pae" -out "$TMP_DIR/attestation.sig"

python3 - "$TMP_DIR" <<'PY'
import base64
import json
import pathlib

root = pathlib.Path(__import__("sys").argv[1])
envelope = lambda payload_type, payload, keyid, signature: {
    "payloadType": payload_type,
    "payload": base64.b64encode(payload).decode(),
    "signatures": [{"keyid": keyid, "sig": base64.b64encode(signature).decode()}],
}
bundle = {
    "schema": "easynet.remoteapp.campaign-bundle.v2",
    "campaign": envelope(
        "application/vnd.easynet.remoteapp.campaign.v2+json",
        (root / "campaign.payload").read_bytes(),
        "campaign-key",
        (root / "campaign.sig").read_bytes(),
    ),
    "attestations": [envelope(
        "application/vnd.easynet.remoteapp.live-attestation.v2+json",
        (root / "attestation.payload").read_bytes(),
        "observer-key",
        (root / "attestation.sig").read_bytes(),
    )],
}
(root / "bundle.json").write_text(json.dumps(bundle), encoding="utf-8")
PY

python3 "$SCRIPT" verify-campaign \
  --bundle "$TMP_DIR/bundle.json" \
  --trust-bundle "$TMP_DIR/trust.json" \
  --campaign-root "$TMP_DIR/campaign-root" \
  --report "domain=$TMP_DIR/campaign-root/domain-report.json" \
  --output "$TMP_DIR/verified-campaign.json"
grep -q '"status": "attestation_verified_receipts_pending"' \
  "$TMP_DIR/verified-campaign.json"

python3 - "$TMP_DIR" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
proof_set = json.loads((root / "campaign-root/domain-receipts.json").read_text())
proof = proof_set["proofs"][0]
binding = {
    field: proof[field]
    for field in (
        "proof_id",
        "descriptor_ref",
        "subject_ura",
        "caller_ura",
        "callee_ura",
        "session_id",
    )
}
(root / "producer-campaign.json").write_text(json.dumps(proof_set["campaign"]))
(root / "producer-proof.json").write_text(json.dumps(binding))
(root / "producer-args.json").write_text(
    '{"session_id":"rd-signed-campaign","consent_ticket":"ticket"}'
)
PY
PRODUCER_NONCE="$(python3 "$SCRIPT" derive-invocation-nonce \
  --campaign-binding "$TMP_DIR/producer-campaign.json" \
  --proof-binding "$TMP_DIR/producer-proof.json" \
  --arguments-json "$TMP_DIR/producer-args.json")"
python3 - "$TMP_DIR" "$PRODUCER_NONCE" <<'PY'
import base64
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
binding = json.loads((root / "producer-proof.json").read_text())
meta = {
    "metadata_state": "finalization_checkpoints_verified",
    "ledger_state": "completed",
    "invocation_ura": "easynet:///r/test/invocation/producer-01",
    "subject_ura": binding["subject_ura"],
    "caller_ura": binding["caller_ura"],
    "callee_ura": binding["callee_ura"],
    "args": {"session_id": binding["session_id"], "consent_ticket": "ticket"},
    "arguments_b64": base64.b64encode(
        (
            '{"session_id":"' + binding["session_id"]
            + '","consent_ticket":"ticket"}'
        ).encode()
    ).decode(),
    "nonce": sys.argv[2],
    "receipt": {
        "verification_checkpoints": {
            "encoding": "prost.base64",
            "admission_receipt_b64": base64.b64encode(b"producer-admission").decode(),
            "terminal_receipt_b64": base64.b64encode(b"producer-terminal").decode(),
        }
    },
}
(root / "producer-meta.json").write_text(json.dumps(meta))
PY
python3 "$SCRIPT" append-receipt-proof \
  --proof-set "$TMP_DIR/produced-receipts.json" \
  --campaign-binding "$TMP_DIR/producer-campaign.json" \
  --proof-binding "$TMP_DIR/producer-proof.json" \
  --arguments-json "$TMP_DIR/producer-args.json" \
  --invocation-meta "$TMP_DIR/producer-meta.json"
python3 - "$TMP_DIR/produced-receipts.json" "$PRODUCER_NONCE" <<'PY'
import json
import base64
import sys

proof_set = json.load(open(sys.argv[1], encoding="utf-8"))
assert proof_set["schema"] == "easynet.remoteapp.receipt-proof-set.v2"
assert len(proof_set["proofs"]) == 1
assert len(sys.argv[2]) == 32
assert base64.b64decode(proof_set["proofs"][0]["arguments_b64"]).startswith(
    b'{"session_id"'
)
PY
if python3 "$SCRIPT" append-receipt-proof \
    --proof-set "$TMP_DIR/produced-receipts.json" \
    --campaign-binding "$TMP_DIR/producer-campaign.json" \
    --proof-binding "$TMP_DIR/producer-proof.json" \
    --arguments-json "$TMP_DIR/producer-args.json" \
    --invocation-meta "$TMP_DIR/producer-meta.json" \
    >"$TMP_DIR/duplicate-proof.out" 2>&1; then
  echo "receipt producer accepted a duplicate proof" >&2
  exit 1
fi
grep -q "already exists" "$TMP_DIR/duplicate-proof.out"

printf '%s\n' '{"status":"passed","unsigned":true}' \
  >"$TMP_DIR/campaign-root/unsigned-nested.json"
python3 - "$SCRIPT" "$TMP_DIR" <<'PY'
import importlib.util
import base64
import json
import pathlib
import sys

script = pathlib.Path(sys.argv[1])
root = pathlib.Path(sys.argv[2])
spec = importlib.util.spec_from_file_location("provenance", script)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
verification = json.loads((root / "verified-campaign.json").read_text())
domain = verification["domains"]["domain"]
receipt_path = root / "campaign-root/domain-receipts.json"
receipt_body = module.read_verified_bytes(
    verification, receipt_path, "signed receipt proof set"
)
validated_proofs = module.validate_receipt_proof_set(
    receipt_body,
    campaign_id=verification["campaign_id"],
    run_id=verification["run_id"],
    challenge_nonce_b64=domain["receipt_expectations"]["campaign"]["challenge_nonce_b64"],
    domain_id="domain",
    caller_device_ura="easynet:///r/test/device/caller",
    provider_device_ura="easynet:///r/test/device/provider",
    label="signed receipt proof set",
)
assert len(validated_proofs["proofs"]) == 2
assert (
    validated_proofs["proofs"][0]["campaign_invocation_nonce"]
    != validated_proofs["proofs"][1]["campaign_invocation_nonce"]
)

try:
    module.validate_receipt_proof_set(
        receipt_body,
        campaign_id=verification["campaign_id"],
        run_id=verification["run_id"],
        challenge_nonce_b64=base64.b64encode(b"replacement-challenge".ljust(32, b"!")).decode(),
        domain_id="domain",
        caller_device_ura="easynet:///r/test/device/caller",
        provider_device_ura="easynet:///r/test/device/provider",
        label="replayed receipt proof set",
    )
except ValueError as error:
    assert "does not match signed campaign/attestation" in str(error)
else:
    raise AssertionError("old receipt proof set was accepted under a new challenge")

wrong_second_session = json.loads(receipt_body)
wrong_second_session["proofs"][1]["arguments_b64"] = base64.b64encode(
    b'{"session_id":"another-session"}'
).decode()
try:
    module.validate_receipt_proof_set(
        json.dumps(wrong_second_session).encode(),
        campaign_id=verification["campaign_id"],
        run_id=verification["run_id"],
        challenge_nonce_b64=domain["receipt_expectations"]["campaign"]["challenge_nonce_b64"],
        domain_id="domain",
        caller_device_ura="easynet:///r/test/device/caller",
        provider_device_ura="easynet:///r/test/device/provider",
        label="receipt proof set with stale second session",
    )
except ValueError as error:
    assert "arguments session_id does not match proof" in str(error)
else:
    raise AssertionError("later receipt proof was not checked for session binding")

report = module.read_verified_json(
    verification,
    root / "campaign-root/domain-report.json",
    "signed report",
)
assert report["status"] == "passed"

try:
    module.read_verified_json(
        verification,
        root / "campaign-root/unsigned-nested.json",
        "unsigned nested evidence",
    )
except ValueError as error:
    assert "not present in the signed evidence manifest" in str(error)
else:
    raise AssertionError("unsigned nested evidence was accepted")

report_path = root / "campaign-root/domain-report.json"
original = report_path.read_bytes()
report_path.write_text('{"status":"passed","tampered":true}\n')
try:
    module.read_verified_json(verification, report_path, "tampered report")
except ValueError as error:
    assert "changed after attestation" in str(error)
else:
    raise AssertionError("post-attestation report mutation was accepted")
report_path.write_bytes(original)

receipt_original = receipt_path.read_bytes()
receipt_path.write_text('{"schema":"tampered"}\n')
try:
    module.read_verified_bytes(verification, receipt_path, "tampered receipt proof set")
except ValueError as error:
    assert "changed after attestation" in str(error)
else:
    raise AssertionError("post-attestation receipt proof mutation was accepted")
receipt_path.write_bytes(receipt_original)

trust = json.loads((root / "trust.json").read_text())
trust_meta = json.loads((root / "campaign-meta.json").read_text())
loaded_trust = module.load_trust_bundle(root / "trust.json")
module.require_trusted_key_active(
    loaded_trust["campaign-key"],
    signed_at_ms=trust_meta["now"] - 1000,
    observed_at_ms=trust_meta["now"],
    label="active campaign",
)

revoked_trust = json.loads((root / "trust.json").read_text())
revoked_trust["keys"][1]["revoked_at_ms"] = trust_meta["now"] - 1
(root / "revoked-trust.json").write_text(json.dumps(revoked_trust))
revoked = module.load_trust_bundle(root / "revoked-trust.json")
try:
    module.require_trusted_key_active(
        revoked["observer-key"],
        signed_at_ms=trust_meta["now"] - 500,
        observed_at_ms=trust_meta["now"],
        label="revoked observer",
    )
except ValueError as error:
    assert "is revoked" in str(error)
else:
    raise AssertionError("revoked observer key remained trusted")

future_trust = json.loads((root / "trust.json").read_text())
future_trust["keys"][1]["not_before_ms"] = trust_meta["now"] + 1000
(root / "future-trust.json").write_text(json.dumps(future_trust))
future = module.load_trust_bundle(root / "future-trust.json")
try:
    module.require_trusted_key_active(
        future["observer-key"],
        signed_at_ms=trust_meta["now"],
        observed_at_ms=trust_meta["now"],
        label="future observer",
    )
except ValueError as error:
    assert "was not valid at signed time" in str(error)
else:
    raise AssertionError("not-yet-active rotation key was accepted")

trust["keys"][1]["roles"] = ["campaign_authority", "observer_runner"]
(root / "dual-role-trust.json").write_text(json.dumps(trust))
try:
    module.load_trust_bundle(root / "dual-role-trust.json")
except ValueError as error:
    assert "must not combine authority roles" in str(error)
else:
    raise AssertionError("one key was allowed to control campaign and observation")

try:
    module.validate_system_authority_path(
        root / "trust.json", directory=False, label="test trust"
    )
except ValueError as error:
    assert any(
        fragment in str(error)
        for fragment in ("owned by root", "group/other writable", "traverse a symlink")
    )
else:
    raise AssertionError("caller-owned trust path was accepted as system authority")
PY

python3 - "$SCRIPT" "$TMP_DIR" <<'PY'
import importlib.util
import json
import pathlib
import sys
import time

script = pathlib.Path(sys.argv[1])
root = pathlib.Path(sys.argv[2])
spec = importlib.util.spec_from_file_location("provenance", script)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
verification = json.loads((root / "verified-campaign.json").read_text())
try:
    module.reserve_campaign_replay(root / "pending-ledger", verification, int(time.time() * 1000))
except ValueError as error:
    assert "only after all Axon receipts are verified" in str(error)
else:
    raise AssertionError("attestation-only campaign consumed replay state")

verification["status"] = "verified"
verification["all_receipts_verified"] = True
ledger = root / "replay-ledger"
module.reserve_campaign_replay(ledger, verification, int(time.time() * 1000))
try:
    module.reserve_campaign_replay(ledger, verification, int(time.time() * 1000))
except ValueError as error:
    assert "already been consumed" in str(error)
else:
    raise AssertionError("signed campaign replay was accepted")
PY

printf '%s\n' '{"status":"tampered"}' >"$TMP_DIR/campaign-root/domain-report.json"
if python3 "$SCRIPT" verify-campaign \
    --bundle "$TMP_DIR/bundle.json" \
    --trust-bundle "$TMP_DIR/trust.json" \
    --campaign-root "$TMP_DIR/campaign-root" \
    --report "domain=$TMP_DIR/campaign-root/domain-report.json" \
    >"$TMP_DIR/tamper.stdout" 2>"$TMP_DIR/tamper.stderr"; then
  echo "signed campaign accepted a tampered report" >&2
  exit 1
fi
grep -q "mismatch" "$TMP_DIR/tamper.stderr"

echo "test_remoteapp_evidence_provenance: ok"

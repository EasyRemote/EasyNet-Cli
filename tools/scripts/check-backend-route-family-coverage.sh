#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
DEFAULT_MANIFEST="$REPO_ROOT/sdk/conformance/backend-route-family-coverage.json"

run_validator() {
  local manifest="$1"
  python3 - "$REPO_ROOT" "$manifest" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path


repo = Path(sys.argv[1]).resolve()
manifest_path = Path(sys.argv[2]).resolve()

CASE_ID = "backend/hub_route_family_coverage"
SPEC_REF = "docs/spec/daemon-sdk-requirements-v1.md#29.2"

REQUIRED = {
    "health_readiness_liveness": {
        "route_family": "/health, daemon readiness, runtime liveness",
        "sdk_clients": ["HealthClient", "DaemonHandle", "RuntimeClient"],
        "sdk_profile_refs": ["health", "daemon_lifecycle", "runtime_core"],
    },
    "identity_prepare_submit": {
        "route_family": "User signing keys, local identity, prepare/submit handoff",
        "sdk_clients": ["IdentityClient", "RuntimeClient", "PreparedInvocation", "SignedInvocation"],
        "sdk_profile_refs": ["identity", "runtime_core", "signing"],
    },
    "pairing_credentials_revoke": {
        "route_family": "Device pairing preflight/validate, credential verification, device revoke",
        "sdk_clients": ["AdminClient", "IdentityClient", "DirectoryClient"],
        "sdk_profile_refs": ["admin_gateway", "identity", "directory"],
    },
    "device_agent_ability_catalog": {
        "route_family": "Device, agent, and ability catalog routes",
        "sdk_clients": ["DirectoryClient", "PublicationClient"],
        "sdk_profile_refs": ["directory", "publication"],
    },
    "device_sessions_gateway_lifecycle": {
        "route_family": "Device sessions and gateway/agent lifecycle",
        "sdk_clients": ["AdminClient", "RuntimeClient", "BidiSession"],
        "sdk_profile_refs": ["admin_gateway", "runtime_core", "bidi"],
    },
    "ability_invoke_stream_bidi": {
        "route_family": "Ability invoke, signed invoke, stream, signed bidi bridge",
        "sdk_clients": ["RuntimeClient", "InvocationBuilder", "StreamHandle", "BidiSession", "ReceiptClient"],
        "sdk_profile_refs": ["runtime_core", "stream", "bidi", "receipt"],
    },
    "directory_session_invocation_events": {
        "route_family": "Directory, device, session, invocation, and SSE events",
        "sdk_clients": ["EventClient", "DirectoryClient.subscribe"],
        "sdk_profile_refs": ["events", "directory"],
    },
    "file_transfer_context_upload": {
        "route_family": "File transfer and context-file upload",
        "sdk_clients": ["Convenience file wrappers", "RuntimeClient", "ReceiptClient"],
        "sdk_profile_refs": ["wrappers", "runtime_core", "receipt"],
    },
    "interactive_media_bridges": {
        "route_family": "Terminal, remote desktop, browser session, voice/media bridges",
        "sdk_clients": ["Convenience wrappers over BidiSession/StreamHandle"],
        "sdk_profile_refs": ["wrappers", "stream", "bidi"],
    },
    "pages_surface_manifests": {
        "route_family": "Page create/list/delete, public pages, surface manifests",
        "sdk_clients": ["SurfaceClient", "DirectoryClient", "HealthClient"],
        "sdk_profile_refs": ["surface", "directory", "health"],
    },
    "skill_plugin_lifecycle": {
        "route_family": "Skill/plugin install/list/remove/upgrade/file tree",
        "sdk_clients": ["PublicationClient", "DirectoryClient"],
        "sdk_profile_refs": ["publication", "directory"],
    },
    "openai_compatibility": {
        "route_family": "OpenAI-compatible models/chat/files",
        "sdk_clients": ["CompatibilityClient", "file wrappers", "RuntimeClient", "DirectoryClient"],
        "sdk_profile_refs": ["compatibility", "wrappers", "runtime_core", "directory"],
    },
    "receipts_history_metrics": {
        "route_family": "Call history, receipts, failure location, metrics",
        "sdk_clients": ["ReceiptClient", "EventClient", "HealthClient"],
        "sdk_profile_refs": ["receipt", "events", "health"],
    },
    "federation_peer_hubs_remote_devices": {
        "route_family": "Federation, peer hubs, remote devices",
        "sdk_clients": ["AdminClient", "DirectoryClient", "RuntimeClient"],
        "sdk_profile_refs": ["admin_gateway", "directory", "runtime_core"],
    },
}

FORBIDDEN_BACKEND_RESPONSIBILITY = (
    "raw axon",
    "generated axon",
    "protobuf",
    "daemon transport",
    "direct daemon",
    "raw socket",
    "direct socket",
    "c abi",
    "ffi",
    "subprocess",
    "easyremote",
    "invocation codec",
    "receipt semantics",
)

ALLOWED_EVIDENCE_KINDS = {"sdk_conformance_case", "static_gate", "backend_smoke"}


def fail(message: str) -> None:
    print(f"backend_route_family_coverage: {message}", file=sys.stderr)
    raise SystemExit(1)


def require_string(value: object, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        fail(f"missing_or_empty_{field}")
    return value.strip()


def require_string_list(value: object, field: str) -> list[str]:
    if not isinstance(value, list) or not value:
        fail(f"missing_or_empty_{field}")
    result: list[str] = []
    for item in value:
        if not isinstance(item, str) or not item.strip():
            fail(f"invalid_{field}")
        result.append(item.strip())
    if len(set(result)) != len(result):
        fail(f"duplicate_{field}")
    return result


def evidence_exists(ref: str) -> bool:
    path = (repo / ref).resolve()
    try:
        path.relative_to(repo)
    except ValueError:
        return False
    return path.exists()


if not manifest_path.exists():
    fail(f"manifest_not_found:{manifest_path}")

try:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
except json.JSONDecodeError as exc:
    fail(f"invalid_json:{exc}")

if manifest.get("schema_version") != 1:
    fail("schema_version_must_be_1")
if manifest.get("case_id") != CASE_ID:
    fail("case_id_mismatch")
if manifest.get("source_spec") != SPEC_REF:
    fail("source_spec_mismatch")

families = manifest.get("families")
if not isinstance(families, list):
    fail("missing_families")
if len(families) != len(REQUIRED):
    fail(f"route_family_count:{len(families)}_want_{len(REQUIRED)}")

seen: set[str] = set()
for family in families:
    if not isinstance(family, dict):
        fail("invalid_family_entry")
    family_id = require_string(family.get("family_id"), "family_id")
    if family_id in seen:
        fail(f"duplicate_route_family:{family_id}")
    seen.add(family_id)
    if family_id not in REQUIRED:
        fail(f"unknown_route_family:{family_id}")

    required = REQUIRED[family_id]
    route_family = require_string(family.get("route_family"), f"{family_id}.route_family")
    if route_family != required["route_family"]:
        fail(f"route_family_text_mismatch:{family_id}")

    sdk_clients = require_string_list(family.get("sdk_clients"), f"{family_id}.sdk_clients")
    if sdk_clients != required["sdk_clients"]:
        fail(f"sdk_clients_mismatch:{family_id}")

    sdk_profile_refs = require_string_list(family.get("sdk_profile_refs"), f"{family_id}.sdk_profile_refs")
    if sdk_profile_refs != required["sdk_profile_refs"]:
        fail(f"sdk_profile_refs_mismatch:{family_id}")
    responsibility = require_string(family.get("backend_responsibility"), f"{family_id}.backend_responsibility")
    lower_responsibility = responsibility.lower()
    for marker in FORBIDDEN_BACKEND_RESPONSIBILITY:
        if marker in lower_responsibility:
            fail(f"backend_local_runtime_ownership:{family_id}:{marker}")

    evidence = family.get("coverage_evidence")
    if not isinstance(evidence, list) or not evidence:
        fail(f"missing_coverage_evidence:{family_id}")
    for index, item in enumerate(evidence):
        if not isinstance(item, dict):
            fail(f"invalid_coverage_evidence:{family_id}:{index}")
        kind = require_string(item.get("kind"), f"{family_id}.coverage_evidence.kind")
        ref = require_string(item.get("ref"), f"{family_id}.coverage_evidence.ref")
        if kind not in ALLOWED_EVIDENCE_KINDS:
            fail(f"unknown_coverage_evidence_kind:{family_id}:{kind}")
        if kind in {"sdk_conformance_case", "static_gate"} and not evidence_exists(ref):
            fail(f"missing_coverage_evidence_ref:{family_id}:{ref}")

missing = sorted(set(REQUIRED) - seen)
if missing:
    fail("missing_route_family:" + ",".join(missing))

print(f"backend route-family coverage ok: {manifest_path}")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  cp "$DEFAULT_MANIFEST" "$tmp/good.json"
  run_validator "$tmp/good.json" >/dev/null

  python3 - "$DEFAULT_MANIFEST" "$tmp/missing.json" "$tmp/duplicate.json" "$tmp/ownership.json" "$tmp/profile_refs.json" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path


source = Path(sys.argv[1])
missing = Path(sys.argv[2])
duplicate = Path(sys.argv[3])
ownership = Path(sys.argv[4])
profile_refs = Path(sys.argv[5])

manifest = json.loads(source.read_text(encoding="utf-8"))

without_family = json.loads(json.dumps(manifest))
without_family["families"] = without_family["families"][:-1]
missing.write_text(json.dumps(without_family), encoding="utf-8")

with_duplicate = json.loads(json.dumps(manifest))
with_duplicate["families"][-1] = with_duplicate["families"][0]
duplicate.write_text(json.dumps(with_duplicate), encoding="utf-8")

with_ownership = json.loads(json.dumps(manifest))
with_ownership["families"][0]["backend_responsibility"] = "Own direct daemon transport for health."
ownership.write_text(json.dumps(with_ownership), encoding="utf-8")

with_profile_ref_mismatch = json.loads(json.dumps(manifest))
with_profile_ref_mismatch["families"][0]["sdk_profile_refs"] = ["runtime_core"]
profile_refs.write_text(json.dumps(with_profile_ref_mismatch), encoding="utf-8")
PY

  if run_validator "$tmp/missing.json" >"$tmp/missing.out" 2>&1; then
    echo "self-test expected missing family fixture to fail" >&2
    exit 1
  fi
  grep -Eq "route_family_count|missing_route_family" "$tmp/missing.out"

  if run_validator "$tmp/duplicate.json" >"$tmp/duplicate.out" 2>&1; then
    echo "self-test expected duplicate family fixture to fail" >&2
    exit 1
  fi
  grep -Fq "duplicate_route_family" "$tmp/duplicate.out"

  if run_validator "$tmp/ownership.json" >"$tmp/ownership.out" 2>&1; then
    echo "self-test expected ownership fixture to fail" >&2
    exit 1
  fi
  grep -Fq "backend_local_runtime_ownership" "$tmp/ownership.out"

  if run_validator "$tmp/profile_refs.json" >"$tmp/profile_refs.out" 2>&1; then
    echo "self-test expected profile-ref fixture to fail" >&2
    exit 1
  fi
  grep -Fq "sdk_profile_refs_mismatch" "$tmp/profile_refs.out"

  echo "check-backend-route-family-coverage self-test ok"
  exit 0
fi

run_validator "${1:-$DEFAULT_MANIFEST}"

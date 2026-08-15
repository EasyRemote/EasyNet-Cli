#!/usr/bin/env bash
# Host-side remoteapp permission subject E2E.
#
# Boundary:
# - This script proves SPEC E2E-02 at the public descriptor-bound Invocation
#   boundary: host-local permission probes use a descriptor-bound user invoke
#   Resource subject, and display/window/application target Resource subjects
#   fail closed as invalid_argument before any OS permission prompt.
# - It does not validate target capture or decoded pixels. Media isolation is
#   owned by host-remoteapp-decoded-frame-e2e.sh.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

MODE=run
OUT_DIR=""

HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA='easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject'

usage() {
  cat <<'USAGE'
Usage:
  host-remoteapp-permission-subject-e2e.sh --run
  host-remoteapp-permission-subject-e2e.sh --self-test

Options:
  --run          Execute against the local EasyNet daemon.
  --self-test    Validate the harness against synthetic positive evidence.
  --out-dir DIR  Report directory. Defaults under target/e2e.

Environment:
  EASYNET_REMOTEAPP_EASYNET_BIN
                 Optional easynet binary override.
  EASYNET_REMOTEAPP_PERMISSION_USER_URA
                 Optional caller User URA override. Defaults to the unique
                 role="user" entry in ~/.easynet/realm-trust.toml.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

TIMESTAMP="$(date -u +%Y%m%d-%H%M%S)"
if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$REPO_ROOT/target/e2e/host-remoteapp-permission-subject/$TIMESTAMP-$$"
fi
mkdir -p "$OUT_DIR"

EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
ABILITY_LIST_JSON="$OUT_DIR/ability-list.json"
RUNTIME_STATUS_JSON="$OUT_DIR/runtime-status.json"
POSITIVE_STDOUT="$OUT_DIR/permission-status-positive.stdout"
POSITIVE_STDERR="$OUT_DIR/permission-status-positive.stderr"

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

run_easynet() {
  if [[ -n "${EASYNET_REMOTEAPP_EASYNET_BIN:-}" ]]; then
    "$EASYNET_REMOTEAPP_EASYNET_BIN" "$@"
  elif [[ -x "$REPO_ROOT/target/debug/easynet" ]]; then
    "$REPO_ROOT/target/debug/easynet" "$@"
  else
    need_cmd cargo
    cargo run --quiet --bin easynet -- "$@"
  fi
}

random_nonce_hex() {
  python3 - <<'PY'
import secrets
print(secrets.token_hex(16))
PY
}

validate_evidence() {
  python3 - "$EVIDENCE_JSON" "$REPORT_JSON" "$REPORT_MD" "$HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA" <<'PY'
import json
import pathlib
import sys

evidence_path, report_path, md_path, contract_ura = sys.argv[1:5]
with open(evidence_path, encoding="utf-8") as f:
    evidence = json.load(f)

errors = []

def require(condition, message):
    if not condition:
        errors.append(message)

def get(path, default=None):
    value = evidence
    for part in path.split("."):
        if not isinstance(value, dict) or part not in value:
            return default
        value = value[part]
    return value

catalog = evidence.get("catalog")
require(isinstance(catalog, dict), "catalog evidence must be an object")
if isinstance(catalog, dict):
    for ability in ["remote_desktop.permission_status", "remote_desktop.request_permission"]:
        row = catalog.get(ability)
        require(isinstance(row, dict), f"catalog must include {ability}")
        if isinstance(row, dict):
            require(
                row.get("subject_contract_ura") == contract_ura,
                f"{ability} must publish the host-local subject contract URA",
            )
            require(row.get("owner_ura", "").startswith("easynet:///r/"),
                    f"{ability} must record its descriptor owner URA")
            scope = row.get("scope_subjects")
            require(isinstance(scope, list), f"{ability} must record scope_subjects")
            if isinstance(scope, list):
                for kind in ["agent", "resource", "user"]:
                    require(kind in scope, f"{ability} scope_subjects must include {kind}")

positive = evidence.get("positive_permission_status")
require(isinstance(positive, dict), "positive permission_status evidence must be an object")
if isinstance(positive, dict):
    subject = positive.get("subject_ura")
    response = positive.get("response")
    require(positive.get("ability") == "remote_desktop.permission_status",
            "positive probe must invoke remote_desktop.permission_status")
    require(positive.get("exit_code") == 0, "positive descriptor-bound permission_status must succeed")
    require(isinstance(subject, str) and "/resource/user." in subject and subject.endswith("/invoke/remote_desktop.permission_status"),
            "positive permission_status subject must be a descriptor-bound user invoke Resource")
    require(isinstance(response, dict), "positive permission_status stdout must decode to JSON")
    if isinstance(response, dict):
        contract = response.get("subject_contract")
        require(isinstance(contract, dict), "positive permission_status response must include subject_contract")
        if isinstance(contract, dict):
            require(contract.get("subject_contract_ura") == contract_ura,
                    "positive permission_status response must return the compiled host-local contract URA")
            require(contract.get("target_resource_subjects_allowed") is False,
                    "positive permission_status response must forbid target Resource subjects")
            allowed = contract.get("allowed_subjects")
            require(isinstance(allowed, list), "positive permission_status contract must list allowed subjects")
            if isinstance(allowed, list):
                require("descriptor_bound_invoke_resource" in allowed,
                        "positive permission_status contract must allow descriptor-bound invoke Resource subjects")

negative = evidence.get("negative_target_subjects")
require(isinstance(negative, list), "negative_target_subjects evidence must be a list")
if isinstance(negative, list):
    expected = {
        ("remote_desktop.permission_status", "display"),
        ("remote_desktop.permission_status", "window"),
        ("remote_desktop.permission_status", "application"),
        ("remote_desktop.request_permission", "display"),
        ("remote_desktop.request_permission", "window"),
        ("remote_desktop.request_permission", "application"),
    }
    seen = set()
    for item in negative:
        require(isinstance(item, dict), "negative target-subject item must be an object")
        if not isinstance(item, dict):
            continue
        key = (item.get("ability"), item.get("target_kind"))
        seen.add(key)
        subject = item.get("subject_ura")
        stderr = item.get("stderr", "")
        require(key in expected, f"unexpected negative permission probe case {key!r}")
        require(item.get("exit_code") not in (None, 0),
                f"{key} target Resource subject must fail")
        require(isinstance(subject, str) and f"/streams/{item.get('target_kind')}." in subject,
                f"{key} subject must be the selected display/window/application target Resource")
        require("invalid_argument" in stderr or "INVALID_ARGUMENT" in stderr,
                f"{key} must fail as invalid_argument")
        require("MUST NOT be scoped" in stderr,
                f"{key} must explain that host-local permission probes cannot be target-scoped")
        require("AUTHORITY_REQUIRED" not in stderr,
                f"{key} must not be misclassified as missing runtime authority")
    require(seen == expected, f"negative target-subject cases mismatch: got {sorted(seen)!r}")

report = {
    "status": "failed" if errors else "passed",
    "errors": errors,
    "evidence_json": evidence_path,
    "positive_subject_ura": get("positive_permission_status.subject_ura"),
    "negative_case_count": len(negative) if isinstance(negative, list) else None,
}
pathlib.Path(report_path).write_text(
    json.dumps(report, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp permission subject E2E report\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Evidence: `{evidence_path}`\n")
    f.write(f"- Positive subject: `{report['positive_subject_ura']}`\n")
    f.write(f"- Negative target-subject cases: `{report['negative_case_count']}`\n")
    if errors:
        f.write("\n## Errors\n")
        for error in errors:
            f.write(f"- {error}\n")
if errors:
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)
PY
}

if [[ "$MODE" == "self-test" ]]; then
  python3 - "$EVIDENCE_JSON" "$HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
contract_ura = sys.argv[2]
user = "easynet:///r/localhost/user/alice"
device = "dev-a"
subject = "easynet:///r/localhost/resource/user.alice/invoke/remote_desktop.permission_status"
catalog_row = {
    "owner_ura": "easynet:///r/localhost/agent/device.dev-a.remote-desktop",
    "subject_contract_ura": contract_ura,
    "scope_subjects": ["agent", "resource", "user"],
}
negative = []
for ability in ["remote_desktop.permission_status", "remote_desktop.request_permission"]:
    for kind in ["display", "window", "application"]:
        negative.append({
            "ability": ability,
            "target_kind": kind,
            "subject_ura": f"easynet:///r/localhost/resource/device.{device}/streams/{kind}.target",
            "exit_code": 1,
            "stderr": f"{ability}: screen-capture permission probes are host-local and MUST NOT be scoped to a remote desktop resource subject; reason=invalid_argument",
        })
evidence = {
    "status": "passed",
    "caller_user_ura": user,
    "catalog": {
        "remote_desktop.permission_status": catalog_row,
        "remote_desktop.request_permission": catalog_row,
    },
    "positive_permission_status": {
        "ability": "remote_desktop.permission_status",
        "subject_ura": subject,
        "exit_code": 0,
        "response": {
            "permission": "screen_capture",
            "subject_contract": {
                "subject_contract_ura": contract_ura,
                "allowed_subjects": [
                    "caller_user_self",
                    "descriptor_bound_invoke_resource",
                    "local_system_loopback",
                ],
                "target_resource_subjects_allowed": False,
            },
        },
    },
    "negative_target_subjects": negative,
}
path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "host-remoteapp-permission-subject-e2e self-test ok"
  exit 0
fi

need_cmd python3

run_easynet runtime status --json >"$RUNTIME_STATUS_JSON"
run_easynet ability list --format json --pattern 'remote_desktop.*' >"$ABILITY_LIST_JSON"

python3 - "$ABILITY_LIST_JSON" "$HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA" <<'PY' >"$OUT_DIR/ability-env.sh"
import json
import shlex
import sys

ability_list_path, contract_ura = sys.argv[1:3]
with open(ability_list_path, encoding="utf-8") as f:
    rows = json.load(f)
selected = {}
for row in rows:
    name = row.get("name")
    if name in {"remote_desktop.permission_status", "remote_desktop.request_permission"}:
        metadata = row.get("metadata") if isinstance(row.get("metadata"), dict) else {}
        scope = row.get("scope_subjects") if isinstance(row.get("scope_subjects"), dict) else {}
        selected[name] = {
            "descriptor_ref": row.get("descriptor_ref"),
            "owner_ura": row.get("owner_ura"),
            "subject_contract_ura": metadata.get("subject_contract_ura"),
            "scope_subjects": scope.get("uras"),
        }
for name in ["remote_desktop.permission_status", "remote_desktop.request_permission"]:
    row = selected.get(name)
    if not isinstance(row, dict):
        raise SystemExit(f"{name} not found in ability catalog")
    if row.get("subject_contract_ura") != contract_ura:
        raise SystemExit(f"{name} missing host-local subject contract URA")
    prefix = "PERMISSION_STATUS" if name.endswith("permission_status") else "REQUEST_PERMISSION"
    print(f"{prefix}_REF={shlex.quote(row['descriptor_ref'])}")
    print(f"{prefix}_OWNER={shlex.quote(row['owner_ura'])}")
PY
source "$OUT_DIR/ability-env.sh"

CALLER_USER_URA="${EASYNET_REMOTEAPP_PERMISSION_USER_URA:-}"
if [[ -z "$CALLER_USER_URA" ]]; then
  CALLER_USER_URA="$(python3 - <<'PY'
import pathlib
import re

trust = pathlib.Path.home() / ".easynet" / "realm-trust.toml"
text = trust.read_text(encoding="utf-8")
matches = re.findall(
    r'agent_ura\s*=\s*"(easynet:///r/[^"]+/user/[^"]+)"[\s\S]*?role\s*=\s*"user"',
    text,
)
unique = sorted(set(matches))
if len(unique) != 1:
    raise SystemExit(f"expected exactly one trusted User URA in {trust}, got {len(unique)}")
print(unique[0])
PY
)"
fi

REALM="$(python3 - "$CALLER_USER_URA" <<'PY'
import sys
parts = sys.argv[1].split("/")
if len(parts) < 6 or parts[3] != "r" or parts[5] != "user":
    raise SystemExit(f"not a canonical User URA: {sys.argv[1]}")
print(parts[4])
PY
)"
USER_ID="${CALLER_USER_URA##*/}"
DEVICE_ID="$(python3 - "$RUNTIME_STATUS_JSON" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as f:
    status = json.load(f)
print(status["connection"]["node_id"])
PY
)"

PERMISSION_STATUS_SUBJECT="easynet:///r/$REALM/resource/user.$USER_ID/invoke/remote_desktop.permission_status"
set +e
run_easynet ability invoke "$PERMISSION_STATUS_REF" \
  --node "$PERMISSION_STATUS_OWNER" \
  --subject "$PERMISSION_STATUS_SUBJECT" \
  --args '{}' \
  --causal-root \
  --nonce-hex "$(random_nonce_hex)" \
  --timeout 5 \
  --raw >"$POSITIVE_STDOUT" 2>"$POSITIVE_STDERR"
POSITIVE_EXIT_CODE=$?
set -e

NEGATIVE_CASES_JSON="$OUT_DIR/negative-cases.json"
printf '[\n' >"$NEGATIVE_CASES_JSON"
first=1
for ability in remote_desktop.permission_status remote_desktop.request_permission; do
  if [[ "$ability" == "remote_desktop.permission_status" ]]; then
    ref="$PERMISSION_STATUS_REF"
    owner="$PERMISSION_STATUS_OWNER"
  else
    ref="$REQUEST_PERMISSION_REF"
    owner="$REQUEST_PERMISSION_OWNER"
  fi
  for kind in display window application; do
    subject="easynet:///r/$REALM/resource/device.$DEVICE_ID/streams/$kind.e2e-permission-subject"
    stdout_path="$OUT_DIR/${ability//./-}-$kind.stdout"
    stderr_path="$OUT_DIR/${ability//./-}-$kind.stderr"
    set +e
    run_easynet ability invoke "$ref" \
      --node "$owner" \
      --subject "$subject" \
      --args '{}' \
      --causal-root \
      --nonce-hex "$(random_nonce_hex)" \
      --timeout 5 \
      --raw >"$stdout_path" 2>"$stderr_path"
    exit_code=$?
    set -e
    [[ "$first" == "1" ]] || printf ',\n' >>"$NEGATIVE_CASES_JSON"
    first=0
    python3 - "$NEGATIVE_CASES_JSON" "$ability" "$kind" "$subject" "$exit_code" "$stdout_path" "$stderr_path" <<'PY'
import json
import pathlib
import sys

out_path, ability, kind, subject, exit_code, stdout_path, stderr_path = sys.argv[1:8]
item = {
    "ability": ability,
    "target_kind": kind,
    "subject_ura": subject,
    "exit_code": int(exit_code),
    "stdout": pathlib.Path(stdout_path).read_text(errors="replace"),
    "stderr": pathlib.Path(stderr_path).read_text(errors="replace"),
}
with open(out_path, "a", encoding="utf-8") as f:
    f.write(json.dumps(item, indent=2, sort_keys=True))
PY
  done
done
printf '\n]\n' >>"$NEGATIVE_CASES_JSON"

python3 - "$EVIDENCE_JSON" "$ABILITY_LIST_JSON" "$RUNTIME_STATUS_JSON" "$CALLER_USER_URA" \
  "$PERMISSION_STATUS_SUBJECT" "$POSITIVE_EXIT_CODE" "$POSITIVE_STDOUT" "$POSITIVE_STDERR" \
  "$NEGATIVE_CASES_JSON" "$HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA" <<'PY'
import json
import pathlib
import sys

(
    evidence_path,
    ability_list_path,
    runtime_status_path,
    caller_user_ura,
    permission_status_subject,
    positive_exit_code,
    positive_stdout_path,
    positive_stderr_path,
    negative_cases_path,
    contract_ura,
) = sys.argv[1:11]

with open(ability_list_path, encoding="utf-8") as f:
    rows = json.load(f)
catalog = {}
for row in rows:
    name = row.get("name")
    if name in {"remote_desktop.permission_status", "remote_desktop.request_permission"}:
        metadata = row.get("metadata") if isinstance(row.get("metadata"), dict) else {}
        scope = row.get("scope_subjects") if isinstance(row.get("scope_subjects"), dict) else {}
        catalog[name] = {
            "owner_ura": row.get("owner_ura"),
            "descriptor_ref": row.get("descriptor_ref"),
            "subject_contract_ura": metadata.get("subject_contract_ura"),
            "scope_subjects": scope.get("uras"),
        }

positive_stdout = pathlib.Path(positive_stdout_path).read_text(errors="replace")
try:
    positive_response = json.loads(positive_stdout) if positive_stdout.strip() else None
except json.JSONDecodeError as error:
    positive_response = {"decode_error": str(error), "raw": positive_stdout}

with open(runtime_status_path, encoding="utf-8") as f:
    runtime_status = json.load(f)
with open(negative_cases_path, encoding="utf-8") as f:
    negative = json.load(f)

evidence = {
    "status": "passed",
    "caller_user_ura": caller_user_ura,
    "runtime": {
        "device_ura": runtime_status.get("connection", {}).get("device_ura"),
        "node_id": runtime_status.get("connection", {}).get("node_id"),
        "state": runtime_status.get("connection", {}).get("state"),
    },
    "catalog": catalog,
    "positive_permission_status": {
        "ability": "remote_desktop.permission_status",
        "subject_ura": permission_status_subject,
        "exit_code": int(positive_exit_code),
        "stdout": positive_stdout,
        "stderr": pathlib.Path(positive_stderr_path).read_text(errors="replace"),
        "response": positive_response,
    },
    "negative_target_subjects": negative,
    "contract_ura": contract_ura,
}
pathlib.Path(evidence_path).write_text(
    json.dumps(evidence, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

validate_evidence
echo "host-remoteapp-permission-subject-e2e ok: $REPORT_MD"

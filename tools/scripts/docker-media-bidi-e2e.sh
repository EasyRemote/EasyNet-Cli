#!/usr/bin/env bash
# Docker E2E for synthetic media stream and bidirectional multimodal transfer.
#
# Topology:
#   hub      - CLI-managed Hub daemon on TCP/TLS.
#   provider - joins the Hub, installs a user sidecar plugin, publishes
#              synthetic media stream/bidi abilities, and executes them through
#              the public CLI.
#   caller   - joins the Hub, verifies remote catalog visibility, and invokes
#              the provider's stream and bidi abilities through descriptor-bound
#              remote CLI public ingress.
#
# This is intentionally synthetic media. Docker CI cannot reliably access host
# camera/microphone/screen devices, but it can prove the canonical runtime data
# plane for audio/video/control frames without a test-only daemon bypass.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
WORKSPACE_ROOT="$(cd "$REPO_ROOT/.." && pwd)"
EASYNET_ROOT="${EASYNET_BACKEND_ROOT:-$WORKSPACE_ROOT/EasyNet}"

PROJECT="${EASYNET_E2E_PROJECT:-easynet-media-bidi}"
RUNTIME_IMAGE="${EASYNET_RUNTIME_IMAGE:-${EASYNET_HUB_IMAGE:-easynet/hub-e2e:local}}"
DOCKER_BIN="${DOCKER_BIN:-}"
REALM="${EASYNET_E2E_REALM:-hub}"
HUB_URA="easynet:///r/${REALM}/authority"
ADMIN_URA="easynet:///r/${REALM}/user/admin"
USER_URA="easynet:///r/${REALM}/user/alice"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="${EASYNET_E2E_OUT_DIR:-$REPO_ROOT/target/e2e/docker-media-bidi/$TIMESTAMP}"
KEEP=0
SKIP_BUILD=0
SELF_TEST=0

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/docker-media-bidi-e2e.sh [options]

Options:
  --skip-build       Reuse the configured runtime Docker image.
  --keep             Keep containers, volumes, and report files after completion.
  --project NAME     Docker Compose project name.
  --out-dir DIR      Report directory.
  --self-test        Validate script structure without Docker.
  -h, --help         Show this help.

Environment:
  EASYNET_RUNTIME_IMAGE      Runtime image containing easynet, easynet-daemon,
                             easynet-keyring, Python, and libeasynet_cli.so.
                             Defaults to easynet/hub-e2e:local.
  EASYNET_CLI_ARTIFACT_ROOT  Prebuilt Linux CLI runtime artifact bundle. When
                             unset, this E2E builds one with cargo zigbuild
                             before Docker image assembly.
  EASYNET_BACKEND_ROOT       Sibling EasyNet repo used to build images.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --keep) KEEP=1; shift ;;
    --project) PROJECT="${2:?missing value for --project}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --self-test) SELF_TEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

resolve_docker() {
  if [[ -n "${DOCKER_BIN:-}" ]]; then
    [[ -x "$DOCKER_BIN" ]] || die "DOCKER_BIN is not executable: $DOCKER_BIN"
    printf '%s\n' "$DOCKER_BIN"
    return 0
  fi
  local candidate
  for candidate in \
    docker \
    /usr/local/bin/docker \
    /opt/homebrew/bin/docker \
    /Applications/Docker.app/Contents/Resources/bin/docker
  do
    if [[ "$candidate" == */* ]]; then
      if [[ -x "$candidate" ]]; then
        printf '%s\n' "$candidate"
        return 0
      fi
    elif command -v "$candidate" >/dev/null 2>&1; then
      command -v "$candidate"
      return 0
    fi
  done
  die "missing command: docker"
}

extend_tool_path() {
  local dir
  for dir in \
    "$(dirname "$DOCKER_BIN")" \
    "$HOME/.cargo/bin" \
    /opt/homebrew/bin \
    /usr/local/bin \
    /usr/local/go/bin
  do
    if [[ -d "$dir" && ":$PATH:" != *":$dir:"* ]]; then
      PATH="$dir:$PATH"
    fi
  done
  export PATH
}

require_paths() {
  [[ -d "$EASYNET_ROOT" ]] || die "EasyNet root not found: $EASYNET_ROOT"
  [[ -x "$EASYNET_ROOT/scripts/docker-build-images.sh" ]] || die "missing EasyNet image build script"
}

ensure_runtime_dirs() {
  mkdir -p "$SHARED_DIR" "$CERT_DIR"
}

if [[ "$SELF_TEST" == "1" ]]; then
  bash -n "$0"
  require_paths
  grep -q "synthetic media stream and bidirectional multimodal transfer" "$0"
  grep -q "kind = \"sidecar\"" "$0"
  grep -q "media.synthetic_stream" "$0"
  grep -q "media.synthetic_bidi" "$0"
  grep -q "transport = \"webrtc\"" "$0"
  ! grep -q "fallback_""transport" "$0"
  grep -q "ability stream" "$0"
  grep -q "ability bidi" "$0"
  grep -q "caller_remote_media_stream_succeeded" "$0"
  grep -q "caller_remote_media_bidi_succeeded" "$0"
  grep -q "caller_media_bidi_descriptor_ref" "$0"
  grep -q "media_stream_unique_invocation_records" "$0"
  grep -q "media_bidi_unique_invocation_records" "$0"
  grep -q "media_product_operations_have_verified_single_terminal_receipt_chains" "$0"
  grep -q -- "--proof-ref 'bootstrap-admin-" "$0"
  grep -q "completed_chain_facts" "$0"
  grep -q "caller_cli_must_fail" "$0"
  grep -q "provider_media_plugin_removed" "$0"
  grep -q "provider_removed_media_routes_reject_invocation" "$0"
  grep -q "serve_plugin(" "$0"
  grep -q "sdk-python/easynet_sdk" "$0"
  grep -q "def candidates(row):" "$0"
  grep -q "ability_ura.endswith(f\".{ability_name}\")" "$0"
  grep -q "resolve_docker" "$0"
  grep -q "extend_tool_path" "$0"
  grep -q "ensure_runtime_dirs" "$0"
  grep -q "random_nonce_hex" "$0"
  grep -q "headless-media" "$SELF_DIR/build-linux-cli-artifact-bundle.sh"
  grep -q "wait_device_online provider /home/provider" "$0"
  grep -q "docker compose" "$0"
  echo "docker-media-bidi-e2e self-test ok"
  exit 0
fi

DOCKER_BIN="$(resolve_docker)"
extend_tool_path
need_cmd jq
need_cmd openssl
need_cmd python3
require_paths
"$DOCKER_BIN" info >/dev/null 2>&1 || die "Docker engine is not available"

mkdir -p "$OUT_DIR"
WORK_ROOT="$(mktemp -d "/tmp/easynet-media-bidi.XXXXXX")"
SHARED_DIR="$WORK_ROOT/shared"
CERT_DIR="$WORK_ROOT/certs"
ensure_runtime_dirs
COMPOSE_FILE="$OUT_DIR/docker-compose.yml"
printf '%s\n' "$WORK_ROOT" >"$OUT_DIR/work-root.txt"

cleanup() {
  local status="$1"
  if [[ "$status" -ne 0 ]]; then
    if declare -F dump_logs >/dev/null 2>&1; then
      dump_logs || true
    fi
  fi
  if [[ "$KEEP" != "1" ]]; then
    if [[ -f "$COMPOSE_FILE" ]]; then
      "$DOCKER_BIN" compose -p "$PROJECT" -f "$COMPOSE_FILE" down -v --remove-orphans >/dev/null 2>&1 || true
    fi
    rm -rf "$WORK_ROOT"
  else
    echo "kept work root: $WORK_ROOT" >&2
    echo "kept report dir: $OUT_DIR" >&2
  fi
}
trap 'cleanup $?' EXIT

if [[ "$SKIP_BUILD" != "1" ]]; then
  CLI_ARTIFACT_ROOT="${EASYNET_CLI_ARTIFACT_ROOT:-}"
  if [[ -z "$CLI_ARTIFACT_ROOT" ]]; then
    CLI_ARTIFACT_ROOT="$WORK_ROOT/cli-artifacts"
    echo "==> building Linux CLI artifact bundle"
    CARGO_BIN="${CARGO_BIN:-}" "$SELF_DIR/build-linux-cli-artifact-bundle.sh" \
      --out-dir "$CLI_ARTIFACT_ROOT"
  else
    echo "==> using caller-provided Linux CLI artifact bundle: $CLI_ARTIFACT_ROOT"
  fi
  echo "==> building Docker runtime images"
  EASYNET_CLI_ARTIFACT_ROOT="$CLI_ARTIFACT_ROOT" \
    EASYNET_HUB_IMAGE="$RUNTIME_IMAGE" \
    "$EASYNET_ROOT/scripts/docker-build-images.sh"
fi
ensure_runtime_dirs

compose() {
  "$DOCKER_BIN" compose -p "$PROJECT" -f "$COMPOSE_FILE" "$@"
}

service_exec() {
  [[ $# -ge 2 ]] || die "service_exec requires <service> and <command>"
  local service="$1"
  shift
  local command="${*:-}"
  [[ -n "$command" ]] || die "service_exec command must not be empty"
  compose exec -T "$service" sh -lc "$command"
}

hub_cli() {
  [[ $# -gt 0 ]] || die "hub_cli requires a command"
  service_exec hub "HOME=/srv/easynet EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so easynet ${*:-}"
}

provider_cli() {
  [[ $# -gt 0 ]] || die "provider_cli requires a command"
  service_exec provider "HOME=/home/provider EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so easynet ${*:-}"
}

caller_cli() {
  [[ $# -gt 0 ]] || die "caller_cli requires a command"
  service_exec caller "HOME=/home/caller EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so easynet ${*:-}"
}

caller_cli_must_fail() {
  [[ $# -eq 3 ]] || die "caller_cli_must_fail requires <command> <stdout> <stderr>"
  local command="$1"
  local stdout="$2"
  local stderr="$3"
  set +e
  service_exec caller "HOME=/home/caller EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so timeout 20s easynet ${command}" \
    >"$stdout" 2>"$stderr"
  local status=$?
  set -e
  printf '%s\n' "$status" >"${stderr}.exit_code"
  if [[ "$status" -eq 0 ]]; then
    cat "$stdout" >&2 2>/dev/null || true
    cat "$stderr" >&2 2>/dev/null || true
    die "expected caller CLI command to fail after plugin removal: easynet ${command}"
  fi
}

dump_logs() {
  echo "==> docker compose logs" >&2
  if [[ -f "$COMPOSE_FILE" ]]; then
    compose logs --no-color hub provider caller >&2 || true
  else
    echo "compose file has not been generated: $COMPOSE_FILE" >&2
  fi
  echo "==> daemon logs" >&2
  service_exec hub "find /srv/easynet/.easynet -maxdepth 4 -type f -name '*.log' -print -exec tail -120 {} \\;" >&2 || true
  service_exec provider "find /home/provider/.easynet -maxdepth 4 -type f -name '*.log' -print -exec tail -180 {} \\;" >&2 || true
  service_exec caller "find /home/caller/.easynet -maxdepth 4 -type f -name '*.log' -print -exec tail -180 {} \\;" >&2 || true
}

wait_runtime() {
  local service="$1"
  local home="$2"
  local label="$3"
  for _ in $(seq 1 120); do
    if service_exec "$service" "HOME='$home' EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so easynet runtime status --json" \
      >"$OUT_DIR/status-$label.latest.json" 2>"$OUT_DIR/status-$label.latest.err"; then
      if jq -e '(.runtime_status // "") != "stopped"' "$OUT_DIR/status-$label.latest.json" >/dev/null; then
        return 0
      fi
    fi
    sleep 0.5
  done
  cat "$OUT_DIR/status-$label.latest.json" >&2 2>/dev/null || true
  cat "$OUT_DIR/status-$label.latest.err" >&2 2>/dev/null || true
  return 1
}

wait_hub_port_from() {
  local service="$1"
  for _ in $(seq 1 80); do
    if service_exec "$service" "python3 - <<'PY'
import socket
with socket.create_connection(('hub', 50443), timeout=0.5):
    pass
PY" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  return 1
}

wait_device_online() {
  local service="$1"
  local home="$2"
  local label="$3"
  local node_id="$4"
  local out="$OUT_DIR/status-${label}-online.latest.json"
  local err="$OUT_DIR/status-${label}-online.latest.err"
  for _ in $(seq 1 120); do
    if service_exec "$service" "HOME='$home' EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so easynet status --json" \
      >"$out" 2>"$err"; then
      if jq -e --arg node "$node_id" '
        .runtime_status == "running"
        and (.connection.node_id // "") == $node
        and (.connection.state_code // "") == "J800"
        and (.product_presence.directory_status // "") == "online"
        and (.product_presence.session_admitted // false) == true
      ' "$out" >/dev/null; then
        return 0
      fi
    fi
    sleep 0.5
  done
  cat "$out" >&2 2>/dev/null || true
  cat "$err" >&2 2>/dev/null || true
  return 1
}

extract_ability_field_by_name() {
  local file="$1"
  local ability_name="$2"
  local field="$3"
  python3 - "$file" "$ability_name" "$field" <<'PY'
import json
import sys

path, expected, field = sys.argv[1:4]
payload = json.load(open(path, encoding="utf-8"))
rows = payload if isinstance(payload, list) else payload.get("abilities") or payload.get("items") or payload.get("records") or []

def candidates(row):
    values = {
        str(row.get("name") or ""),
        str(row.get("ability_name") or ""),
        str(row.get("public_name") or ""),
        str(row.get("qualified_name") or ""),
    }
    owner = str(row.get("owner_ura") or "")
    if "/agent/" in owner:
        owner_tail = owner.rsplit("/agent/", 1)[1]
        name = str(row.get("name") or "")
        if name:
            values.add(f"{owner_tail}.{name}")
            if "." in owner_tail:
                values.add(f"{owner_tail.split('.', 1)[1]}.{name}")
    return values

def matches(row):
    ability_ura = str(row.get("ability_ura") or "")
    return expected in candidates(row) or ability_ura.endswith(f".{expected}")

for row in rows:
    if isinstance(row, dict) and matches(row):
        value = str(row.get(field) or "")
        if value:
            print(value)
            raise SystemExit(0)
PY
}

extract_ability_ura_by_name() {
  extract_ability_field_by_name "$1" "$2" "ability_ura"
}

extract_ability_descriptor_ref_by_name() {
  local file="$1"
  local ability_name="$2"
  local action="${3:-}"
  python3 - "$file" "$ability_name" "$action" <<'PY'
import json
import sys

path, ability_name, action = sys.argv[1:4]
with open(path, "r", encoding="utf-8") as fh:
    data = json.load(fh)
if isinstance(data, dict):
    rows = data.get("abilities") or data.get("items") or []
elif isinstance(data, list):
    rows = data
else:
    rows = []

def candidates(row):
    values = {
        str(row.get("name") or ""),
        str(row.get("ability_name") or ""),
        str(row.get("public_name") or ""),
        str(row.get("qualified_name") or ""),
    }
    owner = str(row.get("owner_ura") or "")
    if "/agent/" in owner:
        owner_tail = owner.rsplit("/agent/", 1)[1]
        name = str(row.get("name") or "")
        if name:
            values.add(f"{owner_tail}.{name}")
            if "." in owner_tail:
                values.add(f"{owner_tail.split('.', 1)[1]}.{name}")
    return values

def matches(row):
    ability_ura = str(row.get("ability_ura") or "")
    return ability_name in candidates(row) or ability_ura.endswith(f".{ability_name}")

for row in rows:
    if not isinstance(row, dict):
        continue
    if not matches(row):
        continue
    ref = str(row.get("descriptor_ref") or "")
    if not ref:
        continue
    if action and not ref.endswith("!" + action):
        continue
    print(ref)
    raise SystemExit(0)
PY
}

wait_ability_name() {
  local cli_func="$1"
  local node_arg="$2"
  local ability_name="$3"
  local stem="$4"
  local out="$OUT_DIR/${stem}.json"
  local err="$OUT_DIR/${stem}.err"
  local ability_ura=""
  for _ in $(seq 1 120); do
    if [[ -n "$node_arg" ]]; then
      "$cli_func" "ability list --node '$node_arg' --format json" >"$out" 2>"$err" || true
    else
      "$cli_func" "ability list --format json" >"$out" 2>"$err" || true
    fi
    if [[ -s "$out" ]]; then
      ability_ura="$(extract_ability_ura_by_name "$out" "$ability_name")"
      if [[ -n "$ability_ura" ]]; then
        printf '%s\n' "$ability_ura"
        return 0
      fi
    fi
    sleep 0.5
  done
  cat "$out" >&2 2>/dev/null || true
  cat "$err" >&2 2>/dev/null || true
  return 1
}

wait_ability_descriptor_ref() {
  local cli_func="$1"
  local node_arg="$2"
  local ability_name="$3"
  local stem="$4"
  local action="${5:-}"
  local out="$OUT_DIR/${stem}.json"
  local err="$OUT_DIR/${stem}.err"
  local descriptor_ref=""
  for _ in $(seq 1 120); do
    if [[ -n "$node_arg" ]]; then
      "$cli_func" "ability list --node '$node_arg' --format json" >"$out" 2>"$err" || true
    else
      "$cli_func" "ability list --format json" >"$out" 2>"$err" || true
    fi
    if [[ -s "$out" ]]; then
      descriptor_ref="$(extract_ability_descriptor_ref_by_name "$out" "$ability_name" "$action")"
      if [[ -n "$descriptor_ref" ]]; then
        printf '%s\n' "$descriptor_ref"
        return 0
      fi
    fi
    sleep 0.5
  done
  cat "$out" >&2 2>/dev/null || true
  cat "$err" >&2 2>/dev/null || true
  return 1
}

json_args() {
  python3 - "$@" <<'PY'
import json
import sys

kind = sys.argv[1]
if kind == "stream":
    print(json.dumps({
        "session_id": sys.argv[2],
        "audio_frames": 2,
        "video_frames": 2,
        "screen_frames": 1,
        "codec": "synthetic-json-b64",
    }, separators=(",", ":")))
elif kind == "bidi":
    print(json.dumps({
        "session_id": sys.argv[2],
        "expect_streams": ["audio", "video", "control"],
        "codec": "synthetic-json-b64",
    }, separators=(",", ":")))
else:
    raise SystemExit(f"unknown args kind: {kind}")
PY
}

json_frame() {
  python3 - "$@" <<'PY'
import base64
import json
import sys

kind, seq, body = sys.argv[1:4]
payload = base64.b64encode(body.encode("utf-8")).decode("ascii")
print(json.dumps({
    "kind": kind,
    "stream_id": {"audio": 1, "video": 2, "control": 3}[kind],
    "pts": int(seq) * 1000,
    "data_b64": payload,
    "body": body,
}, separators=(",", ":")))
PY
}

random_nonce_hex() {
  openssl rand -hex 16
}

issue_device_enrollment() {
  local label="$1"
  local out_file="$OUT_DIR/enrollment-${label}.json"
  hub_cli "principal issue-enrollment --issuer-ura '$ADMIN_URA' --subject-principal-ura '$USER_URA' --proof-ref '$ADMIN_BINDING' --idempotency-key ${label}-$TIMESTAMP --json" \
    >"$out_file"
  jq -r '.principal.enrollments[-1].enrollment_id' "$out_file"
}

cat >"$CERT_DIR/ca.cnf" <<'EOF'
[req]
distinguished_name = dn
x509_extensions = v3_ca
prompt = no
[dn]
CN = EasyNet Docker Media Bidi E2E CA
[v3_ca]
basicConstraints = critical, CA:true
keyUsage = critical, keyCertSign, cRLSign
EOF
cat >"$CERT_DIR/leaf.cnf" <<'EOF'
[req]
distinguished_name = dn
prompt = no
[dn]
CN = hub
[v3_leaf]
subjectAltName = @alt_names
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
[alt_names]
DNS.1 = hub
DNS.2 = localhost
EOF
openssl req -x509 -newkey rsa:2048 -nodes -days 3 \
  -keyout "$CERT_DIR/ca.key" \
  -out "$CERT_DIR/ca.crt" \
  -config "$CERT_DIR/ca.cnf" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes \
  -keyout "$CERT_DIR/hub.key" \
  -out "$CERT_DIR/hub.csr" \
  -config "$CERT_DIR/leaf.cnf" >/dev/null 2>&1
openssl x509 -req -days 3 \
  -in "$CERT_DIR/hub.csr" \
  -CA "$CERT_DIR/ca.crt" \
  -CAkey "$CERT_DIR/ca.key" \
  -CAcreateserial \
  -out "$CERT_DIR/hub.crt" \
  -extfile "$CERT_DIR/leaf.cnf" \
  -extensions v3_leaf >/dev/null 2>&1

cat >"$COMPOSE_FILE" <<YAML
services:
  hub:
    image: ${RUNTIME_IMAGE}
    hostname: hub
    entrypoint: ["/bin/sh", "-lc"]
    command: ["mkdir -p /srv/easynet /shared && tail -f /dev/null"]
    environment:
      HOME: /srv/easynet
      EASYNET_CLI_LIB: /usr/local/lib/libeasynet_cli.so
    volumes:
      - ${SHARED_DIR}:/shared
      - ${CERT_DIR}:/certs:ro
  provider:
    image: ${RUNTIME_IMAGE}
    hostname: provider
    entrypoint: ["/bin/sh", "-lc"]
    command: ["mkdir -p /home/provider /shared && tail -f /dev/null"]
    environment:
      HOME: /home/provider
      EASYNET_CLI_LIB: /usr/local/lib/libeasynet_cli.so
    volumes:
      - ${SHARED_DIR}:/shared
      - ${CERT_DIR}:/certs:ro
  caller:
    image: ${RUNTIME_IMAGE}
    hostname: caller
    entrypoint: ["/bin/sh", "-lc"]
    command: ["mkdir -p /home/caller /shared && tail -f /dev/null"]
    environment:
      HOME: /home/caller
      EASYNET_CLI_LIB: /usr/local/lib/libeasynet_cli.so
    volumes:
      - ${SHARED_DIR}:/shared
      - ${CERT_DIR}:/certs:ro
YAML

echo "==> starting Hub/provider/caller media bidi topology project=$PROJECT"
compose down -v --remove-orphans >/dev/null 2>&1 || true
compose up -d hub provider caller

echo "==> opening Hub daemon"
hub_cli "runtime start --as-hub --tenant '$REALM' --bind 0.0.0.0:50443 --cert /certs/hub.crt --key /certs/hub.key" \
  >"$OUT_DIR/hub-start.txt" 2>"$OUT_DIR/hub-start.err"
wait_hub_port_from caller
wait_runtime hub /srv/easynet hub

echo "==> bootstrapping PrincipalLifecycle"
hub_cli "principal bootstrap --principal-ura '$ADMIN_URA' --proof-ref 'bootstrap-admin-$TIMESTAMP' --create-idempotency-key admin-create-$TIMESTAMP --bind-idempotency-key admin-bind-$TIMESTAMP --json" \
  >"$OUT_DIR/principal-admin.json"
ADMIN_BINDING="$(jq -r '.principal.bindings[0].binding_id' "$OUT_DIR/principal-admin.json")"
hub_cli "principal issue-enrollment --issuer-ura '$ADMIN_URA' --subject-principal-ura '$USER_URA' --proof-ref '$ADMIN_BINDING' --idempotency-key alice-principal-$TIMESTAMP --json" \
  >"$OUT_DIR/enrollment-alice-principal.json"
ALICE_ENROLLMENT="$(jq -r '.principal.enrollments[-1].enrollment_id' "$OUT_DIR/enrollment-alice-principal.json")"
hub_cli "principal enroll --principal-ura '$USER_URA' --enrollment-id '$ALICE_ENROLLMENT' --create-idempotency-key alice-create-$TIMESTAMP --bind-idempotency-key alice-bind-$TIMESTAMP --json" \
  >"$OUT_DIR/principal-alice.json"
PROVIDER_ENROLLMENT="$(issue_device_enrollment provider)"
CALLER_ENROLLMENT="$(issue_device_enrollment caller)"

echo "==> joining provider and caller devices"
provider_cli "device join '$HUB_URA' --principal-ura '$USER_URA' --principal-enrollment-id '$PROVIDER_ENROLLMENT' --hub-ca /certs/ca.crt --hub-port 50443 --boot no --yes" \
  >"$OUT_DIR/provider-join.txt" 2>"$OUT_DIR/provider-join.err"
caller_cli "device join '$HUB_URA' --principal-ura '$USER_URA' --principal-enrollment-id '$CALLER_ENROLLMENT' --hub-ca /certs/ca.crt --hub-port 50443 --boot no --yes" \
  >"$OUT_DIR/caller-join.txt" 2>"$OUT_DIR/caller-join.err"
PROVIDER_NODE="$(service_exec provider "jq -r .node_id /home/provider/.easynet/credentials.json")"
CALLER_NODE="$(service_exec caller "jq -r .node_id /home/caller/.easynet/credentials.json")"
[[ -n "$PROVIDER_NODE" && "$PROVIDER_NODE" != "null" ]] || die "provider join did not persist node_id"
[[ -n "$CALLER_NODE" && "$CALLER_NODE" != "null" ]] || die "caller join did not persist node_id"
PROVIDER_URA="easynet:///r/${REALM}/device/${PROVIDER_NODE}"
CALLER_URA="easynet:///r/${REALM}/device/${CALLER_NODE}"

echo "==> starting provider and caller daemons"
provider_cli "runtime start" >"$OUT_DIR/provider-start.txt" 2>"$OUT_DIR/provider-start.err"
caller_cli "runtime start" >"$OUT_DIR/caller-start.txt" 2>"$OUT_DIR/caller-start.err"
wait_runtime provider /home/provider provider
wait_runtime caller /home/caller caller
wait_device_online provider /home/provider provider "$PROVIDER_NODE"
wait_device_online caller /home/caller caller "$CALLER_NODE"

echo "==> creating user-installed synthetic media sidecar plugin"
MEDIA_PLUGIN_ID="e2e.synthetic_media_bidi"
MEDIA_PLUGIN_VERSION="0.1.0"
MEDIA_STREAM_ABILITY="media.synthetic_stream"
MEDIA_BIDI_ABILITY="media.synthetic_bidi"
MEDIA_PLUGIN_ROOT="/shared/synthetic-media-bidi-plugin"
service_exec provider "rm -rf '$MEDIA_PLUGIN_ROOT' && mkdir -p '$MEDIA_PLUGIN_ROOT/abilities' '$MEDIA_PLUGIN_ROOT/bin'"
mkdir -p "$SHARED_DIR/synthetic-media-bidi-plugin/sdk-python"
cp -R "$REPO_ROOT/sdk/python/easynet_sdk" "$SHARED_DIR/synthetic-media-bidi-plugin/sdk-python/easynet_sdk"
cat >"$SHARED_DIR/synthetic-media-bidi-plugin/plugin.toml" <<'EOF'
schema_version = "1"
id = "e2e.synthetic_media_bidi"
version = "0.1.0"
kind = "sidecar"
entrypoint = "bin/synthetic-media-sidecar"
abilities = ["abilities/*.ability.toml"]
permissions = ["camera", "mic", "speaker", "screen"]
resources = ["camera", "mic", "speaker", "display"]
platforms = []

[limits]
max_sessions = 4
max_frame_queue = 16

[[ability_metadata]]
name = "media.synthetic_stream"
layer = "operational"
call_mode = "stream"

[[ability_metadata]]
name = "media.synthetic_bidi"
layer = "operational"
call_mode = "bidi"
bidi_wire_kind = "json_frames"

[[realtime_capability]]
kind = "camera"
modes = ["subscribe"]
transport = "invoke_stream"
activation_abilities = ["media.synthetic_stream"]
permissions = ["camera"]
resources = ["camera"]
quick_add = true

[[realtime_capability]]
kind = "mic"
modes = ["subscribe"]
transport = "invoke_stream"
activation_abilities = ["media.synthetic_stream"]
permissions = ["mic"]
resources = ["mic"]
quick_add = true

[[realtime_capability]]
kind = "speaker"
modes = ["publish"]
transport = "invoke_bidi"
activation_abilities = ["media.synthetic_bidi"]
permissions = ["speaker"]
resources = ["speaker"]
quick_add = true

[[realtime_capability]]
kind = "screen"
modes = ["subscribe"]
transport = "webrtc"
activation_abilities = ["media.synthetic_bidi"]
permissions = ["screen"]
resources = ["display"]
quick_add = true
EOF
cat >"$SHARED_DIR/synthetic-media-bidi-plugin/abilities/media.synthetic_stream.ability.toml" <<'EOF'
schema_version = "2"
name = "media.synthetic_stream"
descriptor_version = "1.0.0"
description = "Emit bounded synthetic audio/video/screen BinaryChunk-shaped JSON frames for Docker media E2E."
admission_action = "stream"

[input_schema]
type = "object"
additionalProperties = true

[output_schema]
type = "object"
additionalProperties = true
EOF
cat >"$SHARED_DIR/synthetic-media-bidi-plugin/abilities/media.synthetic_bidi.ability.toml" <<'EOF'
schema_version = "2"
name = "media.synthetic_bidi"
descriptor_version = "1.0.0"
description = "Echo synthetic audio/video/control JSON frames over InvokeBidi for Docker media E2E."
admission_action = "stream"

[input_schema]
type = "object"
additionalProperties = true

[output_schema]
type = "object"
additionalProperties = true
EOF
cat >"$SHARED_DIR/synthetic-media-bidi-plugin/bin/synthetic-media-sidecar" <<'PY'
#!/usr/bin/env python3
import base64
import hashlib
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "sdk-python"))

from easynet_sdk.providers.runtime.plugin_exec import SidecarInvocation, serve_plugin

def payload(kind, index, session_id):
    body = f"{kind}:{session_id}:{index}".encode("utf-8")
    return {
        "kind": kind,
        "stream_id": {"audio": 1, "video": 2, "screen": 3, "control": 4}[kind],
        "pts": index * 1000,
        "data_b64": base64.b64encode(body).decode("ascii"),
        "sha256": hashlib.sha256(body).hexdigest(),
        "bytes": len(body),
    }

def ack(frame):
    data = base64.b64decode(frame.get("data_b64", "") or "", validate=True)
    return {
        "kind": "ack",
        "media_kind": frame.get("kind"),
        "stream_id": frame.get("stream_id"),
        "pts": frame.get("pts"),
        "sha256": hashlib.sha256(data).hexdigest(),
        "bytes": len(data),
        "echo_body": frame.get("body"),
    }

def handle_invoke(_invocation: SidecarInvocation):
    return {"ok": True}

def handle_stream(invocation: SidecarInvocation):
    session_id = invocation.args.get("session_id") or "stream"
    yield {
            "kind": "stream_opened",
            "session_id": session_id,
            "caller_ura": invocation.caller_ura,
            "callee_ura": invocation.callee_ura,
            "subject_ura": invocation.subject_ura,
            "ability_ura": invocation.ability_ura,
            "nonce_bytes": len(invocation.invocation_nonce),
    }
    for frame_kind, count in (("audio", 2), ("video", 2), ("screen", 1)):
        for index in range(1, count + 1):
            yield payload(frame_kind, index, session_id)

def handle_bidi(invocation: SidecarInvocation, input_frames):
    session = {
        "session_id": invocation.args.get("session_id") or "bidi",
        "caller_ura": invocation.caller_ura,
        "callee_ura": invocation.callee_ura,
        "subject_ura": invocation.subject_ura,
        "ability_ura": invocation.ability_ura,
        "nonce_bytes": len(invocation.invocation_nonce),
    }
    yield {"kind": "session_established", **session}
    acks = []
    for frame in input_frames:
        if frame.get("kind") in {"audio", "video", "control"}:
            item = ack(frame)
            acks.append(item)
            yield item
            if frame.get("kind") == "control" and frame.get("body") == "close":
                yield {
                    "kind": "summary",
                    "session_id": session.get("session_id"),
                    "ack_count": len(acks),
                    "media_kinds": sorted({item.get("media_kind") for item in acks}),
                }
                return
        else:
            raise ValueError(f"unsupported input frame {frame!r}")

serve_plugin(
    invoke_handler=handle_invoke,
    stream_handler=handle_stream,
    bidi_handler=handle_bidi,
    stream_terminal_reason="stream_complete",
    bidi_terminal_reason="client_control_close",
)
PY
chmod +x "$SHARED_DIR/synthetic-media-bidi-plugin/bin/synthetic-media-sidecar"

provider_cli "plugin install '$MEDIA_PLUGIN_ROOT'" >"$OUT_DIR/provider-plugin-install-media.txt" 2>"$OUT_DIR/provider-plugin-install-media.err"
provider_cli "plugin status '$MEDIA_PLUGIN_ID' --version '$MEDIA_PLUGIN_VERSION' --json" \
  >"$OUT_DIR/provider-plugin-status-media.json" 2>"$OUT_DIR/provider-plugin-status-media.err"
provider_cli "plugin list --format json" >"$OUT_DIR/provider-plugin-list-after-media-install.json" 2>"$OUT_DIR/provider-plugin-list-after-media-install.err"

MEDIA_STREAM_URA="$(wait_ability_name provider_cli "" "$MEDIA_STREAM_ABILITY" "provider-ability-list-media-stream")"
MEDIA_BIDI_URA="$(wait_ability_name provider_cli "" "$MEDIA_BIDI_ABILITY" "provider-ability-list-media-bidi")"
MEDIA_STREAM_DESCRIPTOR_REF="$(wait_ability_descriptor_ref provider_cli "" "$MEDIA_STREAM_ABILITY" "provider-ability-list-media-stream-descriptor" "stream")"
MEDIA_BIDI_DESCRIPTOR_REF="$(wait_ability_descriptor_ref provider_cli "" "$MEDIA_BIDI_ABILITY" "provider-ability-list-media-bidi-descriptor" "stream")"
CALLER_MEDIA_STREAM_URA="$(wait_ability_name caller_cli "$PROVIDER_URA" "$MEDIA_STREAM_ABILITY" "caller-ability-list-media-stream")"
CALLER_MEDIA_BIDI_URA="$(wait_ability_name caller_cli "$PROVIDER_URA" "$MEDIA_BIDI_ABILITY" "caller-ability-list-media-bidi")"
CALLER_MEDIA_STREAM_DESCRIPTOR_REF="$(wait_ability_descriptor_ref caller_cli "$PROVIDER_URA" "$MEDIA_STREAM_ABILITY" "caller-ability-list-media-stream-descriptor" "stream")"
CALLER_MEDIA_BIDI_DESCRIPTOR_REF="$(wait_ability_descriptor_ref caller_cli "$PROVIDER_URA" "$MEDIA_BIDI_ABILITY" "caller-ability-list-media-bidi-descriptor" "stream")"
[[ "$MEDIA_STREAM_URA" == easynet://* ]] || die "provider media stream ability did not resolve to URA"
[[ "$MEDIA_BIDI_URA" == easynet://* ]] || die "provider media bidi ability did not resolve to URA"
[[ "$MEDIA_STREAM_DESCRIPTOR_REF" == easynet://*@*#*'!stream' ]] || die "provider media stream list did not expose stream descriptor-bound ref"
[[ "$MEDIA_BIDI_DESCRIPTOR_REF" == easynet://*@*#*'!stream' ]] || die "provider media bidi list did not expose stream-admission descriptor-bound ref"
[[ "$CALLER_MEDIA_STREAM_URA" == easynet://* ]] || die "caller did not discover provider media stream ability"
[[ "$CALLER_MEDIA_BIDI_URA" == easynet://* ]] || die "caller did not discover provider media bidi ability"
[[ "$CALLER_MEDIA_STREAM_DESCRIPTOR_REF" == easynet://*@*#*'!stream' ]] || die "caller media stream list did not expose stream descriptor-bound ref"
[[ "$CALLER_MEDIA_BIDI_DESCRIPTOR_REF" == easynet://*@*#*'!stream' ]] || die "caller media bidi list did not expose stream-admission descriptor-bound ref"

MEDIA_SUBJECT="easynet:///r/${REALM}/resource/e2e/synthetic-media/session-${TIMESTAMP}"

echo "==> invoking provider synthetic media stream through CLI"
STREAM_NONCE_HEX="$(random_nonce_hex)"
provider_cli "ability stream '$MEDIA_STREAM_URA' --subject '$MEDIA_SUBJECT' --nonce-hex '$STREAM_NONCE_HEX' --causal-root --args '$(json_args stream "stream-$TIMESTAMP")' --format json --raw" \
  >"$OUT_DIR/provider-media-stream.json" 2>"$OUT_DIR/provider-media-stream.err"

echo "==> invoking provider synthetic media stream from caller through remote CLI"
CALLER_REMOTE_STREAM_NONCE_HEX="$(random_nonce_hex)"
caller_cli "ability stream '$CALLER_MEDIA_STREAM_DESCRIPTOR_REF' --node '$PROVIDER_URA' --subject '$MEDIA_SUBJECT' --nonce-hex '$CALLER_REMOTE_STREAM_NONCE_HEX' --causal-root --args '$(json_args stream "caller-remote-stream-$TIMESTAMP")' --format json --raw" \
  >"$OUT_DIR/caller-remote-media-stream.json" 2>"$OUT_DIR/caller-remote-media-stream.err"

echo "==> invoking provider synthetic media bidi from caller through remote CLI"
CALLER_REMOTE_BIDI_NONCE_HEX="$(random_nonce_hex)"
caller_cli "ability bidi '$CALLER_MEDIA_BIDI_DESCRIPTOR_REF' --node '$PROVIDER_URA' --subject '$MEDIA_SUBJECT' --nonce-hex '$CALLER_REMOTE_BIDI_NONCE_HEX' --causal-root --args '$(json_args bidi "caller-remote-bidi-$TIMESTAMP")' --input '$(json_frame audio 1 remote-audio-frame-1)' --input '$(json_frame video 2 remote-video-frame-2)' --input '$(json_frame control 3 close)' --until-terminal --format json --raw" \
  >"$OUT_DIR/caller-remote-media-bidi.json" 2>"$OUT_DIR/caller-remote-media-bidi.err"

echo "==> invoking provider synthetic media bidi through CLI"
BIDI_NONCE_HEX="$(random_nonce_hex)"
provider_cli "ability bidi '$MEDIA_BIDI_URA' --subject '$MEDIA_SUBJECT' --nonce-hex '$BIDI_NONCE_HEX' --causal-root --args '$(json_args bidi "bidi-$TIMESTAMP")' --input '$(json_frame audio 1 audio-frame-1)' --input '$(json_frame video 2 video-frame-2)' --input '$(json_frame control 3 close)' --until-terminal --format json --raw" \
  >"$OUT_DIR/provider-media-bidi.json" 2>"$OUT_DIR/provider-media-bidi.err"

provider_cli "invocation list --ability-ura '$MEDIA_STREAM_URA' --format json" \
  >"$OUT_DIR/provider-invocation-list-media-stream.json" 2>"$OUT_DIR/provider-invocation-list-media-stream.err"
provider_cli "invocation list --ability-ura '$MEDIA_BIDI_URA' --format json" \
  >"$OUT_DIR/provider-invocation-list-media-bidi.json" 2>"$OUT_DIR/provider-invocation-list-media-bidi.err"

service_exec hub "cat /srv/easynet/.easynet/logs/easynet-daemon.log" \
  >"$OUT_DIR/hub-daemon.log" 2>"$OUT_DIR/hub-daemon-log.err" || true
service_exec provider "cat /home/provider/.easynet/logs/easynet-daemon.log" \
  >"$OUT_DIR/provider-daemon.log" 2>"$OUT_DIR/provider-daemon-log.err" || true
service_exec caller "cat /home/caller/.easynet/logs/easynet-daemon.log" \
  >"$OUT_DIR/caller-daemon.log" 2>"$OUT_DIR/caller-daemon-log.err" || true

provider_cli "plugin remove '$MEDIA_PLUGIN_ID' '$MEDIA_PLUGIN_VERSION'" >"$OUT_DIR/provider-plugin-remove-media.txt" 2>"$OUT_DIR/provider-plugin-remove-media.err"
provider_cli "plugin list --format json" >"$OUT_DIR/provider-plugin-list-after-media-remove.json" 2>"$OUT_DIR/provider-plugin-list-after-media-remove.err"
provider_cli "ability list --format json" >"$OUT_DIR/provider-ability-list-after-media-remove.json" 2>"$OUT_DIR/provider-ability-list-after-media-remove.err"
caller_cli_must_fail \
  "ability stream '$CALLER_MEDIA_STREAM_DESCRIPTOR_REF' --node '$PROVIDER_URA' --subject '$MEDIA_SUBJECT' --nonce-hex '$(random_nonce_hex)' --causal-root --timeout 5 --args '$(json_args stream "removed-stream-$TIMESTAMP")' --format json --raw" \
  "$OUT_DIR/caller-removed-media-stream.stdout" \
  "$OUT_DIR/caller-removed-media-stream.stderr"
caller_cli_must_fail \
  "ability bidi '$CALLER_MEDIA_BIDI_DESCRIPTOR_REF' --node '$PROVIDER_URA' --subject '$MEDIA_SUBJECT' --nonce-hex '$(random_nonce_hex)' --causal-root --timeout 5 --args '$(json_args bidi "removed-bidi-$TIMESTAMP")' --input '$(json_frame control 1 close)' --until-terminal --format json --raw" \
  "$OUT_DIR/caller-removed-media-bidi.stdout" \
  "$OUT_DIR/caller-removed-media-bidi.stderr"

python3 - "$OUT_DIR" "$PROVIDER_URA" "$CALLER_URA" "$MEDIA_STREAM_URA" "$MEDIA_BIDI_URA" "$MEDIA_STREAM_DESCRIPTOR_REF" "$MEDIA_BIDI_DESCRIPTOR_REF" "$CALLER_MEDIA_STREAM_URA" "$CALLER_MEDIA_BIDI_URA" "$CALLER_MEDIA_STREAM_DESCRIPTOR_REF" "$CALLER_MEDIA_BIDI_DESCRIPTOR_REF" <<'PY' >"$OUT_DIR/report.json"
import json
import pathlib
import sys

out = pathlib.Path(sys.argv[1])
provider_ura, caller_ura, stream_ura, bidi_ura, stream_descriptor_ref, bidi_descriptor_ref, caller_stream_ura, caller_bidi_ura, caller_stream_descriptor_ref, caller_bidi_descriptor_ref = sys.argv[2:12]

def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))

def load_json_array_from_cli(path):
    text = path.read_text(encoding="utf-8")
    start = text.find("[")
    end = text.rfind("]")
    if start < 0 or end < start:
        return []
    return json.loads(text[start:end + 1])

def load_text(path):
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return ""

def load_int(path):
    text = load_text(path).strip()
    if not text:
        return None
    try:
        return int(text)
    except ValueError:
        return None

def rows(path):
    payload = load_json(path)
    if isinstance(payload, list):
        return payload
    for key in ("records", "items", "invocations", "abilities", "packages"):
        value = payload.get(key)
        if isinstance(value, list):
            return value
    return []

def receipt_projected(path):
    records = rows(path)
    if not records:
        return False
    def has_receipt(row):
        blob = json.dumps(row, sort_keys=True)
        return "receipt" in blob and ("completed" in blob.lower() or "succeeded" in blob.lower() or "terminal" in blob.lower())
    return any(has_receipt(row) for row in records)

def completed_receipt_records(path):
    completed = []
    for row in rows(path):
        if not isinstance(row, dict):
            continue
        blob = json.dumps(row, sort_keys=True).lower()
        if "receipt" in blob and "completed" in blob:
            completed.append(row)
    return completed

def receipt_anchors(record):
    chain = record.get("receipt_chain")
    if not isinstance(chain, dict):
        return []
    anchors = chain.get("anchors")
    return anchors if isinstance(anchors, list) else []

def unique_non_empty(values):
    compact = [value for value in values if value]
    return bool(compact) and len(compact) == len(set(compact))

def completed_chain_facts(records, expected_ability_ura, expected_callee_ura):
    facts = {
        "record_count": len(records),
        "request_ids": [str(record.get("request_id") or "") for record in records],
        "invocation_uras": [str(record.get("invocation_ura") or "") for record in records],
        "head_receipt_hashes": [],
        "terminal_receipt_counts": [],
        "all_completed": False,
        "all_expected_ability": False,
        "all_expected_callee": False,
        "unique_request_ids": False,
        "unique_invocation_uras": False,
        "all_verified_receipt_chains": False,
        "all_single_completed_terminal_receipt": False,
        "all_terminal_head_receipts": False,
    }
    if not records:
        return facts
    terminal_counts = []
    terminal_heads = []
    for record in records:
        chain = record.get("receipt_chain") if isinstance(record, dict) else None
        chain = chain if isinstance(chain, dict) else {}
        head_hash = str(chain.get("head_receipt_hash") or "")
        facts["head_receipt_hashes"].append(head_hash)
        completed_anchors = [
            anchor
            for anchor in receipt_anchors(record)
            if isinstance(anchor, dict)
            and anchor.get("state") == "completed"
            and anchor.get("receipt_type") == "completed"
        ]
        terminal_counts.append(len(completed_anchors))
        terminal_heads.append(
            len(completed_anchors) == 1
            and head_hash
            and str(completed_anchors[0].get("receipt_hash") or "") == head_hash
        )
    facts["terminal_receipt_counts"] = terminal_counts
    facts["all_completed"] = all(record.get("state") == "completed" for record in records)
    facts["all_expected_ability"] = all(record.get("ability_ura") == expected_ability_ura for record in records)
    facts["all_expected_callee"] = all(record.get("callee_ura") == expected_callee_ura for record in records)
    facts["unique_request_ids"] = unique_non_empty(facts["request_ids"])
    facts["unique_invocation_uras"] = unique_non_empty(facts["invocation_uras"])
    facts["all_verified_receipt_chains"] = all(
        isinstance(record.get("receipt_chain"), dict)
        and record["receipt_chain"].get("verified") is True
        for record in records
    )
    facts["all_single_completed_terminal_receipt"] = all(count == 1 for count in terminal_counts)
    facts["all_terminal_head_receipts"] = all(terminal_heads)
    return facts

plugin_status = load_json(out / "provider-plugin-status-media.json")
stream_frames = load_json_array_from_cli(out / "provider-media-stream.json")
caller_remote_stream_frames = load_json_array_from_cli(out / "caller-remote-media-stream.json")
bidi_frames = load_json_array_from_cli(out / "provider-media-bidi.json")
caller_remote_bidi_frames = load_json_array_from_cli(out / "caller-remote-media-bidi.json")
caller_remote_bidi_stderr = load_text(out / "caller-remote-media-bidi.err")
hub_daemon_log = load_text(out / "hub-daemon.log")

stream_payloads = [frame.get("payload") for frame in stream_frames if isinstance(frame, dict)]
caller_remote_stream_payloads = [frame.get("payload") for frame in caller_remote_stream_frames if isinstance(frame, dict)]
bidi_payloads = [frame.get("payload") for frame in bidi_frames if isinstance(frame, dict)]
caller_remote_bidi_payloads = [frame.get("payload") for frame in caller_remote_bidi_frames if isinstance(frame, dict)]
stream_records = completed_receipt_records(out / "provider-invocation-list-media-stream.json")
bidi_records = completed_receipt_records(out / "provider-invocation-list-media-bidi.json")
stream_chain_facts = completed_chain_facts(stream_records, stream_ura, provider_ura)
bidi_chain_facts = completed_chain_facts(bidi_records, bidi_ura, provider_ura)

def payload_kinds(payloads):
    return {p.get("kind") for p in payloads if isinstance(p, dict)}

def terminal_count(frames):
    return sum(1 for frame in frames if isinstance(frame, dict) and frame.get("terminal") is True)

def catalog_exposes_ability(path, ability_ura, ability_name):
    for row in rows(path):
        if not isinstance(row, dict):
            continue
        values = {
            str(row.get("ability_ura") or ""),
            str(row.get("name") or ""),
            str(row.get("ability_name") or ""),
            str(row.get("public_name") or ""),
            str(row.get("qualified_name") or ""),
        }
        if ability_ura in values or ability_name in values:
            return True
    return False

plugin_list_after_remove = load_json(out / "provider-plugin-list-after-media-remove.json")
ability_list_after_remove = out / "provider-ability-list-after-media-remove.json"
removed_stream_exit = load_int(out / "caller-removed-media-stream.stderr.exit_code")
removed_bidi_exit = load_int(out / "caller-removed-media-bidi.stderr.exit_code")
removed_stream_error = load_text(out / "caller-removed-media-stream.stderr") + load_text(out / "caller-removed-media-stream.stdout")
removed_bidi_error = load_text(out / "caller-removed-media-bidi.stderr") + load_text(out / "caller-removed-media-bidi.stdout")
plugin_after_remove_blob = json.dumps(plugin_list_after_remove, sort_keys=True)

def rejected_without_harness_timeout(exit_code, text):
    return exit_code not in (None, 0, 124) and bool(text.strip()) and "success" not in text.lower()

status_blob = json.dumps(plugin_status, sort_keys=True)
assertions = {
    "provider_media_plugin_loaded": "e2e.synthetic_media_bidi" in status_blob,
    "provider_media_stream_invokable": "media.synthetic_stream" in status_blob and "invokable" in status_blob,
    "provider_media_bidi_invoked": bool(bidi_frames) and "session_established" in payload_kinds(bidi_payloads),
    "plugin_declares_single_webrtc_transport": "webrtc" in status_blob and ("fallback_" + "transport") not in status_blob,
    "provider_media_stream_audio_video_screen_frames": {"audio", "video", "screen"}.issubset(payload_kinds(stream_payloads)),
    "provider_media_stream_preserved_invocation_tuple": any(
        isinstance(p, dict)
        and p.get("kind") == "stream_opened"
        and p.get("caller_ura")
        and p.get("callee_ura")
        and p.get("subject_ura")
        and p.get("ability_ura") == stream_ura
        and p.get("nonce_bytes") == 16
        for p in stream_payloads
    ),
    "provider_media_bidi_established": "session_established" in payload_kinds(bidi_payloads),
    "provider_media_bidi_audio_video_control_acks": {"audio", "video", "control"} == {
        p.get("media_kind") for p in bidi_payloads if isinstance(p, dict) and p.get("kind") == "ack"
    },
    "provider_media_bidi_summary": any(isinstance(p, dict) and p.get("kind") == "summary" and p.get("ack_count") == 3 for p in bidi_payloads),
    "provider_media_bidi_single_terminal": terminal_count(bidi_frames) == 1,
    "provider_media_stream_receipt_chain_projected": receipt_projected(out / "provider-invocation-list-media-stream.json"),
    "provider_media_bidi_receipt_chain_projected": receipt_projected(out / "provider-invocation-list-media-bidi.json"),
    "caller_discovered_provider_media_stream": caller_stream_ura.startswith("easynet://"),
    "caller_discovered_provider_media_bidi": caller_bidi_ura.startswith("easynet://"),
    "caller_media_stream_descriptor_ref": caller_stream_descriptor_ref.startswith("easynet://")
        and "@" in caller_stream_descriptor_ref
        and "#" in caller_stream_descriptor_ref
        and caller_stream_descriptor_ref.endswith("!stream"),
    "caller_media_bidi_descriptor_ref": caller_bidi_descriptor_ref.startswith("easynet://")
        and "@" in caller_bidi_descriptor_ref
        and "#" in caller_bidi_descriptor_ref
        and "media.synthetic_bidi" in caller_bidi_descriptor_ref
        and caller_bidi_descriptor_ref.endswith("!stream"),
    "caller_remote_media_stream_succeeded": bool(caller_remote_stream_frames)
        and {"audio", "video", "screen"}.issubset(payload_kinds(caller_remote_stream_payloads)),
    "caller_remote_media_stream_preserved_invocation_tuple": any(
        isinstance(p, dict)
        and p.get("kind") == "stream_opened"
        and p.get("caller_ura")
        and p.get("callee_ura")
        and p.get("subject_ura")
        and p.get("ability_ura") == stream_ura
        and p.get("nonce_bytes") == 16
        for p in caller_remote_stream_payloads
    ),
    "caller_remote_media_bidi_succeeded": bool(caller_remote_bidi_frames)
        and "session_established" in payload_kinds(caller_remote_bidi_payloads),
    "caller_remote_media_bidi_audio_video_control_acks": {"audio", "video", "control"} == {
        p.get("media_kind")
        for p in caller_remote_bidi_payloads
        if isinstance(p, dict) and p.get("kind") == "ack"
    },
    "caller_remote_media_bidi_summary": any(
        isinstance(p, dict) and p.get("kind") == "summary" and p.get("ack_count") == 3
        for p in caller_remote_bidi_payloads
    ),
    "caller_remote_media_bidi_single_terminal": terminal_count(caller_remote_bidi_frames) == 1,
    "caller_remote_media_bidi_used_invoke_bidi_cli": "canonical InvokeBidi" in caller_remote_bidi_stderr,
    "hub_observed_reverse_bidi_open": "carrier_v1_reverse_bidi_opened" in hub_daemon_log,
    "hub_observed_reverse_bidi_input": "carrier_v1_reverse_bidi_input" in hub_daemon_log,
    "media_stream_two_operations_two_receipt_chains": len(stream_records) == 2,
    "media_bidi_two_operations_two_receipt_chains": len(bidi_records) == 2,
    "media_stream_unique_invocation_records": stream_chain_facts["record_count"] == 2
        and stream_chain_facts["unique_request_ids"]
        and stream_chain_facts["unique_invocation_uras"],
    "media_bidi_unique_invocation_records": bidi_chain_facts["record_count"] == 2
        and bidi_chain_facts["unique_request_ids"]
        and bidi_chain_facts["unique_invocation_uras"],
    "media_stream_preserved_provider_tuple": stream_chain_facts["all_expected_ability"]
        and stream_chain_facts["all_expected_callee"],
    "media_bidi_preserved_provider_tuple": bidi_chain_facts["all_expected_ability"]
        and bidi_chain_facts["all_expected_callee"],
    "media_stream_verified_single_terminal_receipt_chains": stream_chain_facts["all_completed"]
        and stream_chain_facts["all_verified_receipt_chains"]
        and stream_chain_facts["all_single_completed_terminal_receipt"]
        and stream_chain_facts["all_terminal_head_receipts"],
    "media_bidi_verified_single_terminal_receipt_chains": bidi_chain_facts["all_completed"]
        and bidi_chain_facts["all_verified_receipt_chains"]
        and bidi_chain_facts["all_single_completed_terminal_receipt"]
        and bidi_chain_facts["all_terminal_head_receipts"],
    "provider_media_plugin_removed": "e2e.synthetic_media_bidi" not in plugin_after_remove_blob,
    "provider_removed_media_abilities_unpublished": not catalog_exposes_ability(
        ability_list_after_remove,
        stream_ura,
        "media.synthetic_stream",
    )
        and not catalog_exposes_ability(
            ability_list_after_remove,
            bidi_ura,
            "media.synthetic_bidi",
        ),
    "provider_removed_media_routes_reject_invocation": rejected_without_harness_timeout(
        removed_stream_exit,
        removed_stream_error,
    )
        and rejected_without_harness_timeout(removed_bidi_exit, removed_bidi_error),
}
assertions["media_product_operations_have_verified_single_terminal_receipt_chains"] = (
    assertions["media_stream_unique_invocation_records"]
    and assertions["media_bidi_unique_invocation_records"]
    and assertions["media_stream_preserved_provider_tuple"]
    and assertions["media_bidi_preserved_provider_tuple"]
    and assertions["media_stream_verified_single_terminal_receipt_chains"]
    and assertions["media_bidi_verified_single_terminal_receipt_chains"]
)

report = {
    "topology": {
        "provider_ura": provider_ura,
        "caller_ura": caller_ura,
    },
    "abilities": {
        "stream_ura": stream_ura,
        "bidi_ura": bidi_ura,
        "stream_descriptor_ref": stream_descriptor_ref,
        "bidi_descriptor_ref": bidi_descriptor_ref,
        "caller_stream_ura": caller_stream_ura,
        "caller_bidi_ura": caller_bidi_ura,
        "caller_stream_descriptor_ref": caller_stream_descriptor_ref,
        "caller_bidi_descriptor_ref": caller_bidi_descriptor_ref,
    },
    "stream_frames": stream_frames,
    "caller_remote_stream_frames": caller_remote_stream_frames,
    "bidi_frames": bidi_frames,
    "caller_remote_bidi_frames": caller_remote_bidi_frames,
    "provider_media_stream_invocation_records": stream_records,
    "provider_media_bidi_invocation_records": bidi_records,
    "mutation_facts": {
        "stream": stream_chain_facts,
        "bidi": bidi_chain_facts,
    },
    "plugin_removal_facts": {
        "stream_route_exit_code": removed_stream_exit,
        "bidi_route_exit_code": removed_bidi_exit,
        "stream_error_excerpt": removed_stream_error.strip()[:800],
        "bidi_error_excerpt": removed_bidi_error.strip()[:800],
        "provider_catalog_exposes_stream_after_remove": catalog_exposes_ability(
            ability_list_after_remove,
            stream_ura,
            "media.synthetic_stream",
        ),
        "provider_catalog_exposes_bidi_after_remove": catalog_exposes_ability(
            ability_list_after_remove,
            bidi_ura,
            "media.synthetic_bidi",
        ),
    },
    "assertions": assertions,
}
print(json.dumps(report, indent=2, sort_keys=True))
PY

python3 - "$OUT_DIR/report.json" >"$OUT_DIR/report.md" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
print("# Docker media/bidi E2E report")
print()
print(f"- Provider device: `{report['topology']['provider_ura']}`")
print(f"- Caller device: `{report['topology']['caller_ura']}`")
print(f"- Stream ability: `{report['abilities']['stream_ura']}`")
print(f"- Bidi ability: `{report['abilities']['bidi_ura']}`")
print(f"- Provider stream descriptor ref: `{report['abilities']['stream_descriptor_ref']}`")
print(f"- Provider bidi descriptor ref: `{report['abilities']['bidi_descriptor_ref']}`")
print(f"- Caller remote stream ability: `{report['abilities']['caller_stream_ura']}`")
print(f"- Caller remote bidi ability: `{report['abilities']['caller_bidi_ura']}`")
print(f"- Caller stream descriptor ref: `{report['abilities']['caller_stream_descriptor_ref']}`")
print(f"- Caller bidi descriptor ref: `{report['abilities']['caller_bidi_descriptor_ref']}`")
print()
print("## Assertions")
print()
for key, value in report["assertions"].items():
    print(f"- `{key}`: `{str(value).lower()}`")
print()
print("## Mutation facts")
print()
for kind in ("stream", "bidi"):
    facts = report["mutation_facts"][kind]
    print(f"### {kind}")
    print()
    print(f"- record_count: `{facts['record_count']}`")
    print(f"- request_ids: `{', '.join(facts['request_ids'])}`")
    print(f"- invocation_uras: `{', '.join(facts['invocation_uras'])}`")
    print(f"- terminal_receipt_counts: `{', '.join(str(v) for v in facts['terminal_receipt_counts'])}`")
    print(f"- head_receipt_hashes: `{', '.join(facts['head_receipt_hashes'])}`")
    print(f"- all_verified_receipt_chains: `{str(facts['all_verified_receipt_chains']).lower()}`")
    print(f"- all_terminal_head_receipts: `{str(facts['all_terminal_head_receipts']).lower()}`")
    print()
print("## Plugin removal facts")
print()
removal = report["plugin_removal_facts"]
print(f"- stream_route_exit_code: `{removal['stream_route_exit_code']}`")
print(f"- bidi_route_exit_code: `{removal['bidi_route_exit_code']}`")
print(f"- provider_catalog_exposes_stream_after_remove: `{str(removal['provider_catalog_exposes_stream_after_remove']).lower()}`")
print(f"- provider_catalog_exposes_bidi_after_remove: `{str(removal['provider_catalog_exposes_bidi_after_remove']).lower()}`")
PY

python3 - "$OUT_DIR/report.json" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
failed = [key for key, value in report["assertions"].items() if value is not True]
if failed:
    print("docker-media-bidi-e2e failed assertions:", ", ".join(failed), file=sys.stderr)
    raise SystemExit(1)
PY

echo "PASS: $OUT_DIR/report.md"

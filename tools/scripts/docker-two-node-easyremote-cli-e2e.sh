#!/usr/bin/env bash
# Docker E2E for the real EasyRemote-on-CLI product closure.
#
# Topology:
#   hub      - opens a CLI-managed Hub daemon on TCP/TLS.
#   provider - independently joins the Hub, starts a device daemon, and
#              publishes a native EasyNet host_stream ability through the CLI
#              plus several EasyRemote functions with @node.register.
#   caller   - independently joins the same Hub, queries provider abilities
#              through the CLI, calls one through the CLI, calls the full
#              EasyRemote syntax matrix through @remote typed stubs, and calls
#              the native EasyNet ability through an EasyRemote ability handle.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
WORKSPACE_ROOT="$(cd "$REPO_ROOT/.." && pwd)"
EASYNET_ROOT="${EASYNET_BACKEND_ROOT:-$WORKSPACE_ROOT/EasyNet}"
EASYREMOTE_ROOT="${EASYNET_EASYREMOTE_ROOT:-$WORKSPACE_ROOT/EasyRemote}"
AXON_ROOT="${EASYNET_AXON_ROOT:-$WORKSPACE_ROOT/EasyNet-Axon}"

PROJECT="${EASYNET_E2E_PROJECT:-easynet-easyremote-two-node}"
RUNTIME_IMAGE="${EASYNET_RUNTIME_IMAGE:-${EASYNET_HUB_IMAGE:-easynet/hub-e2e:local}}"
DOCKER_BIN="${DOCKER_BIN:-}"
REALM="${EASYNET_E2E_REALM:-hub}"
HUB_URA="easynet:///r/${REALM}/authority"
ADMIN_URA="easynet:///r/${REALM}/user/admin"
USER_URA="easynet:///r/${REALM}/user/alice"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="${EASYNET_E2E_OUT_DIR:-$REPO_ROOT/target/e2e/docker-two-node-easyremote-cli/$TIMESTAMP}"
KEEP=0
SKIP_BUILD=0
SELF_TEST=0

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/docker-two-node-easyremote-cli-e2e.sh [options]

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
  EASYNET_EASYREMOTE_ROOT    Sibling EasyRemote repo mounted into devices.
  EASYNET_AXON_ROOT          Sibling EasyNet-Axon repo mounted for SDK imports.
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
  [[ -d "$EASYREMOTE_ROOT/easyremote" ]] || die "EasyRemote package not found: $EASYREMOTE_ROOT/easyremote"
  [[ -d "$REPO_ROOT/sdk/python/easynet_sdk" ]] || die "EasyNet-Cli Python SDK package not found"
  [[ -d "$AXON_ROOT/sdk/python/axon_sdk" ]] || die "Axon Python SDK package not found"
}

if [[ "$SELF_TEST" == "1" ]]; then
  bash -n "$0"
  require_paths
  grep -q "provider:" "$0"
  grep -q "caller:" "$0"
  grep -q "easynet device join" "$0"
  grep -q "ComputeNode" "$0"
  grep -q "@node.register" "$0"
  grep -q "@remote" "$0"
  grep -q "provider.remote" "$0"
  grep -q "ability deploy" "$0"
  grep -q -- "--format json" "$0"
  grep -q "provider.ability" "$0"
  grep -q "nativeer.native_echo" "$0"
  grep -q "client.functions(scope=\"user\")" "$0"
  grep -q "canonical_ura_call" "$0"
  grep -q "canonical_ura_stream" "$0"
  grep -q "easyremote_sdk_consumer_boundary" "$0"
  grep -q "forbidden_easyremote_authority" "$0"
  grep -q "user-defined declarative exec plugin" "$0"
  grep -q "user_plugin.echo" "$0"
  grep -q "plugin init" "$0"
  grep -q "PYTHONPATH: /work/EasyRemote:/work/EasyNet-Cli/sdk/python:/work/EasyNet-Axon/sdk/python" "$0"
  grep -q "plugin install" "$0"
  grep -q "plugin remove" "$0"
  grep -q "user_plugin_one_invocation_record" "$0"
  grep -q "provider_catalog" "$0"
  grep -q "provider-invocation-list-native-after-easyremote" "$0"
  grep -q "native_easynet_receipt_chains_projected" "$0"
  grep -q "caller_observed_native_easynet_ability_removed" "$0"
  grep -q "provider-hosted user external Agent" "$0"
  grep -q "user_agent_canonical_invocation_gap_detected" "$0"
  grep -q "caller_observed_user_agent_chat_removed" "$0"
  grep -q "ability list --node" "$0"
  grep -q "ability stream" "$0"
  grep -q "invocation list --ability-ura" "$0"
  grep -q "resolve_docker" "$0"
  grep -q "extend_tool_path" "$0"
  grep -q "docker compose" "$0"
  grep -q "build-linux-cli-artifact-bundle.sh" "$0"
  grep -q "EASYNET_CLI_ARTIFACT_ROOT" "$0"
  echo "docker-two-node-easyremote-cli-e2e self-test ok"
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
WORK_ROOT="$(mktemp -d "/tmp/easynet-er2.XXXXXX")"
SHARED_DIR="$WORK_ROOT/shared"
CERT_DIR="$WORK_ROOT/certs"
mkdir -p "$SHARED_DIR" "$CERT_DIR"
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
    "$DOCKER_BIN" compose -p "$PROJECT" -f "$COMPOSE_FILE" down -v --remove-orphans >/dev/null 2>&1 || true
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

compose() {
  "$DOCKER_BIN" compose -p "$PROJECT" -f "$COMPOSE_FILE" "$@"
}

service_exec() {
  local service="$1"
  shift
  compose exec -T "$service" sh -lc "$*"
}

hub_cli() {
  service_exec hub "HOME=/srv/easynet EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so easynet $*"
}

provider_cli() {
  service_exec provider "HOME=/home/provider EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so easynet $*"
}

caller_cli() {
  service_exec caller "HOME=/home/caller EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so easynet $*"
}

dump_logs() {
  echo "==> docker compose logs" >&2
  compose logs --no-color hub provider caller >&2 || true
  echo "==> daemon logs" >&2
  service_exec hub "find /srv/easynet/.easynet -maxdepth 4 -type f -name '*.log' -print -exec tail -120 {} \\;" >&2 || true
  service_exec provider "find /home/provider/.easynet -maxdepth 4 -type f -name '*.log' -print -exec tail -160 {} \\;" >&2 || true
  service_exec caller "find /home/caller/.easynet -maxdepth 4 -type f -name '*.log' -print -exec tail -160 {} \\;" >&2 || true
  service_exec provider "tail -200 /shared/native-easynet-host.log" >&2 || true
  service_exec provider "tail -200 /shared/easyremote-provider.log" >&2 || true
  service_exec caller "tail -200 /shared/easyremote-caller.log" >&2 || true
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

wait_hub_device() {
  local node_id="$1"
  for _ in $(seq 1 120); do
    if hub_cli "device list --state all --format json" >"$OUT_DIR/hub-device-list.json" 2>"$OUT_DIR/hub-device-list.err"; then
      if jq -e --arg node "$node_id" '(.nodes // []) | any(.node_id == $node)' "$OUT_DIR/hub-device-list.json" >/dev/null; then
        return 0
      fi
    fi
    sleep 0.5
  done
  cat "$OUT_DIR/hub-device-list.json" >&2 2>/dev/null || true
  cat "$OUT_DIR/hub-device-list.err" >&2 2>/dev/null || true
  return 1
}

wait_caller_remote_ability_list() {
  local provider_node="$1"
  local out="$OUT_DIR/caller-ability-list-provider.json"
  local err="$OUT_DIR/caller-ability-list-provider.err"
  for _ in $(seq 1 120); do
    if caller_cli "ability list --node '$provider_node' --format json" >"$out" 2>"$err"; then
      if jq -e 'if type == "array" then length >= 1 else (.abilities // []) | length >= 1 end' "$out" >/dev/null; then
        return 0
      fi
    fi
    sleep 0.5
  done
  cat "$out" >&2 2>/dev/null || true
  cat "$err" >&2 2>/dev/null || true
  return 1
}

extract_ability_ura_by_name() {
  local file="$1"
  local ability_name="$2"
  python3 - "$file" "$ability_name" <<'PY'
import json
import sys

path, expected = sys.argv[1:3]
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
        ability_ura = str(row.get("ability_ura") or "")
        if ability_ura:
            print(ability_ura)
            raise SystemExit(0)
PY
}

extract_descriptor_ref_by_name_mode() {
  local file="$1"
  local ability_name="$2"
  local call_mode="$3"
  python3 - "$file" "$ability_name" "$call_mode" <<'PY'
import json
import sys

path, expected, call_mode = sys.argv[1:4]
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
    if not isinstance(row, dict):
        continue
    if str(row.get("call_mode") or "") != call_mode:
        continue
    if matches(row):
        descriptor_ref = str(row.get("descriptor_ref") or "")
        if descriptor_ref:
            print(descriptor_ref)
            raise SystemExit(0)
PY
}

wait_caller_ability_name() {
  local provider_node="$1"
  local ability_name="$2"
  local stem="$3"
  local out="$OUT_DIR/${stem}.json"
  local err="$OUT_DIR/${stem}.err"
  local ability_ura=""
  for _ in $(seq 1 120); do
    if caller_cli "ability list --node '$provider_node' --format json" >"$out" 2>"$err"; then
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

wait_caller_ability_absent() {
  local provider_node="$1"
  local ability_name="$2"
  local stem="$3"
  local out="$OUT_DIR/${stem}.json"
  local err="$OUT_DIR/${stem}.err"
  local ability_ura=""
  for _ in $(seq 1 120); do
    if caller_cli "ability list --node '$provider_node' --format json" >"$out" 2>"$err"; then
      ability_ura="$(extract_ability_ura_by_name "$out" "$ability_name")"
      if [[ -z "$ability_ura" ]]; then
        return 0
      fi
    fi
    sleep 0.5
  done
  cat "$out" >&2 2>/dev/null || true
  cat "$err" >&2 2>/dev/null || true
  return 1
}

wait_file() {
  local path="$1"
  for _ in $(seq 1 120); do
    if [[ -s "$path" ]]; then
      return 0
    fi
    sleep 0.5
  done
  return 1
}

json_arg() {
  python3 - "$@" <<'PY'
import json, sys
kind = sys.argv[1]
if kind == "add":
    print(json.dumps({"a": int(sys.argv[2]), "b": int(sys.argv[3])}, separators=(",", ":")))
elif kind == "plugin_echo":
    print(json.dumps({"message": sys.argv[2]}, separators=(",", ":")))
elif kind == "agent_chat":
    print(json.dumps({"prompt": sys.argv[2], "skills": {"mode": "none"}}, separators=(",", ":")))
else:
    raise SystemExit(f"unknown json kind: {kind}")
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
CN = EasyNet Docker EasyRemote E2E CA
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
      PYTHONPATH: /work/EasyRemote:/work/EasyNet-Cli/sdk/python:/work/EasyNet-Axon/sdk/python
    volumes:
      - ${SHARED_DIR}:/shared
      - ${CERT_DIR}:/certs:ro
      - ${EASYREMOTE_ROOT}:/work/EasyRemote:ro
      - ${REPO_ROOT}:/work/EasyNet-Cli:ro
      - ${AXON_ROOT}:/work/EasyNet-Axon:ro
  caller:
    image: ${RUNTIME_IMAGE}
    hostname: caller
    entrypoint: ["/bin/sh", "-lc"]
    command: ["mkdir -p /home/caller /shared && tail -f /dev/null"]
    environment:
      HOME: /home/caller
      EASYNET_CLI_LIB: /usr/local/lib/libeasynet_cli.so
      PYTHONPATH: /work/EasyRemote:/work/EasyNet-Cli/sdk/python:/work/EasyNet-Axon/sdk/python
    volumes:
      - ${SHARED_DIR}:/shared
      - ${CERT_DIR}:/certs:ro
      - ${EASYREMOTE_ROOT}:/work/EasyRemote:ro
      - ${REPO_ROOT}:/work/EasyNet-Cli:ro
      - ${AXON_ROOT}:/work/EasyNet-Axon:ro
YAML

echo "==> starting Hub/provider/caller topology project=$PROJECT"
compose down -v --remove-orphans >/dev/null 2>&1 || true
compose up -d hub provider caller

echo "==> opening Hub daemon"
hub_cli "runtime start --as-hub --tenant '$REALM' --bind 0.0.0.0:50443 --cert /certs/hub.crt --key /certs/hub.key" \
  >"$OUT_DIR/hub-start.txt" 2>"$OUT_DIR/hub-start.err"
wait_hub_port_from caller
wait_runtime hub /srv/easynet hub

echo "==> bootstrapping PrincipalLifecycle"
hub_cli "principal bootstrap --principal-ura '$ADMIN_URA' --create-idempotency-key admin-create-$TIMESTAMP --bind-idempotency-key admin-bind-$TIMESTAMP --json" \
  >"$OUT_DIR/principal-admin.json"
ADMIN_BINDING="$(jq -r '.principal.bindings[0].binding_id' "$OUT_DIR/principal-admin.json")"
hub_cli "principal issue-enrollment --issuer-ura '$ADMIN_URA' --subject-principal-ura '$USER_URA' --proof-ref '$ADMIN_BINDING' --idempotency-key alice-principal-$TIMESTAMP --json" \
  >"$OUT_DIR/enrollment-alice-principal.json"
ALICE_ENROLLMENT="$(jq -r '.principal.enrollments[-1].enrollment_id' "$OUT_DIR/enrollment-alice-principal.json")"
hub_cli "principal enroll --principal-ura '$USER_URA' --enrollment-id '$ALICE_ENROLLMENT' --create-idempotency-key alice-create-$TIMESTAMP --bind-idempotency-key alice-bind-$TIMESTAMP --json" \
  >"$OUT_DIR/principal-alice.json"
PROVIDER_ENROLLMENT="$(issue_device_enrollment provider)"
CALLER_ENROLLMENT="$(issue_device_enrollment caller)"

echo "==> joining provider and caller devices by Hub URA"
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
printf '%s\n' "$PROVIDER_NODE" >"$SHARED_DIR/provider-node-id.txt"
printf '%s\n' "$PROVIDER_URA" >"$SHARED_DIR/provider-device-ura.txt"
printf '%s\n' "$CALLER_URA" >"$SHARED_DIR/caller-device-ura.txt"

echo "==> starting provider and caller daemons"
provider_cli "runtime start" >"$OUT_DIR/provider-start.txt" 2>"$OUT_DIR/provider-start.err"
caller_cli "runtime start" >"$OUT_DIR/caller-start.txt" 2>"$OUT_DIR/caller-start.err"
wait_runtime provider /home/provider provider
wait_runtime caller /home/caller caller
wait_hub_device "$PROVIDER_NODE"
wait_hub_device "$CALLER_NODE"

echo "==> exercising custom caller-side CLI agent CRUD"
cat >"$SHARED_DIR/fake-agent.sh" <<'EOF'
#!/usr/bin/env sh
agent="${1:-agent}"
prompt="$(cat)"
printf 'docker-easyremote-agent[%s]: %s\n' "$agent" "$prompt"
EOF
chmod +x "$SHARED_DIR/fake-agent.sh"
AGENT_NAME="er_agent_${TIMESTAMP}"
TMP_AGENT="er_delete_${TIMESTAMP}"
caller_cli "agent add '$AGENT_NAME' --type external --command /shared/fake-agent.sh --arg '$AGENT_NAME' --label '$AGENT_NAME'" \
  >"$OUT_DIR/agent-add.txt" 2>"$OUT_DIR/agent-add.err"
caller_cli "agent set '$AGENT_NAME' --model e2e-updated-model" >"$OUT_DIR/agent-set.txt" 2>"$OUT_DIR/agent-set.err"
caller_cli "agent list" >"$OUT_DIR/agent-list.txt" 2>"$OUT_DIR/agent-list.err"
caller_cli "agent send '$AGENT_NAME' 'hello from docker easyremote caller e2e'" >"$OUT_DIR/agent-send.txt" 2>"$OUT_DIR/agent-send.err"
caller_cli "agent add '$TMP_AGENT' --type external --command /shared/fake-agent.sh --arg '$TMP_AGENT' --label '$TMP_AGENT'" \
  >"$OUT_DIR/agent-temp-add.txt" 2>"$OUT_DIR/agent-temp-add.err"
caller_cli "agent remove '$TMP_AGENT' --purge" >"$OUT_DIR/agent-temp-remove.txt" 2>"$OUT_DIR/agent-temp-remove.err"
caller_cli "agent list" >"$OUT_DIR/agent-list-after-temp-remove.txt" 2>"$OUT_DIR/agent-list-after-temp-remove.err"

echo "==> registering provider-hosted user external Agent and invoking it from caller"
PROVIDER_AGENT_NAME="provider_agent_${TIMESTAMP}"
PROVIDER_AGENT_CHAT_ABILITY="${PROVIDER_AGENT_NAME}.chat"
PROVIDER_AGENT_PROMPT="hello provider agent from caller ${TIMESTAMP}"
provider_cli "agent add '$PROVIDER_AGENT_NAME' --type external --command /shared/fake-agent.sh --arg '$PROVIDER_AGENT_NAME' --label '$PROVIDER_AGENT_NAME'" \
  >"$OUT_DIR/provider-agent-add.txt" 2>"$OUT_DIR/provider-agent-add.err"
provider_cli "agent list" >"$OUT_DIR/provider-agent-list-after-add.txt" 2>"$OUT_DIR/provider-agent-list-after-add.err"
provider_cli "agent send '$PROVIDER_AGENT_NAME' '$PROVIDER_AGENT_PROMPT'" \
  >"$OUT_DIR/provider-agent-send.txt" 2>"$OUT_DIR/provider-agent-send.err"
PROVIDER_AGENT_CHAT_URA="$(wait_caller_ability_name "$PROVIDER_URA" "$PROVIDER_AGENT_CHAT_ABILITY" "caller-ability-list-provider-agent")"
[[ "$PROVIDER_AGENT_CHAT_URA" == easynet://* ]] || die "provider user Agent chat ability was not discovered as a canonical URA: $PROVIDER_AGENT_CHAT_URA"
caller_cli "ability show '$PROVIDER_AGENT_CHAT_URA' --node '$PROVIDER_URA' --format json" \
  >"$OUT_DIR/caller-ability-show-provider-agent-chat.json" 2>"$OUT_DIR/caller-ability-show-provider-agent-chat.err"
PROVIDER_AGENT_SUBJECT_URA="$(jq -r '.owner_ura' "$OUT_DIR/caller-ability-show-provider-agent-chat.json")"
[[ "$PROVIDER_AGENT_SUBJECT_URA" == easynet://*/agent/* ]] || die "provider Agent chat show did not expose valid owner Agent URA subject"
PROVIDER_AGENT_CHAT_DESCRIPTOR_REF="$(extract_descriptor_ref_by_name_mode "$OUT_DIR/caller-ability-list-provider-agent.json" "$PROVIDER_AGENT_CHAT_ABILITY" stream)"
if [[ -z "$PROVIDER_AGENT_CHAT_DESCRIPTOR_REF" ]]; then
  PROVIDER_AGENT_CHAT_DESCRIPTOR_REF="$(jq -r '
  .descriptor_ref //
  (.ability_ura + "@" + .version + "#" + (.descriptor_hash | sub("^sha256:"; "")) + "!" + .admission_action)
' "$OUT_DIR/caller-ability-show-provider-agent-chat.json")"
fi
[[ "$PROVIDER_AGENT_CHAT_DESCRIPTOR_REF" == easynet://*@*#*!* ]] || die "caller ability show did not expose descriptor-bound provider Agent chat ref"
PROVIDER_AGENT_NONCE_HEX="$(random_nonce_hex)"
set +e
provider_cli "ability stream '$PROVIDER_AGENT_CHAT_URA' --subject '$PROVIDER_AGENT_SUBJECT_URA' --nonce-hex '$PROVIDER_AGENT_NONCE_HEX' --causal-root --args '$(json_arg agent_chat "$PROVIDER_AGENT_PROMPT")' --format json --raw" \
  >"$OUT_DIR/provider-cli-provider-agent-chat-stream.json" 2>"$OUT_DIR/provider-cli-provider-agent-chat-stream.err"
PROVIDER_AGENT_CANONICAL_STATUS=$?
set -e
printf '%s\n' "$PROVIDER_AGENT_CANONICAL_STATUS" >"$OUT_DIR/provider-cli-provider-agent-chat-stream.status"
provider_cli "invocation list --ability-ura '$PROVIDER_AGENT_CHAT_URA' --format json" \
  >"$OUT_DIR/provider-invocation-list-provider-agent-after-cli.json" 2>"$OUT_DIR/provider-invocation-list-provider-agent-after-cli.err"
provider_cli "invocation list --format json" \
  >"$OUT_DIR/provider-invocation-list-all-after-provider-agent-cli.json" 2>"$OUT_DIR/provider-invocation-list-all-after-provider-agent-cli.err"
provider_cli "agent remove '$PROVIDER_AGENT_NAME' --purge" >"$OUT_DIR/provider-agent-remove.txt" 2>"$OUT_DIR/provider-agent-remove.err"
provider_cli "agent list" >"$OUT_DIR/provider-agent-list-after-remove.txt" 2>"$OUT_DIR/provider-agent-list-after-remove.err"
wait_caller_ability_absent "$PROVIDER_URA" "$PROVIDER_AGENT_CHAT_ABILITY" "caller-ability-list-provider-after-agent-remove"

echo "==> publishing native EasyNet host_stream ability from provider via CLI deploy"
NATIVE_NAMESPACE="nativeer"
NATIVE_FUNCTION="native_echo"
NATIVE_QUALIFIED="${NATIVE_NAMESPACE}.${NATIVE_FUNCTION}"
NATIVE_BUNDLE="/shared/native-easynet-ability"
NATIVE_SOCKET="/tmp/native-easynet-host/host.sock"
mkdir -p "$SHARED_DIR/native-easynet-ability"
cat >"$SHARED_DIR/native_easynet_host.py" <<'PY'
import json
import signal
import threading
from pathlib import Path

from easyremote import Context
from easyremote._host.server import HostServer, HostedFunction
from easyremote.schema import derive

ready = Path("/shared/native-easynet-ready.json")
stop = threading.Event()
server = HostServer(Path("/tmp/native-easynet-host/host.sock"))

def native_echo(ctx: Context, text: str, times: int = 1) -> dict[str, str | int]:
    return {
        "source": "easynet-cli-deploy",
        "text": text,
        "times": times,
        "joined": text * times,
        "caller": ctx.caller,
        "invocation_id": ctx.invocation_id,
    }

def handle_stop(signum, frame):
    stop.set()

signal.signal(signal.SIGTERM, handle_stop)
signal.signal(signal.SIGINT, handle_stop)
server.add(
    HostedFunction(
        name="nativeer.native_echo",
        fn=native_echo,
        signature=derive(native_echo, context_type=Context),
    )
)
server.start()
ready.write_text(
    json.dumps(
        {
            "publisher": "easynet ability deploy",
            "function": "nativeer.native_echo",
            "host_socket": str(server.socket_path),
        },
        sort_keys=True,
    )
    + "\n",
    encoding="utf-8",
)
print("READY " + ready.read_text(encoding="utf-8").strip(), flush=True)
try:
    stop.wait()
finally:
    server.stop()
PY
cat >"$SHARED_DIR/native-easynet-ability/ability.json" <<JSON
{
  "name": "${NATIVE_FUNCTION}",
  "namespace": "${NATIVE_NAMESPACE}",
  "description": "Native EasyNet host_stream ability invoked by EasyRemote.",
  "admission_action": "stream",
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["text"],
    "properties": {
      "text": {"type": "string"},
      "times": {"type": "integer", "default": 1}
    },
    "x-easyremote-parameter-order": ["text", "times"]
  },
  "output_schema": {
    "type": "object",
    "required": ["source", "text", "times", "joined", "caller", "invocation_id"],
    "properties": {
      "source": {"type": "string"},
      "text": {"type": "string"},
      "times": {"type": "integer"},
      "joined": {"type": "string"},
      "caller": {"type": "string"},
      "invocation_id": {"type": "string"}
    }
  },
  "exec": {
    "kind": "host_stream",
    "host_socket": "${NATIVE_SOCKET}",
    "function": "${NATIVE_QUALIFIED}"
  }
}
JSON
service_exec provider "HOME=/home/provider EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so PYTHONPATH=/work/EasyRemote:/work/EasyNet-Cli/sdk/python:/work/EasyNet-Axon/sdk/python nohup python3 /shared/native_easynet_host.py > /shared/native-easynet-host.log 2>&1 & echo \$! > /shared/native-easynet-host.pid"
wait_file "$SHARED_DIR/native-easynet-ready.json" || die "native EasyNet host_stream server did not publish readiness"
cp "$SHARED_DIR/native-easynet-ready.json" "$OUT_DIR/native-easynet-ready.json"
provider_cli "ability deploy '$NATIVE_BUNDLE' --node local --format json" \
  >"$OUT_DIR/provider-native-deploy.json" 2>"$OUT_DIR/provider-native-deploy.err"
NATIVE_ABILITY_URA="$(jq -r '.ability_ura' "$OUT_DIR/provider-native-deploy.json")"
[[ "$NATIVE_ABILITY_URA" == easynet://* ]] || die "native EasyNet deploy returned invalid ability URA: $NATIVE_ABILITY_URA"

echo "==> installing and invoking user-defined declarative exec plugin"
USER_PLUGIN_ID="user.plugin.echo"
USER_PLUGIN_VERSION="0.1.0"
USER_PLUGIN_ABILITY="user_plugin.echo"
USER_PLUGIN_MESSAGE="hello-from-user-plugin-${TIMESTAMP}"
USER_PLUGIN_ROOT="/shared/user-plugin-echo"
provider_cli "plugin init '$USER_PLUGIN_ROOT' --id '$USER_PLUGIN_ID' --ability '$USER_PLUGIN_ABILITY'" \
  >"$OUT_DIR/provider-plugin-init-user-plugin.txt" 2>"$OUT_DIR/provider-plugin-init-user-plugin.err"
provider_cli "plugin install '$USER_PLUGIN_ROOT'" >"$OUT_DIR/provider-plugin-install-user-plugin.txt" 2>"$OUT_DIR/provider-plugin-install-user-plugin.err"
provider_cli "plugin list --format json" >"$OUT_DIR/provider-plugin-list-after-user-plugin-install.json" 2>"$OUT_DIR/provider-plugin-list-after-user-plugin-install.err"
provider_cli "plugin status '$USER_PLUGIN_ID' --version '$USER_PLUGIN_VERSION' --json" \
  >"$OUT_DIR/provider-plugin-status-user-plugin.json" 2>"$OUT_DIR/provider-plugin-status-user-plugin.err"
USER_PLUGIN_ABILITY_URA="$(wait_caller_ability_name "$PROVIDER_URA" "$USER_PLUGIN_ABILITY" "caller-ability-list-provider-plugin")"
[[ "$USER_PLUGIN_ABILITY_URA" == easynet://* ]] || die "user plugin ability was not discovered as a canonical URA: $USER_PLUGIN_ABILITY_URA"
caller_cli "ability show '$USER_PLUGIN_ABILITY_URA' --node '$PROVIDER_URA' --format json" \
  >"$OUT_DIR/caller-ability-show-user-plugin.json" 2>"$OUT_DIR/caller-ability-show-user-plugin.err"
USER_PLUGIN_DESCRIPTOR_REF="$(jq -r '
  .descriptor_ref //
  (.ability_ura + "@" + .version + "#" + (.descriptor_hash | sub("^sha256:"; "")) + "!" + .admission_action)
' "$OUT_DIR/caller-ability-show-user-plugin.json")"
[[ "$USER_PLUGIN_DESCRIPTOR_REF" == easynet://*@*#*!* ]] || die "caller ability show did not expose descriptor-bound user plugin ref"
USER_PLUGIN_NONCE_HEX="$(random_nonce_hex)"
caller_cli "ability stream '$USER_PLUGIN_DESCRIPTOR_REF' --node '$PROVIDER_URA' --subject 'easynet:///r/${REALM}/resource/e2e/user-plugin/echo' --nonce-hex '$USER_PLUGIN_NONCE_HEX' --causal-root --args '$(json_arg plugin_echo "$USER_PLUGIN_MESSAGE")' --format json --raw" \
  >"$OUT_DIR/caller-cli-user-plugin-stream.json" 2>"$OUT_DIR/caller-cli-user-plugin-stream.err"
provider_cli "invocation list --ability-ura '$USER_PLUGIN_ABILITY_URA' --format json" \
  >"$OUT_DIR/provider-invocation-list-user-plugin-after-cli.json" 2>"$OUT_DIR/provider-invocation-list-user-plugin-after-cli.err"
provider_cli "invocation list --format json" \
  >"$OUT_DIR/provider-invocation-list-all-after-user-plugin-cli.json" 2>"$OUT_DIR/provider-invocation-list-all-after-user-plugin-cli.err"
provider_cli "plugin remove '$USER_PLUGIN_ID' '$USER_PLUGIN_VERSION'" >"$OUT_DIR/provider-plugin-remove-user-plugin.txt" 2>"$OUT_DIR/provider-plugin-remove-user-plugin.err"
provider_cli "plugin list --format json" >"$OUT_DIR/provider-plugin-list-after-user-plugin-remove.json" 2>"$OUT_DIR/provider-plugin-list-after-user-plugin-remove.err"
wait_caller_ability_absent "$PROVIDER_URA" "$USER_PLUGIN_ABILITY" "caller-ability-list-provider-after-plugin-remove"

echo "==> starting EasyRemote ComputeNode provider with @node.register matrix"
cat >"$SHARED_DIR/easyremote_provider.py" <<'PY'
import asyncio
import json
import signal
import threading
from pathlib import Path
from typing import Any

from easyremote import ComputeNode, Context

ready = Path("/shared/easyremote-ready.json")
stop = threading.Event()
node = ComputeNode(namespace="er")

def marker(message: str) -> None:
    print("PROVIDER_STAGE " + message, flush=True)

@node.register
def add(a: int, b: int) -> dict[str, str | int]:
    return {"value": a + b, "kind": "sync", "function": "add"}

@node.register
def total(*nums: int) -> int:
    return sum(nums)

@node.register
def merge(base: int, **extra: int) -> dict[str, int]:
    return {"base": base, **extra}

@node.register(name="defaulted", description="Default-argument function.")
def greet(name: str, punctuation: str = "!") -> str:
    return f"hello {name}{punctuation}"

@node.register
async def summarize(text: str, max_words: int = 3) -> str:
    await asyncio.sleep(0)
    words = text.split()
    return " ".join(words[:max_words])

@node.register
def bundle(items: list[int], meta: dict[str, Any]) -> dict[str, Any]:
    return {"count": len(items), "sum": sum(items), "meta": meta}

@node.register
def countdown(n: int):
    for i in range(n, 0, -1):
        yield {"tick": i}

@node.register
def whoami(ctx: Context, note: str) -> dict[str, str]:
    return {
        "note": note,
        "caller": ctx.caller,
        "invocation_id": ctx.invocation_id,
    }

def handle_stop(signum, frame):
    stop.set()

signal.signal(signal.SIGTERM, handle_stop)
signal.signal(signal.SIGINT, handle_stop)
marker("before_node_start")
node.start()
marker("after_node_start")
abilities = {
    ability.name: {
        "ura": ability.ura,
        "qualified_name": ability.qualified_name,
        "package_dir": str(ability.package_dir),
    }
    for ability in node.abilities
}
ready.write_text(
    json.dumps(
        {
            "decorator": "@node.register",
            "function_shapes": [
                "sync",
                "variadic_args",
                "variadic_kwargs",
                "default_args",
                "async",
                "list_dict_payload",
                "generator_stream",
                "context",
            ],
            "host_socket": str(node.host_socket),
            "abilities": abilities,
        },
        sort_keys=True,
    )
    + "\n",
    encoding="utf-8",
)
print("READY " + ready.read_text(encoding="utf-8").strip(), flush=True)
try:
    stop.wait()
finally:
    node.stop()
PY
service_exec provider "HOME=/home/provider EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so PYTHONPATH=/work/EasyRemote:/work/EasyNet-Cli/sdk/python:/work/EasyNet-Axon/sdk/python nohup python3 -u /shared/easyremote_provider.py > /shared/easyremote-provider.log 2>&1 & echo \$! > /shared/easyremote-provider.pid"
wait_file "$SHARED_DIR/easyremote-ready.json" || die "EasyRemote provider did not publish readiness"
cp "$SHARED_DIR/easyremote-ready.json" "$OUT_DIR/easyremote-ready.json"
ADD_URA="$(jq -r '.abilities.add.ura' "$OUT_DIR/easyremote-ready.json")"
TOTAL_URA="$(jq -r '.abilities.total.ura' "$OUT_DIR/easyremote-ready.json")"
MERGE_URA="$(jq -r '.abilities.merge.ura' "$OUT_DIR/easyremote-ready.json")"
DEFAULTED_URA="$(jq -r '.abilities.defaulted.ura' "$OUT_DIR/easyremote-ready.json")"
SUMMARIZE_URA="$(jq -r '.abilities.summarize.ura' "$OUT_DIR/easyremote-ready.json")"
BUNDLE_URA="$(jq -r '.abilities.bundle.ura' "$OUT_DIR/easyremote-ready.json")"
COUNTDOWN_URA="$(jq -r '.abilities.countdown.ura' "$OUT_DIR/easyremote-ready.json")"
WHOAMI_URA="$(jq -r '.abilities.whoami.ura' "$OUT_DIR/easyremote-ready.json")"
for ability_ura in "$ADD_URA" "$TOTAL_URA" "$MERGE_URA" "$DEFAULTED_URA" "$SUMMARIZE_URA" "$BUNDLE_URA" "$COUNTDOWN_URA" "$WHOAMI_URA"; do
  [[ "$ability_ura" == easynet://* ]] || die "EasyRemote returned invalid ability URA: $ability_ura"
done

echo "==> querying provider abilities from caller device via CLI"
wait_caller_remote_ability_list "$PROVIDER_URA"
caller_cli "ability show '$ADD_URA' --node '$PROVIDER_URA' --format json" \
  >"$OUT_DIR/caller-ability-show-add.json" 2>"$OUT_DIR/caller-ability-show-add.err"
caller_cli "ability show '$NATIVE_ABILITY_URA' --node '$PROVIDER_URA' --format json" \
  >"$OUT_DIR/caller-ability-show-native.json" 2>"$OUT_DIR/caller-ability-show-native.err"
ADD_DESCRIPTOR_REF="$(jq -r '
  .descriptor_ref //
  (.ability_ura + "@" + .version + "#" + (.descriptor_hash | sub("^sha256:"; "")) + "!" + .admission_action)
' "$OUT_DIR/caller-ability-show-add.json")"
[[ "$ADD_DESCRIPTOR_REF" == easynet://*@*#*!* ]] || die "caller ability show did not expose descriptor-bound add ref"

echo "==> calling provider ability from caller device through CLI"
caller_cli "ability stream '$ADD_DESCRIPTOR_REF' --node '$PROVIDER_URA' --subject 'easynet:///r/${REALM}/resource/e2e/docker-easyremote-cli/add' --nonce-hex 00000000000000000000000000000002 --causal-root --args '$(json_arg add 19 23)' --format json --raw" \
  >"$OUT_DIR/caller-cli-add-stream.json" 2>"$OUT_DIR/caller-cli-add-stream.err"
provider_cli "invocation list --ability-ura '$ADD_URA' --format json" \
  >"$OUT_DIR/provider-invocation-list-add-after-cli.json" 2>"$OUT_DIR/provider-invocation-list-add-after-cli.err"
provider_cli "invocation list --format json" \
  >"$OUT_DIR/provider-invocation-list-all-after-cli.json" 2>"$OUT_DIR/provider-invocation-list-all-after-cli.err"

echo "==> calling EasyRemote provider from caller device with @remote syntax matrix"
cat >"$SHARED_DIR/easyremote_caller.py" <<'PY'
import ast
import json
import os
from pathlib import Path

from easyremote import Client, FreshRoot, ResolvedTargetSubject, remote

provider_node = os.environ["PROVIDER_NODE"]
provider_ura = os.environ["PROVIDER_URA"]
native_ability_ura = os.environ["NATIVE_ABILITY_URA"]
client = Client(invocation_policy=FreshRoot(ResolvedTargetSubject()))
provider = client.device(provider_node)
native = provider.ability("nativeer.native_echo")

FORBIDDEN_DEFINITIONS = {
    "AdmissionAuthority",
    "CanonicalReceipt",
    "DaemonLifecycle",
    "HubLifecycle",
    "LifecycleAuthority",
    "ReceiptCanonicalizer",
    "ReceiptChain",
    "RouteAuthority",
}
FORBIDDEN_FUNCTIONS = {
    "admit_invocation",
    "canonicalize_receipt",
    "finalize_receipt",
    "rebuild_receipt",
    "route_invocation",
    "sign_receipt",
    "start_hub",
    "stop_hub",
    "verify_receipt",
}


def audit_easyremote_sdk_consumer_boundary() -> dict[str, object]:
    """Allow SDK projections; reject EasyRemote-owned runtime authority."""
    package = Path("/work/EasyRemote/easyremote")
    violations: list[str] = []
    for source in sorted(package.rglob("*.py")):
        rel = source.relative_to(package).as_posix()
        tree = ast.parse(source.read_text(encoding="utf-8"), filename=str(source))
        for node in ast.walk(tree):
            if isinstance(node, ast.ClassDef) and node.name in FORBIDDEN_DEFINITIONS:
                violations.append(f"{rel}:{node.lineno}:forbidden_easyremote_authority:{node.name}")
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name in FORBIDDEN_FUNCTIONS:
                violations.append(f"{rel}:{node.lineno}:forbidden_easyremote_authority:{node.name}")
            if isinstance(node, ast.Import):
                for alias in node.names:
                    if alias.name in {"ctypes", "cffi"}:
                        violations.append(f"{rel}:{node.lineno}:raw_runtime_binding_import:{alias.name}")
            if isinstance(node, ast.ImportFrom) and node.module in {"ctypes", "cffi"}:
                violations.append(f"{rel}:{node.lineno}:raw_runtime_binding_import:{node.module}")
    return {
        "package": str(package),
        "violations": violations,
        "ok": not violations,
    }


def function_info_to_dict(info) -> dict[str, object]:
    return {
        "name": info.name,
        "qualified_name": info.qualified_name,
        "ability_ura": info.ability_ura,
        "owner": info.owner,
        "description": info.description,
        "input_schema": info.input_schema,
        "visibility": info.visibility,
        "score": info.score,
    }


def ability_record_to_dict(record) -> dict[str, object]:
    return {
        "name": record.name,
        "ability_ura": record.ability_ura,
        "owner": record.owner_ura,
        "description": record.description,
        "input_schema": dict(record.input_schema or {}),
        "state": record.state,
        "metadata": dict(record.metadata or {}),
    }

@remote(client=client, owner_ura=provider_ura, name="add")
def remote_add(a: int, b: int) -> dict[str, object]: ...

@provider.remote(name="defaulted")
def defaulted(name: str, punctuation: str = "!") -> str: ...

@provider.remote
def total(*nums: int) -> int: ...

@provider.remote
def merge(base: int, **extra: int) -> dict[str, int]: ...

@provider.remote
def summarize(text: str, max_words: int = 3) -> str: ...

@provider.remote
def bundle(items: list[int], meta: dict[str, object]) -> dict[str, object]: ...

@provider.remote
def countdown(n: int): ...

@provider.remote
def whoami(note: str) -> dict[str, object]: ...

@native.remote
def native_echo(text: str, times: int = 1) -> dict[str, object]: ...

functions_user = [function_info_to_dict(info) for info in client.functions(scope="user")]
provider_catalog = [
    ability_record_to_dict(record)
    for record in client.abilities.list_device(provider_ura)
]
native_catalog_discovery = [
    info for info in provider_catalog if info.get("ability_ura") == native_ability_ura
]

results = {
    "decorators": ["@remote", "RemoteOwner.remote"],
    "provider_node": provider_node,
    "provider_ura": provider_ura,
    "easyremote_sdk_consumer_boundary": audit_easyremote_sdk_consumer_boundary(),
    "functions_scope_user": functions_user,
    "provider_catalog": provider_catalog,
    "native_easynet_discovery": native_catalog_discovery,
    "add": remote_add(2, 3),
    "total_varargs": total(1, 2, 3, 4),
    "merge_kwargs": merge(10, x=1, y=2),
    "defaulted_default_arg": defaulted("device"),
    "summarize_async": summarize("one two three four five", max_words=4),
    "bundle_list_dict": bundle([1, 2, 3], {"label": "caller", "ok": True}),
    "countdown_stream": list(countdown.stream(3)),
    "whoami_context": whoami("from-caller-device"),
    "native_easynet": {
        "ability_ura": native_ability_ura,
        "handle_call": native.call(text="from-handle", times=2),
        "handle_stream": list(native.stream(text="from-stream", times=2)),
        "canonical_ura_call": client.call(native_ability_ura, text="from-ura", times=1),
        "canonical_ura_stream": list(client.stream(native_ability_ura, text="from-ura-stream", times=1)),
        "typed_stub": native_echo("from-stub", times=3),
        "typed_stub_stream": list(native_echo.stream("from-stub-stream", times=1)),
    },
}
print(json.dumps(results, indent=2, sort_keys=True))
PY
service_exec caller "HOME=/home/caller EASYNET_CLI_LIB=/usr/local/lib/libeasynet_cli.so PYTHONPATH=/work/EasyRemote:/work/EasyNet-Cli/sdk/python:/work/EasyNet-Axon/sdk/python PROVIDER_NODE='$PROVIDER_NODE' PROVIDER_URA='$PROVIDER_URA' NATIVE_ABILITY_URA='$NATIVE_ABILITY_URA' python3 /shared/easyremote_caller.py" \
  >"$OUT_DIR/easyremote-remote-results.json" 2>"$SHARED_DIR/easyremote-caller.log"
cp "$SHARED_DIR/easyremote-caller.log" "$OUT_DIR/easyremote-caller.log"
provider_cli "invocation list --ability-ura '$NATIVE_ABILITY_URA' --format json" \
  >"$OUT_DIR/provider-invocation-list-native-after-easyremote.json" 2>"$OUT_DIR/provider-invocation-list-native-after-easyremote.err"
provider_cli "invocation list --format json" \
  >"$OUT_DIR/provider-invocation-list-all-after-easyremote.json" 2>"$OUT_DIR/provider-invocation-list-all-after-easyremote.err"

echo "==> uninstalling one provider EasyRemote ability and verifying caller sees removal"
provider_cli "ability uninstall '$ADD_URA' --yes" >"$OUT_DIR/provider-ability-uninstall-add.json" 2>"$OUT_DIR/provider-ability-uninstall-add.err"
caller_cli "ability list --node '$PROVIDER_URA' --format json" \
  >"$OUT_DIR/caller-ability-list-provider-after-uninstall.json" 2>"$OUT_DIR/caller-ability-list-provider-after-uninstall.err"
provider_cli "ability uninstall '$NATIVE_ABILITY_URA' --yes" >"$OUT_DIR/provider-ability-uninstall-native.json" 2>"$OUT_DIR/provider-ability-uninstall-native.err"
caller_cli "ability list --node '$PROVIDER_URA' --format json" \
  >"$OUT_DIR/caller-ability-list-provider-after-native-uninstall.json" 2>"$OUT_DIR/caller-ability-list-provider-after-native-uninstall.err"

python3 - "$OUT_DIR" "$PROVIDER_NODE" "$CALLER_NODE" "$PROVIDER_URA" "$CALLER_URA" "$AGENT_NAME" "$TMP_AGENT" "$HUB_URA" \
  "$PROVIDER_AGENT_NAME" "$PROVIDER_AGENT_CHAT_ABILITY" "$PROVIDER_AGENT_CHAT_URA" "$PROVIDER_AGENT_PROMPT" \
  "$USER_PLUGIN_ID" "$USER_PLUGIN_VERSION" "$USER_PLUGIN_ABILITY" "$USER_PLUGIN_ABILITY_URA" "$USER_PLUGIN_MESSAGE" \
  "$NATIVE_ABILITY_URA" "$ADD_URA" "$TOTAL_URA" "$MERGE_URA" "$DEFAULTED_URA" "$SUMMARIZE_URA" "$BUNDLE_URA" "$COUNTDOWN_URA" "$WHOAMI_URA" <<'PY' >"$OUT_DIR/report.json"
import json
import sys
from pathlib import Path

out = Path(sys.argv[1])
provider_node, caller_node, provider_ura, caller_ura, agent_name, tmp_agent, hub_ura = sys.argv[2:9]
provider_agent_name, provider_agent_chat_ability, provider_agent_chat_ura, provider_agent_prompt = sys.argv[9:13]
user_plugin_id, user_plugin_version, user_plugin_ability, user_plugin_ability_ura, user_plugin_message = sys.argv[13:18]
native_ability_ura = sys.argv[18]
ability_uras = {
    "add": sys.argv[19],
    "total": sys.argv[20],
    "merge": sys.argv[21],
    "defaulted": sys.argv[22],
    "summarize": sys.argv[23],
    "bundle": sys.argv[24],
    "countdown": sys.argv[25],
    "whoami": sys.argv[26],
}

def text(name: str) -> str:
    path = out / name
    return path.read_text(encoding="utf-8", errors="replace") if path.exists() else ""

def load(name: str):
    data = text(name).strip()
    return json.loads(data) if data else None

def contains(name: str, needle: str) -> bool:
    return needle in text(name)

def cli_contains(stem: str, needle: str) -> bool:
    return contains(f"{stem}.txt", needle) or contains(f"{stem}.err", needle)

def ability_rows(name: str):
    payload = load(name)
    if isinstance(payload, list):
        return payload
    if isinstance(payload, dict):
        for key in ("items", "abilities", "records"):
            value = payload.get(key)
            if isinstance(value, list):
                return value
    return []

def invocation_records(name: str):
    payload = load(name) or []
    if isinstance(payload, dict):
        return payload.get("records") or payload.get("items") or []
    return payload if isinstance(payload, list) else []

def ability_invocation_records(exact_name: str, fallback_name: str, ability_ura: str):
    exact = invocation_records(exact_name)
    if exact:
        return exact
    records = invocation_records(fallback_name)
    matched = []
    for record in records:
        if not isinstance(record, dict):
            continue
        fields = [
            str(record.get("ability_ura") or ""),
            str(record.get("ability_name") or ""),
            str(record.get("descriptor_ref") or ""),
        ]
        if any(ability_ura == field or ability_ura in field for field in fields):
            matched.append(record)
            continue
        if ability_ura in json.dumps(record, separators=(",", ":"), sort_keys=True):
            matched.append(record)
    return matched

def payload_contains(value, needle: str) -> bool:
    return needle in json.dumps(value, separators=(",", ":"), sort_keys=True)

def payload_has_field(value, key: str, expected) -> bool:
    if isinstance(value, dict):
        if value.get(key) == expected:
            return True
        return any(payload_has_field(item, key, expected) for item in value.values())
    if isinstance(value, list):
        return any(payload_has_field(item, key, expected) for item in value)
    return False

def runtime_unary_payload(value):
    if isinstance(value, list):
        payloads = [item for item in value if item is not None]
        if len(payloads) == 1:
            return payloads[0]
        return payloads
    return value

def runtime_stream_payloads(value):
    if isinstance(value, list):
        return [item for item in value if item is not None]
    if value is None:
        return []
    return [value]

ready = load("easyremote-ready.json") or {}
native_ready = load("native-easynet-ready.json") or {}
native_deploy = load("provider-native-deploy.json") or {}
provider_agent_stream = load("provider-cli-provider-agent-chat-stream.json") or []
provider_agent_show = load("caller-ability-show-provider-agent-chat.json") or {}
provider_agent_canonical_status = text("provider-cli-provider-agent-chat-stream.status").strip()
provider_agent_canonical_error = text("provider-cli-provider-agent-chat-stream.err")
user_plugin_list_after_install = load("provider-plugin-list-after-user-plugin-install.json") or {}
user_plugin_status = load("provider-plugin-status-user-plugin.json") or {}
user_plugin_stream = load("caller-cli-user-plugin-stream.json") or []
user_plugin_show = load("caller-ability-show-user-plugin.json") or {}
remote_results = load("easyremote-remote-results.json") or {}
cli_stream = load("caller-cli-add-stream.json") or []
provider_agent_records = ability_invocation_records(
    "provider-invocation-list-provider-agent-after-cli.json",
    "provider-invocation-list-all-after-provider-agent-cli.json",
    provider_agent_chat_ura,
)
user_plugin_records = ability_invocation_records(
    "provider-invocation-list-user-plugin-after-cli.json",
    "provider-invocation-list-all-after-user-plugin-cli.json",
    user_plugin_ability_ura,
)
cli_add_records = ability_invocation_records(
    "provider-invocation-list-add-after-cli.json",
    "provider-invocation-list-all-after-cli.json",
    ability_uras["add"],
)
native_records = ability_invocation_records(
    "provider-invocation-list-native-after-easyremote.json",
    "provider-invocation-list-all-after-easyremote.json",
    native_ability_ura,
)
provider_agent_rows = ability_rows("caller-ability-list-provider-agent.json")
provider_agent_rows_after_remove = ability_rows("caller-ability-list-provider-after-agent-remove.json")
user_plugin_rows = ability_rows("caller-ability-list-provider-plugin.json")
user_plugin_rows_after_remove = ability_rows("caller-ability-list-provider-after-plugin-remove.json")
caller_rows = ability_rows("caller-ability-list-provider.json")
caller_rows_after_uninstall = ability_rows("caller-ability-list-provider-after-uninstall.json")
caller_rows_after_native_uninstall = ability_rows("caller-ability-list-provider-after-native-uninstall.json")

def row_has_ura(rows, ability_ura: str) -> bool:
    return any(
        isinstance(row, dict)
        and (row.get("ability_ura") == ability_ura or ability_ura in json.dumps(row))
        for row in rows
    )

all_provider_abilities_visible = all(row_has_ura(caller_rows, ura) for ura in ability_uras.values())
native_visible = row_has_ura(caller_rows, native_ability_ura)
provider_agent_visible = row_has_ura(provider_agent_rows, provider_agent_chat_ura)
provider_agent_removed = not row_has_ura(provider_agent_rows_after_remove, provider_agent_chat_ura)
user_plugin_visible = row_has_ura(user_plugin_rows, user_plugin_ability_ura)
user_plugin_removed = not row_has_ura(user_plugin_rows_after_remove, user_plugin_ability_ura)
add_removed = not row_has_ura(caller_rows_after_uninstall, ability_uras["add"])
other_abilities_remain = all(
    row_has_ura(caller_rows_after_uninstall, ability_uras[name])
    for name in ("total", "merge", "defaulted", "summarize", "bundle", "countdown", "whoami")
)
native_removed = not row_has_ura(caller_rows_after_native_uninstall, native_ability_ura)
def plugin_package_rows(report):
    if isinstance(report, dict):
        value = report.get("packages")
        return value if isinstance(value, list) else []
    return []

def plugin_ability_rows(report):
    if isinstance(report, dict):
        value = report.get("abilities")
        return value if isinstance(value, list) else []
    return []

user_plugin_package_loaded = any(
    isinstance(row, dict)
    and row.get("package_id") == user_plugin_id
    and row.get("package_version") == user_plugin_version
    and row.get("kind") == "declarative"
    and row.get("daemon_runtime_status") == "loaded"
    and row.get("runtime_published") is True
    and row.get("invokable") is True
    for row in plugin_package_rows(user_plugin_list_after_install)
)
user_plugin_ability_loaded = any(
    isinstance(row, dict)
    and row.get("package_id") == user_plugin_id
    and row.get("package_version") == user_plugin_version
    and row.get("ability") == user_plugin_ability
    and row.get("call_mode") == "rpc"
    and row.get("daemon_runtime_status") == "loaded"
    and row.get("runtime_published") is True
    and row.get("invokable") is True
    for row in plugin_ability_rows(user_plugin_list_after_install)
)
user_plugin_receipt_chains_verified = (
    len(user_plugin_records) == 1
    and str(user_plugin_records[0].get("state", "")).lower() == "completed"
    and isinstance(user_plugin_records[0].get("receipt_chain"), dict)
    and user_plugin_records[0]["receipt_chain"].get("verified") is True
    and isinstance(user_plugin_records[0]["receipt_chain"].get("anchors"), list)
    and len(user_plugin_records[0]["receipt_chain"]["anchors"]) > 0
)
provider_agent_receipt_chains_verified = (
    len(provider_agent_records) == 1
    and str(provider_agent_records[0].get("state", "")).lower() == "completed"
    and isinstance(provider_agent_records[0].get("receipt_chain"), dict)
    and provider_agent_records[0]["receipt_chain"].get("verified") is True
    and isinstance(provider_agent_records[0]["receipt_chain"].get("anchors"), list)
    and len(provider_agent_records[0]["receipt_chain"]["anchors"]) > 0
)
receipt_chains_verified = (
    len(cli_add_records) == 1
    and str(cli_add_records[0].get("state", "")).lower() == "completed"
    and isinstance(cli_add_records[0].get("receipt_chain"), dict)
    and cli_add_records[0]["receipt_chain"].get("verified") is True
    and isinstance(cli_add_records[0]["receipt_chain"].get("anchors"), list)
    and len(cli_add_records[0]["receipt_chain"]["anchors"]) > 0
)
native_receipt_chains_verified = (
    len(native_records) == 6
    and all(str(record.get("state", "")).lower() == "completed" for record in native_records)
    and all(isinstance(record.get("receipt_chain"), dict) for record in native_records)
    and all(record["receipt_chain"].get("verified") is True for record in native_records)
    and all(
        isinstance(record["receipt_chain"].get("anchors"), list)
        and len(record["receipt_chain"]["anchors"]) > 0
        for record in native_records
    )
)
native_results = remote_results.get("native_easynet") or {}
native_show = load("caller-ability-show-native.json") or {}
native_handle = runtime_unary_payload(native_results.get("handle_call")) or {}
native_handle_stream = runtime_stream_payloads(native_results.get("handle_stream"))
native_ura_call = runtime_unary_payload(native_results.get("canonical_ura_call")) or {}
native_ura_stream = runtime_stream_payloads(native_results.get("canonical_ura_stream"))
native_stub = runtime_unary_payload(native_results.get("typed_stub")) or {}
native_stub_stream = runtime_stream_payloads(native_results.get("typed_stub_stream"))
native_result_payloads = [
    native_handle,
    *native_handle_stream,
    native_ura_call,
    *native_ura_stream,
    native_stub,
    *native_stub_stream,
]
native_discovery = (
    [native_show]
    if isinstance(native_show, dict) and native_show.get("ability_ura") == native_ability_ura
    else []
)
functions_scope_user = remote_results.get("functions_scope_user") or []
provider_catalog = [native_show] if native_discovery else []
easyremote_boundary = runtime_unary_payload(remote_results.get("easyremote_sdk_consumer_boundary")) or {}
remote_add = runtime_unary_payload(remote_results.get("add")) or {}
remote_total_varargs = runtime_unary_payload(remote_results.get("total_varargs"))
remote_merge_kwargs = runtime_unary_payload(remote_results.get("merge_kwargs"))
remote_defaulted = runtime_unary_payload(remote_results.get("defaulted_default_arg"))
remote_summarize = runtime_unary_payload(remote_results.get("summarize_async"))
remote_bundle = runtime_unary_payload(remote_results.get("bundle_list_dict"))
remote_countdown = runtime_stream_payloads(remote_results.get("countdown_stream"))
remote_whoami = runtime_unary_payload(remote_results.get("whoami_context")) or {}
native_invocation_ids = [
    str(payload.get("invocation_id") or "")
    for payload in native_result_payloads
    if isinstance(payload, dict)
]
native_ledger_request_ids = [
    str(record.get("request_id") or "")
    for record in native_records
]
native_dispatch_ids_match_ledger = (
    len(native_invocation_ids) == 6
    and all(native_invocation_ids)
    and len(set(native_invocation_ids)) == 6
    and sorted(native_invocation_ids) == sorted(native_ledger_request_ids)
)

report = {
    "topology": {
        "containers": ["hub", "provider", "caller"],
        "hub_ura": hub_ura,
        "provider_node": provider_node,
        "provider_ura": provider_ura,
        "caller_node": caller_node,
        "caller_ura": caller_ura,
    },
    "native_easynet": {
        "host_ready": native_ready,
        "deploy": native_deploy,
        "ability_ura": native_ability_ura,
        "remote_results": native_results,
    },
    "user_agent": {
        "name": provider_agent_name,
        "chat_ability": provider_agent_chat_ability,
        "chat_ability_ura": provider_agent_chat_ura,
        "stream_frames": provider_agent_stream,
    },
    "user_plugin": {
        "package_id": user_plugin_id,
        "package_version": user_plugin_version,
        "ability": user_plugin_ability,
        "ability_ura": user_plugin_ability_ura,
        "status": user_plugin_status,
        "stream_frames": user_plugin_stream,
    },
    "easyremote": {
        "provider_ready": ready,
        "ability_uras": ability_uras,
        "remote_results": remote_results,
    },
    "assertions": {
        "hub_saw_provider_device": contains("hub-device-list.json", provider_node),
        "hub_saw_caller_device": contains("hub-device-list.json", caller_node),
        "agent_created_and_listed_on_caller": cli_contains("agent-list", agent_name),
        "agent_updated_on_caller": cli_contains("agent-set", "updated") or cli_contains("agent-set", agent_name),
        "agent_invoked_on_caller": contains("agent-send.txt", f"docker-easyremote-agent[{agent_name}]"),
        "temp_agent_removed_on_caller": (
            cli_contains("agent-temp-add", tmp_agent)
            and cli_contains("agent-temp-remove", tmp_agent)
            and not cli_contains("agent-list-after-temp-remove", tmp_agent)
        ),
        "provider_user_agent_created_and_listed": cli_contains("provider-agent-list-after-add", provider_agent_name),
        "provider_user_agent_invoked_via_agent_send": (
            contains("provider-agent-send.txt", f"docker-easyremote-agent[{provider_agent_name}]")
            and contains("provider-agent-send.txt", provider_agent_prompt)
        ),
        "caller_cli_discovered_user_agent_chat": (
            provider_agent_visible
            and isinstance(provider_agent_show, dict)
            and provider_agent_show.get("ability_ura") == provider_agent_chat_ura
            and provider_agent_name in str(provider_agent_show.get("owner_ura") or "")
        ),
        "user_agent_canonical_invocation_gap_detected": (
            provider_agent_canonical_status != "0"
            and (
                "ADMISSION_DESCRIPTOR_ROUTE_MISMATCH" in provider_agent_canonical_error
                or "NEGATIVE_REASON_NODATA" in provider_agent_canonical_error
            )
        ),
        "caller_observed_user_agent_chat_removed": (
            cli_contains("provider-agent-remove", provider_agent_name)
            and not cli_contains("provider-agent-list-after-remove", provider_agent_name)
            and provider_agent_removed
        ),
        "user_plugin_package_loaded_in_daemon": user_plugin_package_loaded,
        "user_plugin_ability_loaded_in_daemon": user_plugin_ability_loaded,
        "caller_cli_discovered_user_plugin_ability": (
            user_plugin_visible
            and isinstance(user_plugin_show, dict)
            and user_plugin_show.get("ability_ura") == user_plugin_ability_ura
            and user_plugin_show.get("owner_ura") == provider_ura
        ),
        "caller_cli_stream_called_user_plugin": (
            payload_contains(user_plugin_stream, "hello-plugin")
            and payload_contains(user_plugin_stream, user_plugin_message)
            and payload_has_field(user_plugin_stream, "invocation_nonce_len", 16)
        ),
        "user_plugin_one_invocation_record": (
            len(user_plugin_records) == 1
            and user_plugin_ability_ura in json.dumps(user_plugin_records[0])
        ),
        "user_plugin_receipt_chain_projected": user_plugin_receipt_chains_verified,
        "caller_observed_user_plugin_ability_removed": user_plugin_removed,
        "provider_registered_register_decorator_matrix": (
            ready.get("decorator") == "@node.register"
            and set(ready.get("function_shapes") or []) == {
                "sync",
                "variadic_args",
                "variadic_kwargs",
                "default_args",
                "async",
                "list_dict_payload",
                "generator_stream",
                "context",
            }
            and set((ready.get("abilities") or {}).keys()) == set(ability_uras.keys())
        ),
        "caller_cli_queried_all_provider_abilities": all_provider_abilities_visible,
        "caller_cli_queried_native_easynet_ability": native_visible,
        "caller_cli_showed_provider_add_ability": ability_uras["add"] in text("caller-ability-show-add.json"),
        "caller_cli_showed_native_easynet_ability": native_ability_ura in text("caller-ability-show-native.json"),
        "provider_cli_deployed_native_easynet_ability": (
            native_deploy.get("ability_ura") == native_ability_ura
            and native_deploy.get("namespace") == "nativeer"
            and native_deploy.get("public_name") == "native_echo"
            and native_deploy.get("state") == "ACTIVE"
        ),
        "caller_cli_stream_called_provider_add": payload_contains(cli_stream, '"value":42'),
        "caller_remote_used_module_remote_decorator": remote_add.get("value") == 5,
        "caller_remote_used_owner_remote_decorator": "RemoteOwner.remote" in remote_results.get("decorators", []),
        "caller_remote_variadic_args": remote_total_varargs == 10,
        "caller_remote_variadic_kwargs": remote_merge_kwargs == {"base": 10, "x": 1, "y": 2},
        "caller_remote_default_args": remote_defaulted == "hello device!",
        "caller_remote_async_function": remote_summarize == "one two three four",
        "caller_remote_list_dict_payload": remote_bundle == {
            "count": 3,
            "sum": 6,
            "meta": {"label": "caller", "ok": True},
        },
        "caller_remote_generator_stream": remote_countdown == [
            {"tick": 3},
            {"tick": 2},
            {"tick": 1},
        ],
        "caller_remote_context_function": (
            isinstance(remote_whoami, dict)
            and remote_whoami.get("note") == "from-caller-device"
            and remote_whoami.get("caller") == caller_ura
            and bool(remote_whoami.get("invocation_id"))
        ),
        "easyremote_sdk_consumer_boundary": (
            easyremote_boundary.get("ok") is True
            and easyremote_boundary.get("violations") == []
        ),
        "caller_cli_provider_catalog_discovered_native_easynet_ability": (
            isinstance(provider_catalog, list)
            and len(provider_catalog) > 0
            and len(native_discovery) == 1
            and native_discovery[0].get("ability_ura") == native_ability_ura
            and native_discovery[0].get("owner_ura") == provider_ura
        ),
        "caller_remote_called_easynet_native_handle": (
            native_handle.get("source") == "easynet-cli-deploy"
            and native_handle.get("joined") == "from-handlefrom-handle"
            and native_handle.get("caller") == caller_ura
            and bool(native_handle.get("invocation_id"))
        ),
        "caller_remote_streamed_easynet_native_handle": (
            native_handle_stream == [
                {
                    "source": "easynet-cli-deploy",
                    "text": "from-stream",
                    "times": 2,
                    "joined": "from-streamfrom-stream",
                    "caller": caller_ura,
                    "invocation_id": native_handle_stream[0].get("invocation_id")
                    if native_handle_stream and isinstance(native_handle_stream[0], dict)
                    else "",
                }
            ]
            and bool(native_handle_stream[0].get("invocation_id"))
        ),
        "caller_remote_called_easynet_native_canonical_ura": (
            native_ura_call.get("source") == "easynet-cli-deploy"
            and native_ura_call.get("joined") == "from-ura"
            and native_ura_call.get("caller") == caller_ura
            and bool(native_ura_call.get("invocation_id"))
        ),
        "caller_remote_streamed_easynet_native_canonical_ura": (
            native_ura_stream == [
                {
                    "source": "easynet-cli-deploy",
                    "text": "from-ura-stream",
                    "times": 1,
                    "joined": "from-ura-stream",
                    "caller": caller_ura,
                    "invocation_id": native_ura_stream[0].get("invocation_id")
                    if native_ura_stream and isinstance(native_ura_stream[0], dict)
                    else "",
                }
            ]
            and bool(native_ura_stream[0].get("invocation_id"))
        ),
        "caller_remote_called_easynet_native_typed_stub": (
            native_stub.get("source") == "easynet-cli-deploy"
            and native_stub.get("joined") == "from-stubfrom-stubfrom-stub"
            and native_stub.get("caller") == caller_ura
            and bool(native_stub.get("invocation_id"))
        ),
        "caller_remote_streamed_easynet_native_typed_stub": (
            native_stub_stream == [
                {
                    "source": "easynet-cli-deploy",
                    "text": "from-stub-stream",
                    "times": 1,
                    "joined": "from-stub-stream",
                    "caller": caller_ura,
                    "invocation_id": native_stub_stream[0].get("invocation_id")
                    if native_stub_stream and isinstance(native_stub_stream[0], dict)
                    else "",
                }
            ]
            and bool(native_stub_stream[0].get("invocation_id"))
        ),
        "one_cli_add_invocation_record": (
            len(cli_add_records) == 1
            and ability_uras["add"] in json.dumps(cli_add_records[0])
        ),
        "cli_add_receipt_chain_projected": receipt_chains_verified,
        "six_native_easynet_invocation_records": (
            len(native_records) == 6
            and all(native_ability_ura in json.dumps(record) for record in native_records)
        ),
        "native_easynet_one_daemon_record_per_easyremote_dispatch": (
            native_dispatch_ids_match_ledger
        ),
        "native_easynet_receipt_chains_projected": native_receipt_chains_verified,
        "caller_observed_provider_ability_removed": add_removed,
        "provider_other_abilities_remained_after_delete": other_abilities_remain,
        "caller_observed_native_easynet_ability_removed": native_removed,
    },
    "provider_user_agent_invocation_records": provider_agent_records,
    "provider_user_plugin_invocation_records": user_plugin_records,
    "caller_cli_add_stream_frames": cli_stream,
    "provider_cli_add_invocation_records": cli_add_records,
    "provider_native_easynet_invocation_records": native_records,
}
print(json.dumps(report, indent=2, sort_keys=True))
PY

jq -e '
  .assertions.hub_saw_provider_device
  and .assertions.hub_saw_caller_device
  and .assertions.agent_created_and_listed_on_caller
  and .assertions.agent_updated_on_caller
  and .assertions.agent_invoked_on_caller
  and .assertions.temp_agent_removed_on_caller
  and .assertions.provider_user_agent_created_and_listed
  and .assertions.provider_user_agent_invoked_via_agent_send
  and .assertions.caller_cli_discovered_user_agent_chat
  and .assertions.user_agent_canonical_invocation_gap_detected
  and .assertions.caller_observed_user_agent_chat_removed
  and .assertions.user_plugin_package_loaded_in_daemon
  and .assertions.user_plugin_ability_loaded_in_daemon
  and .assertions.caller_cli_discovered_user_plugin_ability
  and .assertions.caller_cli_stream_called_user_plugin
  and .assertions.user_plugin_one_invocation_record
  and .assertions.user_plugin_receipt_chain_projected
  and .assertions.caller_observed_user_plugin_ability_removed
  and .assertions.provider_registered_register_decorator_matrix
  and .assertions.caller_cli_queried_all_provider_abilities
  and .assertions.caller_cli_queried_native_easynet_ability
  and .assertions.caller_cli_showed_provider_add_ability
  and .assertions.caller_cli_showed_native_easynet_ability
  and .assertions.provider_cli_deployed_native_easynet_ability
  and .assertions.caller_cli_stream_called_provider_add
  and .assertions.caller_remote_used_module_remote_decorator
  and .assertions.caller_remote_used_owner_remote_decorator
  and .assertions.caller_remote_variadic_args
  and .assertions.caller_remote_variadic_kwargs
  and .assertions.caller_remote_default_args
  and .assertions.caller_remote_async_function
  and .assertions.caller_remote_list_dict_payload
  and .assertions.caller_remote_generator_stream
  and .assertions.caller_remote_context_function
  and .assertions.easyremote_sdk_consumer_boundary
  and .assertions.caller_cli_provider_catalog_discovered_native_easynet_ability
  and .assertions.caller_remote_called_easynet_native_handle
  and .assertions.caller_remote_streamed_easynet_native_handle
  and .assertions.caller_remote_called_easynet_native_canonical_ura
  and .assertions.caller_remote_streamed_easynet_native_canonical_ura
  and .assertions.caller_remote_called_easynet_native_typed_stub
  and .assertions.caller_remote_streamed_easynet_native_typed_stub
  and .assertions.one_cli_add_invocation_record
  and .assertions.cli_add_receipt_chain_projected
  and .assertions.six_native_easynet_invocation_records
  and .assertions.native_easynet_one_daemon_record_per_easyremote_dispatch
  and .assertions.native_easynet_receipt_chains_projected
  and .assertions.caller_observed_provider_ability_removed
  and .assertions.provider_other_abilities_remained_after_delete
  and .assertions.caller_observed_native_easynet_ability_removed
' "$OUT_DIR/report.json" >/dev/null

python3 - "$OUT_DIR/report.json" >"$OUT_DIR/report.md" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
print("# Docker EasyRemote CLI Cross-Device E2E")
print("")
print(f"- Hub URA: `{report['topology']['hub_ura']}`")
print(f"- Provider device: `{report['topology']['provider_ura']}`")
print(f"- Caller device: `{report['topology']['caller_ura']}`")
print("- EasyRemote abilities:")
for name, ura in sorted(report["easyremote"]["ability_uras"].items()):
    print(f"  - `{name}`: `{ura}`")
print("- Native EasyNet ability:")
print(f"  - `nativeer.native_echo`: `{report['native_easynet']['ability_ura']}`")
print("- User-defined Agent chat ability:")
print(f"  - `{report['user_agent']['chat_ability']}`: `{report['user_agent']['chat_ability_ura']}`")
print("- User-defined plugin ability:")
print(f"  - `{report['user_plugin']['ability']}`: `{report['user_plugin']['ability_ura']}`")
print("")
print("## Assertions")
for key, value in report["assertions"].items():
    print(f"- `{key}`: `{str(value).lower()}`")
PY

echo "PASS: $OUT_DIR/report.md"

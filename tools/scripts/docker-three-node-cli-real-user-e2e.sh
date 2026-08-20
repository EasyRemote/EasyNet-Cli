#!/usr/bin/env bash
# Start a real Docker e2e topology (one Hub + two devices), attach both
# devices to one backend user using CLI auth/pair/join commands, add real
# daemon-owned agent/ability/skill state, and query the frontend-equivalent
# surfaces with CLI commands.
#
# Business operations intentionally go through `easynet`:
#   - easynet auth login --register-if-missing
#   - easynet auth pair
#   - easynet device join
#   - easynet agent add
#   - easynet ability invoke ability.publish / skill.publish
#   - easynet auth devices / abilities / agents
#
# Docker compose, health waiting, and report writing are harness concerns.

set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
cli_root="$(cd "$script_dir/../.." && pwd)"
easynet_root="${EASYNET_BACKEND_ROOT:-$(cd "$cli_root/../EasyNet" 2>/dev/null && pwd)}"
compose_file="$easynet_root/docker/e2e/docker-compose.yml"

project="${EASYNET_E2E_PROJECT:-easynet-cli-real-user}"
hub_realm="${HUB_REALM:-easynet.run}"
hub_http_port="${HUB_HTTP_PORT:-18080}"
hub_tls_port="${HUB_TLS_PORT:-50443}"
hub_http_url="http://127.0.0.1:${hub_http_port}"
hub_public_endpoint="${HUB_PUBLIC_ENDPOINT:-https://hub:50443}"
hub_image="${EASYNET_HUB_IMAGE:-easynet/hub-e2e:local}"
device_image="${EASYNET_DEVICE_IMAGE:-easynet/device-e2e:local}"

linux_home="/home/easynet"
macos_home="/Users/easynet"
timestamp="$(date +%Y%m%d-%H%M%S)"
email="${EASYNET_E2E_EMAIL:-cli-real-user-${timestamp}@example.test}"
password="${EASYNET_E2E_PASSWORD:-Strong-Password-123}"
nickname="${EASYNET_E2E_NICKNAME:-cli-real-user}"

requests="${EASYNET_E2E_REQUESTS:-24}"
concurrency="${EASYNET_E2E_CONCURRENCY:-6}"
skip_build=0
keep=0
reset_stack=1
self_test=0
online_projection_status="not_checked"
strict_frontend_projection=0

usage() {
    cat <<'USAGE'
Usage:
  tools/scripts/docker-three-node-cli-real-user-e2e.sh [options]

Options:
  --skip-build             Reuse existing easynet/hub-e2e:local and easynet/device-e2e:local images.
  --keep                   Leave containers/volumes running after completion.
  --no-reset               Do not compose down before starting.
  --requests N             Requests per load-test target (default: 24).
  --concurrency N          Concurrent requests per load-test batch (default: 6).
  --strict-frontend-projection
                           Fail when Hub/frontend auth projection does not show the
                           local custom agents, abilities, skills, and ONLINE devices.
  --self-test              Validate script prerequisites that do not require Docker engine.
  -h, --help               Show this help.

Environment:
  EASYNET_BACKEND_ROOT     Path to sibling EasyNet repo. Default: ../EasyNet.
  EASYNET_E2E_EMAIL        User email. Default: generated cli-real-user-<timestamp>@example.test.
  EASYNET_E2E_PASSWORD     User password. Default: Strong-Password-123.
  EASYNET_E2E_PROJECT      Docker Compose project name. Default: easynet-cli-real-user.
  HUB_HTTP_PORT            Host HTTP port. Default: 18080.
  HUB_TLS_PORT             Host daemon TLS port. Default: 50443.
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-build)
            skip_build=1
            shift
            ;;
        --keep)
            keep=1
            shift
            ;;
        --no-reset)
            reset_stack=0
            shift
            ;;
        --requests)
            requests="${2:?--requests requires a value}"
            shift 2
            ;;
        --concurrency)
            concurrency="${2:?--concurrency requires a value}"
            shift 2
            ;;
        --strict-frontend-projection)
            strict_frontend_projection=1
            shift
            ;;
        --self-test)
            self_test=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 64
            ;;
    esac
done

report_root="$cli_root/target/e2e/docker-three-node-cli-real-user/$timestamp"
mkdir -p "$report_root"

compose() {
    docker compose -p "$project" -f "$compose_file" "$@"
}

service_exec() {
    local service="$1"
    shift
    compose exec -T "$service" sh -lc "$*"
}

linux_cli() {
    service_exec dev-linux "HOME='$linux_home' EASYNET_REALM_TRUST_PATH='$linux_home/realm-trust.toml' easynet $*"
}

macos_cli() {
    service_exec dev-macos-sim "HOME='$macos_home' EASYNET_REALM_TRUST_PATH='$macos_home/realm-trust.toml' easynet $*"
}

die() {
    echo "[FAIL] $*" >&2
    exit 1
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "missing command: $1"
}

require_prereqs() {
    [[ -f "$compose_file" ]] || die "compose file not found: $compose_file"
    [[ -x "$easynet_root/scripts/docker-build-images.sh" ]] || die "build script not found: $easynet_root/scripts/docker-build-images.sh"
    need_cmd docker
    need_cmd curl
    need_cmd jq
    need_cmd python3
}

require_docker_engine() {
    docker info >/dev/null 2>&1 || die "Docker engine is not running or the Docker socket is unavailable"
}

dump_logs() {
    echo "==> docker compose logs" >&2
    compose logs --no-color hub dev-linux dev-macos-sim postgres >&2 || true
    echo "==> hub daemon log" >&2
    service_exec hub "tail -160 /tmp/easynet-hub-daemon.log" >&2 || true
    echo "==> dev-linux daemon log" >&2
    service_exec dev-linux "tail -160 /tmp/easynet-daemon.log" >&2 || true
    echo "==> dev-macos-sim daemon log" >&2
    service_exec dev-macos-sim "tail -160 /tmp/easynet-daemon.log" >&2 || true
}

cleanup() {
    local status="$1"
    if [[ "$status" -ne 0 ]]; then
        dump_logs
    fi
    if [[ "$keep" != "1" ]]; then
        compose down -v --remove-orphans >/dev/null 2>&1 || true
    fi
}

wait_for_hub() {
    echo "==> waiting for Hub HTTP health at $hub_http_url"
    for _ in $(seq 1 90); do
        if curl -fsS "$hub_http_url/api/v1/health" >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    curl -fsS "$hub_http_url/api/v1/health" >/dev/null
}

wait_for_daemon() {
    local service="$1"
    local home="$2"
    echo "==> waiting for $service daemon"
    for _ in $(seq 1 90); do
        if service_exec "$service" "test -S '$home/.easynet/daemon.sock' && pgrep -f easynet-daemon >/dev/null" >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    service_exec "$service" "ls -la '$home/.easynet'; tail -120 /tmp/easynet-daemon.log" >&2 || true
    return 1
}

wait_for_online_devices() {
    local node_a="$1"
    local node_b="$2"
    echo "==> waiting for both devices to project ONLINE through CLI auth devices"
    for _ in $(seq 1 15); do
        if linux_cli "auth devices --json" >"$report_root/auth-devices-latest.json" 2>"$report_root/auth-devices-latest.err"; then
            if jq -e --arg a "$node_a" --arg b "$node_b" '
                def online($id):
                  [.items[]? | select(.node_id == $id and ((.state // "") | ascii_upcase) == "ONLINE")] | length > 0;
                online($a) and online($b)
            ' "$report_root/auth-devices-latest.json" >/dev/null; then
                online_projection_status="online"
                return 0
            fi
        fi
        sleep 1
    done
    online_projection_status="not_online"
    echo "WARN: backend auth devices did not project both devices ONLINE; continuing with daemon-bound CLI checks" >&2
    cat "$report_root/auth-devices-latest.json" >&2 || true
    cat "$report_root/auth-devices-latest.err" >&2 || true
    return 0
}

ability_publish_args() {
    local owner_ura="$1"
    local ability_name="$2"
    python3 - "$owner_ura" "$ability_name" <<'PY'
import json
import sys

owner_ura = sys.argv[1]
ability_name = sys.argv[2]
manifest = f'''schema_version = "1"
name = "{ability_name}"
description = "CLI e2e custom agent ability"
[input_schema]
type = "object"
'''
print(json.dumps({
    "owner_ura": owner_ura,
    "manifest_toml": manifest,
}, separators=(",", ":")))
PY
}

skill_publish_args() {
    local owner_agent_id="$1"
    local skill_name="$2"
    python3 - "$owner_agent_id" "$skill_name" <<'PY'
import json
import sys

owner = sys.argv[1]
skill = sys.argv[2]
body = f"""# {skill}

Use this skill when the CLI e2e harness needs a real custom skill owned by {owner}.
This file is intentionally small so it exercises daemon materialisation without adding
network or marketplace dependencies.
"""
print(json.dumps({
    "owner_agent_id": owner,
    "skill_name": skill,
    "skill_md": body,
    "mission_run_id": "docker-three-node-cli-real-user-e2e",
}, separators=(",", ":")))
PY
}

write_count() {
    local file="$1"
    jq '[.items[]?] | length' "$file"
}

json_contains() {
    local file="$1"
    local needle="$2"
    python3 - "$file" "$needle" <<'PY'
import json
import sys

path, needle = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as fh:
    payload = json.load(fh)

def walk(value):
    if isinstance(value, dict):
        for key, child in value.items():
            if needle in str(key):
                return True
            if walk(child):
                return True
        return False
    if isinstance(value, list):
        return any(walk(child) for child in value)
    return needle in str(value)

sys.exit(0 if walk(payload) else 1)
PY
}

text_contains() {
    local file="$1"
    local needle="$2"
    grep -F "$needle" "$file" >/dev/null 2>&1
}

bool_json() {
    if "$@"; then
        printf 'true'
    else
        printf 'false'
    fi
}

ms_now() {
    python3 -c 'import time; print(int(time.time() * 1000))'
}

summarize_durations() {
    local file="$1"
    python3 - "$file" <<'PY'
import json
import sys

path = sys.argv[1]
durations = []
ok = 0
fail = 0
with open(path, "r", encoding="utf-8") as fh:
    for line in fh:
        if not line.strip():
            continue
        row = json.loads(line)
        durations.append(float(row["ms"]))
        if row["ok"]:
            ok += 1
        else:
            fail += 1

durations.sort()
def pct(p):
    if not durations:
        return 0.0
    idx = int(round((len(durations) - 1) * p))
    return durations[max(0, min(idx, len(durations) - 1))]

avg = sum(durations) / len(durations) if durations else 0.0
print(json.dumps({
    "ok": ok,
    "fail": fail,
    "count": len(durations),
    "avg_ms": round(avg, 2),
    "p50_ms": round(pct(0.50), 2),
    "p95_ms": round(pct(0.95), 2),
    "p99_ms": round(pct(0.99), 2),
    "max_ms": round(max(durations) if durations else 0.0, 2),
}, separators=(",", ":")))
PY
}

run_load() {
    local label="$1"
    local command="$2"
    local dir="$report_root/load-$label"
    local durations="$dir/durations.jsonl"
    mkdir -p "$dir"
    : >"$durations"

    echo "==> load: $label requests=$requests concurrency=$concurrency"
    local launched=0
    while [[ "$launched" -lt "$requests" ]]; do
        local batch=0
        local pids=()
        while [[ "$batch" -lt "$concurrency" && "$launched" -lt "$requests" ]]; do
            launched=$((launched + 1))
            batch=$((batch + 1))
            (
                local start end ok out err
                out="$dir/$launched.out"
                err="$dir/$launched.err"
                start="$(ms_now)"
                if eval "$command" >"$out" 2>"$err"; then
                    ok=true
                else
                    ok=false
                fi
                end="$(ms_now)"
                printf '{"request":%s,"ok":%s,"ms":%s}\n' "$launched" "$ok" "$((end - start))"
            ) >>"$durations" &
            pids+=("$!")
        done
        local pid
        for pid in "${pids[@]}"; do
            wait "$pid" || true
        done
    done

    summarize_durations "$durations" >"$dir/summary.json"
}

if [[ "$self_test" == "1" ]]; then
    require_prereqs
    bash -n "$0"
    echo "self-test ok"
    exit 0
fi

require_prereqs
require_docker_engine
trap 'cleanup $?' EXIT

if [[ "$skip_build" != "1" ]]; then
    echo "==> building Docker e2e images"
    EASYNET_HUB_IMAGE="$hub_image" EASYNET_DEVICE_IMAGE="$device_image" \
        "$easynet_root/scripts/docker-build-images.sh"
fi

export EASYNET_HUB_IMAGE="$hub_image"
export EASYNET_DEVICE_IMAGE="$device_image"
export HUB_HTTP_PORT="$hub_http_port"
export HUB_TLS_PORT="$hub_tls_port"
export HUB_REALM="$hub_realm"
export HUB_PUBLIC_ENDPOINT="$hub_public_endpoint"
export EASYNET_DEVICE_AUTOSTART_MODE="${EASYNET_DEVICE_AUTOSTART_MODE:-daemon}"

if [[ "$reset_stack" == "1" ]]; then
    compose down -v --remove-orphans >/dev/null 2>&1 || true
fi

echo "==> starting Docker topology project=$project"
compose up -d postgres hub dev-linux dev-macos-sim
wait_for_hub

echo "==> CLI login/register as $email"
linux_cli "auth login '$email' --password '$password' --hub 'http://hub:8080' --register-if-missing --nickname '$nickname'" \
    >"$report_root/auth-login.txt"
linux_cli "auth whoami" >"$report_root/auth-whoami.txt"

echo "==> minting pairing tokens through CLI"
token_linux="$(linux_cli "auth pair --quiet" | tr -d '\r\n')"
token_macos="$(linux_cli "auth pair --quiet" | tr -d '\r\n')"
[[ -n "$token_linux" && -n "$token_macos" ]] || die "empty pairing token"

echo "==> joining dev-linux through CLI"
linux_cli "device join '$token_linux' --hub 'http://hub:8080' --boot no --yes" >"$report_root/join-linux.txt"
echo "==> joining dev-macos-sim through CLI"
macos_cli "device join '$token_macos' --hub 'http://hub:8080' --boot no --yes" >"$report_root/join-macos.txt"

node_linux="$(service_exec dev-linux "jq -r .node_id '$linux_home/.easynet/credentials.json'")"
node_macos="$(service_exec dev-macos-sim "jq -r .node_id '$macos_home/.easynet/credentials.json'")"
[[ -n "$node_linux" && "$node_linux" != "null" ]] || die "missing linux node_id"
[[ -n "$node_macos" && "$node_macos" != "null" ]] || die "missing macos node_id"

wait_for_daemon dev-linux "$linux_home"
wait_for_daemon dev-macos-sim "$macos_home"
wait_for_online_devices "$node_linux" "$node_macos"

username="$(service_exec dev-linux "jq -r '.username // (.email | split(\"@\")[0])' '$linux_home/.easynet/auth.json'")"
agent_linux="cli-linux-${timestamp}"
agent_macos="cli-macos-${timestamp}"
ability_linux="custom-linux-${timestamp}"
ability_macos="custom-macos-${timestamp}"
skill_linux="custom-linux-skill-${timestamp}"
skill_macos="custom-macos-skill-${timestamp}"
owner_linux="easynet:///r/${hub_realm}/agent/${username}.${agent_linux}"
owner_macos="easynet:///r/${hub_realm}/agent/${username}.${agent_macos}"
ability_publish_linux_ura="easynet:///r/${hub_realm}/ability/device.${node_linux}.ability.publish"
ability_publish_macos_ura="easynet:///r/${hub_realm}/ability/device.${node_macos}.ability.publish"
skill_publish_linux_ura="easynet:///r/${hub_realm}/ability/device.${node_linux}.skill.publish"
skill_publish_macos_ura="easynet:///r/${hub_realm}/ability/device.${node_macos}.skill.publish"

echo "==> adding custom agents"
linux_cli "agent add '$agent_linux' --type claude-code --model e2e-stub --label '$agent_linux'" \
    >"$report_root/agent-add-linux.txt"
macos_cli "agent add '$agent_macos' --type claude-code --model e2e-stub --label '$agent_macos'" \
    >"$report_root/agent-add-macos.txt"

echo "==> publishing custom agent abilities through CLI ability invoke"
linux_ability_args="$(ability_publish_args "$owner_linux" "$ability_linux")"
macos_ability_args="$(ability_publish_args "$owner_macos" "$ability_macos")"
linux_cli "ability invoke '$ability_publish_linux_ura' --args '$linux_ability_args' --raw" \
    >"$report_root/ability-publish-linux.json"
macos_cli "ability invoke '$ability_publish_macos_ura' --args '$macos_ability_args' --raw" \
    >"$report_root/ability-publish-macos.json"

echo "==> publishing custom skills through CLI ability invoke"
linux_skill_args="$(skill_publish_args "$agent_linux" "$skill_linux")"
macos_skill_args="$(skill_publish_args "$agent_macos" "$skill_macos")"
linux_cli "ability invoke '$skill_publish_linux_ura' --args '$linux_skill_args' --raw" \
    >"$report_root/skill-publish-linux.json"
macos_cli "ability invoke '$skill_publish_macos_ura' --args '$macos_skill_args' --raw" \
    >"$report_root/skill-publish-macos.json"

echo "==> refreshing agents"
linux_cli "agent refresh --agent '$agent_linux'" >"$report_root/agent-refresh-linux.txt" || true
macos_cli "agent refresh --agent '$agent_macos'" >"$report_root/agent-refresh-macos.txt" || true

echo "==> collecting local daemon-bound views"
linux_cli "agent list" >"$report_root/local-agent-list-linux.txt" 2>&1
macos_cli "agent list" >"$report_root/local-agent-list-macos.txt" 2>&1
linux_cli "agent abilities '$agent_linux'" >"$report_root/local-agent-abilities-linux.txt" 2>&1
macos_cli "agent abilities '$agent_macos'" >"$report_root/local-agent-abilities-macos.txt" 2>&1
linux_cli "skill list --agent '$agent_linux' --json" >"$report_root/local-skill-list-linux.json"
macos_cli "skill list --agent '$agent_macos' --json" >"$report_root/local-skill-list-macos.json"

echo "==> collecting frontend-equivalent user views through CLI auth"
linux_cli "auth devices --json" >"$report_root/auth-devices.json"
linux_cli "auth abilities '$node_linux' --json" >"$report_root/auth-abilities-linux.json"
linux_cli "auth abilities '$node_macos' --json" >"$report_root/auth-abilities-macos.json"
linux_cli "auth agents --json" >"$report_root/auth-agents.json"

linux_ability_count="$(write_count "$report_root/auth-abilities-linux.json")"
macos_ability_count="$(write_count "$report_root/auth-abilities-macos.json")"
agent_count="$(write_count "$report_root/auth-agents.json")"
device_count="$(write_count "$report_root/auth-devices.json")"

local_linux_agent_visible="$(bool_json text_contains "$report_root/local-agent-list-linux.txt" "$agent_linux")"
local_macos_agent_visible="$(bool_json text_contains "$report_root/local-agent-list-macos.txt" "$agent_macos")"
local_linux_ability_visible="$(bool_json text_contains "$report_root/local-agent-abilities-linux.txt" "$ability_linux")"
local_macos_ability_visible="$(bool_json text_contains "$report_root/local-agent-abilities-macos.txt" "$ability_macos")"
local_linux_skill_visible="$(bool_json json_contains "$report_root/local-skill-list-linux.json" "$skill_linux")"
local_macos_skill_visible="$(bool_json json_contains "$report_root/local-skill-list-macos.json" "$skill_macos")"
frontend_linux_agent_visible="$(bool_json json_contains "$report_root/auth-agents.json" "$agent_linux")"
frontend_macos_agent_visible="$(bool_json json_contains "$report_root/auth-agents.json" "$agent_macos")"
frontend_linux_ability_visible="$(bool_json json_contains "$report_root/auth-abilities-linux.json" "$ability_linux")"
frontend_macos_ability_visible="$(bool_json json_contains "$report_root/auth-abilities-macos.json" "$ability_macos")"
frontend_online_projection=false
if [[ "$online_projection_status" == "online" ]]; then
    frontend_online_projection=true
fi

cat >"$report_root/projection-assertions.json" <<JSON
{
  "local_daemon": {
    "linux_agent_visible": $local_linux_agent_visible,
    "macos_agent_visible": $local_macos_agent_visible,
    "linux_custom_ability_visible": $local_linux_ability_visible,
    "macos_custom_ability_visible": $local_macos_ability_visible,
    "linux_custom_skill_visible": $local_linux_skill_visible,
    "macos_custom_skill_visible": $local_macos_skill_visible
  },
  "frontend_auth_projection": {
    "online_devices_visible": $frontend_online_projection,
    "linux_agent_visible": $frontend_linux_agent_visible,
    "macos_agent_visible": $frontend_macos_agent_visible,
    "linux_custom_ability_visible": $frontend_linux_ability_visible,
    "macos_custom_ability_visible": $frontend_macos_ability_visible
  }
}
JSON

jq -e '
  .local_daemon.linux_agent_visible
  and .local_daemon.macos_agent_visible
  and .local_daemon.linux_custom_ability_visible
  and .local_daemon.macos_custom_ability_visible
  and .local_daemon.linux_custom_skill_visible
  and .local_daemon.macos_custom_skill_visible
' "$report_root/projection-assertions.json" >/dev/null || die "local daemon custom agent/ability/skill visibility assertion failed"

run_load "auth-devices" "linux_cli \"auth devices --json\""
run_load "auth-abilities-linux" "linux_cli \"auth abilities '$node_linux' --json\""
run_load "auth-abilities-macos" "linux_cli \"auth abilities '$node_macos' --json\""
run_load "auth-agents" "linux_cli \"auth agents --json\""

cat >"$report_root/report.json" <<JSON
{
  "project": "$project",
  "hub_http_url": "$hub_http_url",
  "hub_realm": "$hub_realm",
  "email": "$email",
  "username": "$username",
  "nodes": {
    "dev_linux": "$node_linux",
    "dev_macos_sim": "$node_macos"
  },
  "custom": {
    "dev_linux": {
      "agent": "$agent_linux",
      "agent_ura": "$owner_linux",
      "ability": "$ability_linux",
      "skill": "$skill_linux"
    },
    "dev_macos_sim": {
      "agent": "$agent_macos",
      "agent_ura": "$owner_macos",
      "ability": "$ability_macos",
      "skill": "$skill_macos"
    }
  },
  "baseline_counts": {
    "auth_devices": $device_count,
    "auth_agents": $agent_count,
    "auth_abilities_linux": $linux_ability_count,
    "auth_abilities_macos": $macos_ability_count
  },
  "device_online_projection": "$online_projection_status",
  "projection_assertions": $(cat "$report_root/projection-assertions.json"),
  "load": {
    "auth_devices": $(cat "$report_root/load-auth-devices/summary.json"),
    "auth_abilities_linux": $(cat "$report_root/load-auth-abilities-linux/summary.json"),
    "auth_abilities_macos": $(cat "$report_root/load-auth-abilities-macos/summary.json"),
    "auth_agents": $(cat "$report_root/load-auth-agents/summary.json")
  }
}
JSON

cat >"$report_root/report.md" <<MD
# Docker Three-Node CLI Real User E2E

- project: \`$project\`
- hub: \`$hub_http_url\`
- user: \`$email\` / \`$username\`
- devices: \`$node_linux\`, \`$node_macos\`
- backend online projection: \`$online_projection_status\`
- custom linux agent/ability/skill: \`$agent_linux\`, \`$ability_linux\`, \`$skill_linux\`
- custom macos agent/ability/skill: \`$agent_macos\`, \`$ability_macos\`, \`$skill_macos\`

## Frontend-equivalent CLI counts

- \`easynet auth devices --json\`: $device_count
- \`easynet auth agents --json\`: $agent_count
- \`easynet auth abilities $node_linux --json\`: $linux_ability_count
- \`easynet auth abilities $node_macos --json\`: $macos_ability_count

## Projection assertions

\`\`\`json
$(jq '.' "$report_root/projection-assertions.json")
\`\`\`

## Load summary

\`\`\`json
$(jq '.load' "$report_root/report.json")
\`\`\`

## Artifacts

- auth devices: \`$report_root/auth-devices.json\`
- auth agents: \`$report_root/auth-agents.json\`
- auth abilities linux: \`$report_root/auth-abilities-linux.json\`
- auth abilities macos: \`$report_root/auth-abilities-macos.json\`
- projection assertions: \`$report_root/projection-assertions.json\`
- local agent abilities linux: \`$report_root/local-agent-abilities-linux.txt\`
- local agent abilities macos: \`$report_root/local-agent-abilities-macos.txt\`
- local skill list linux: \`$report_root/local-skill-list-linux.json\`
- local skill list macos: \`$report_root/local-skill-list-macos.json\`
MD

echo "==> report: $report_root/report.md"
cat "$report_root/report.md"

if [[ "$strict_frontend_projection" == "1" ]]; then
    if ! jq -e '
        .frontend_auth_projection.online_devices_visible
        and .frontend_auth_projection.linux_agent_visible
        and .frontend_auth_projection.macos_agent_visible
        and .frontend_auth_projection.linux_custom_ability_visible
        and .frontend_auth_projection.macos_custom_ability_visible
    ' "$report_root/projection-assertions.json" >/dev/null; then
        echo "[FAIL] strict frontend projection assertions failed" >&2
        jq '.' "$report_root/projection-assertions.json" >&2
        exit 1
    fi
fi

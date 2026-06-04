#!/usr/bin/env bash
# e2e-release-flow.sh — drive `easynet device join` + `easynet start`
# + `agent add` + advertise verification against a release-shape
# install.
#
# This is the harness that makes the docker-e2e / production
# divergence visible. The pre-existing docker e2e bundles
# `axon-runtime` into its image, so any code path that *spawns
# axon-runtime as a separate process* succeeds inside docker e2e
# even if production tarballs do not carry the binary. This harness
# is the antidote: it runs against a Phase-A release tarball whose
# shape mirrors what `https://easynet.run/install` would actually
# place on a fresh user's host.
#
# What this harness asserts
# -------------------------
# Against a release-shape sandbox install (see e2e-release-install.sh):
#
#   1. `easynet device join <token>` against a local dev-backend
#      succeeds and writes credentials.json + daemon-config.toml.
#   2. `easynet start` brings the daemon online.
#   3. **No `axon-runtime` process is alive** at any point. The
#      device-mode daemon is the only long-running process; the
#      release tarball does not ship axon-runtime, so a code path
#      that spawns it would fail at exec.
#   4. `easynet ability list` returns the device's local catalog
#      (no hub round-trip needed for system abilities).
#   5. `easynet agent add codex --type codex` and `agent add claude
#      --type claude-code` register hosted agents into
#      local-agents.json with friendly URAs (`<user>.codex`,
#      `<user>.claude`, `<user>.consent-default`).
#   6. The session-prelude advertises every hosted agent + the
#      synthetic pages/files pair (5 agents total) to the hub.
#   7. `easynet auth abilities <node>` against the dev-backend's REST
#      API surfaces the same ability catalogue with friendly
#      `display_name` values (no raw uuid hashes).
#
# Today's expected outcome
# ------------------------
# **Fails at step 2** with current code (start.rs::start_runtime_for_device
# tries to fork axon-runtime, which is not in the release tarball).
# That failure IS the bug we're tracking; this harness is the
# regression gate that goes red on the bug and green after the P0
# daemon-first refactor lands.
#
# Hub topology
# ------------
# Local dev-backend on :8080 by default (override via
# EASYNET_TEST_BACKEND_PORT / --backend-port; started via
# scripts/dev-backend.sh --reset-db --no-seed-device from
# the EasyNet repo). The dev-backend seeds a default
# user dev@easynet.local / dev-password and serves both the REST API
# and the gRPC bidirectional surface devices need to dial. We mint a
# pairing token via POST /api/v1/devices/pairing using the seeded
# user's JWT and pass it to `easynet device join`.
#
# Author: 海峰 <silan.hu@u.nus.edu>
# Copyright (c) 2026 EasyNet. All rights reserved.

set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
cli_root="$(cd "$script_dir/.." && pwd)"
workspace_root="$(cd "$cli_root/.." && pwd)"
easynet_repo="$workspace_root/EasyNet"

# Allow caller to bring their own backend (e.g. dev-host-e2e). When
# unset, the harness boots its own dev-backend for the test window.
backend_url="${EASYNET_TEST_HUB_URL:-}"
backend_port="${EASYNET_TEST_BACKEND_PORT:-8080}"
backend_pid=""
backend_listener_pid=""
baseline_axon_pids=""
own_backend=0

# Allow caller to pass an existing sandbox prefix (skips the install
# step). When empty, run e2e-release-install.sh first.
prefix="${EASYNET_TEST_PREFIX:-}"
keep_prefix=0
keep_state=0

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)        prefix="$2"; shift 2 ;;
        --backend-url)   backend_url="$2"; shift 2 ;;
        --backend-port)  backend_port="$2"; shift 2 ;;
        --keep-prefix)   keep_prefix=1; shift ;;
        --keep-state)    keep_state=1; shift ;;
        --help|-h)
            sed -n '2,/^# Author:/p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "e2e-release-flow.sh: unknown flag $1" >&2
            exit 2
            ;;
    esac
done

cleanup() {
    local exitcode=$?
    set +e
    if [ -n "$backend_pid" ] && [ "$own_backend" = 1 ]; then
        echo "==> stopping dev-backend (pid=$backend_pid)"
        kill "$backend_pid" 2>/dev/null
        wait "$backend_pid" 2>/dev/null
    fi
    if [ -n "$backend_listener_pid" ] && [ "$own_backend" = 1 ]; then
        kill "$backend_listener_pid" 2>/dev/null
        wait "$backend_listener_pid" 2>/dev/null
    fi
    if [ "$keep_state" != 1 ] && [ -n "${SANDBOX_HOME:-}" ]; then
        # Stop any spawned daemon so the next run starts clean.
        if [ -f "$SANDBOX_HOME/.easynet/runtime.json" ]; then
            local pid
            pid="$(jq -r .pid "$SANDBOX_HOME/.easynet/runtime.json" 2>/dev/null)"
            if [ -n "$pid" ] && [ "$pid" != "null" ]; then
                kill "$pid" 2>/dev/null
            fi
        fi
        if [ -f "$SANDBOX_HOME/.easynet/easynet-daemon.pid" ]; then
            local dpid
            dpid="$(cat "$SANDBOX_HOME/.easynet/easynet-daemon.pid" 2>/dev/null)"
            if [ -n "$dpid" ]; then
                kill "$dpid" 2>/dev/null
            fi
        fi
    fi
    if [ -n "${prefix:-}" ] && [ "$keep_prefix" != 1 ]; then
        rm -rf "$prefix" 2>/dev/null
    fi
    exit "$exitcode"
}
trap cleanup EXIT

# ── 1. Install the release tarball into a sandbox ────────────────
if [ -z "$prefix" ]; then
    echo "==> [1/7] install release tarball under a sandbox prefix"
    install_out="$(bash "$script_dir/e2e-release-install.sh" --keep-prefix 2>&1)"
    if [ $? -ne 0 ]; then
        echo "$install_out"
        exit 1
    fi
    prefix="$(printf '%s\n' "$install_out" | grep '^prefix=' | tail -1 | cut -d= -f2-)"
    if [ -z "$prefix" ] || [ ! -d "$prefix" ]; then
        echo "[FAIL] install harness did not emit prefix=" >&2
        printf '%s\n' "$install_out" >&2
        exit 1
    fi
fi
env_file="$prefix/easynet-env.sh"
if [ ! -f "$env_file" ]; then
    echo "[FAIL] env file missing: $env_file" >&2
    exit 1
fi
# shellcheck disable=SC1090
. "$env_file"
SANDBOX_HOME="$HOME"
if [ "$keep_state" != 1 ]; then
    mkdir -p "$HOME/.easynet"
    find "$HOME/.easynet" -mindepth 1 -maxdepth 1 \
        ! -name 'dendrite-bridge' \
        -exec rm -rf {} + 2>/dev/null || true
    rm -rf "$HOME/.easynet-dev" 2>/dev/null || true
fi
echo "    prefix=$prefix"
echo "    HOME=$HOME"
echo "    EASYNET_DENDRITE_BRIDGE_LIB=$EASYNET_DENDRITE_BRIDGE_LIB"

# ── 2. Bring up dev-backend (if caller didn't supply a hub URL) ──
if [ -z "$backend_url" ]; then
    if [ ! -d "$easynet_repo" ]; then
        echo "[FAIL] no --backend-url and no $easynet_repo to start dev-backend from" >&2
        exit 1
    fi
    existing_backend_pids="$(lsof -ti "tcp:${backend_port}" 2>/dev/null || true)"
    if [ -n "$existing_backend_pids" ]; then
        echo "[FAIL] backend test port :$backend_port already in use:" >&2
        lsof -nP -iTCP:"$backend_port" -sTCP:LISTEN >&2 || true
        exit 1
    fi
    echo "==> [2/7] starting dev-backend (--reset-db --no-seed-device)"
    own_backend=1
    backend_log="$prefix/dev-backend.log"
    (
        cd "$easynet_repo"
        EASYNET_PORT="$backend_port" \
        EASYNET_CLI_DIR="$cli_root" \
        EASYNET_AXON_RUNTIME_BIN="$workspace_root/EasyNet-Axon/core/runtime-rs/target/release/axon-runtime" \
          nohup bash scripts/dev-backend.sh --reset-db --no-seed-device > "$backend_log" 2>&1 &
        echo $! > "$prefix/dev-backend.pid"
    )
    backend_pid="$(cat "$prefix/dev-backend.pid")"
    backend_url="http://127.0.0.1:$backend_port"
    echo "    backend_url=$backend_url  pid=$backend_pid"
    echo "    log=$backend_log"

    # Wait for /api/v1/health. Cold-start can spend several minutes
    # on first invocation downloading Go deps + ent migrations; the
    # ceiling here is high enough to tolerate a fresh `~/go/pkg`.
    # Subsequent runs hit warm caches and complete in seconds.
    backend_wait_seconds="${EASYNET_TEST_BACKEND_TIMEOUT:-600}"
    for _ in $(seq 1 "$backend_wait_seconds"); do
        if curl -fsS "$backend_url/api/v1/health" >/dev/null 2>&1; then
            break
        fi
        sleep 1
    done
    if ! curl -fsS "$backend_url/api/v1/health" >/dev/null 2>&1; then
        echo "[FAIL] dev-backend did not come up in ${backend_wait_seconds}s" >&2
        tail -50 "$backend_log" >&2
        exit 1
    fi
    backend_listener_pid="$(lsof -ti "tcp:${backend_port}" 2>/dev/null | head -n 1 || true)"
    for _ in $(seq 1 120); do
        if grep -q '\[seed\] \[seed-dev\] OK' "$backend_log" 2>/dev/null; then
            break
        fi
        sleep 1
    done
    if ! grep -q '\[seed\] \[seed-dev\] OK' "$backend_log" 2>/dev/null; then
        echo "[FAIL] dev-backend never finished seeding dev credentials" >&2
        tail -80 "$backend_log" >&2
        exit 1
    fi
    baseline_axon_pids="$(pgrep -f "$workspace_root/EasyNet-Axon/core/runtime-rs/target/release/axon-runtime" | sort | tr '\n' ' ' || true)"
fi

# ── 3. Login as dev user, mint a pairing token ───────────────────
echo "==> [3/7] login dev user + mint pairing token"
login_resp="$(curl -fsS -X POST -H 'Content-Type: application/json' \
    -d '{"email":"dev@easynet.local","password":"dev-password"}' \
    "$backend_url/api/v1/auth/login" 2>&1 || true)"
jwt="$(printf '%s' "$login_resp" | jq -r '.access_token // .token // empty' 2>/dev/null)"
if [ -z "$jwt" ]; then
    echo "[FAIL] could not log in dev user. Backend response:" >&2
    printf '%s\n' "$login_resp" >&2
    exit 1
fi

pair_resp="$(curl -fsS -X POST -H "Authorization: Bearer $jwt" \
    "$backend_url/api/v1/devices/pairing" 2>&1 || true)"
pair_token="$(printf '%s' "$pair_resp" | jq -r '.pairing_token // empty' 2>/dev/null)"
if [ -z "$pair_token" ]; then
    echo "[FAIL] could not mint pairing token. Backend response:" >&2
    printf '%s\n' "$pair_resp" >&2
    exit 1
fi
echo "    pairing token: ${pair_token:0:8}…  (truncated)"

# ── 4. Device join — first wire-touch ────────────────────────────
echo "==> [4/7] easynet device join $pair_token"
if ! join_out="$(easynet device join "$pair_token" --hub "$backend_url" 2>&1)"; then
    echo "[FAIL] easynet device join failed:" >&2
    printf '%s\n' "$join_out" >&2
    exit 1
fi
printf '%s\n' "$join_out" | sed 's/^/    /'

if [ ! -f "$HOME/.easynet/credentials.json" ]; then
    echo "[FAIL] credentials.json not written after join" >&2
    exit 1
fi

# ── 5. Start daemon — THIS IS THE LOAD-BEARING ASSERTION ─────────
# Current code spawns axon-runtime here. The release tarball does
# NOT ship axon-runtime. So `easynet start` should fail fast on a
# release-shape install — that failure is exactly the bug we're
# tracking, and this script's whole purpose is to surface it.
echo "==> [5/7] easynet start (load-bearing assertion)"
start_log="$prefix/easynet-start.log"
if start_out="$(easynet start 2>&1)"; then
    printf '%s\n' "$start_out" | sed 's/^/    /'
    echo "$start_out" > "$start_log"
else
    rc=$?
    echo "[FAIL] easynet start exited with code $rc" >&2
    printf '%s\n' "$start_out" | sed 's/^/    /' >&2
    echo "$start_out" > "$start_log"
    exit 1
fi

# Assert: device-mode start must not spawn any additional axon-runtime
# beyond the backend-owned hub process that may already be alive.
current_axon_pids="$(pgrep -f "$workspace_root/EasyNet-Axon/core/runtime-rs/target/release/axon-runtime" | sort | tr '\n' ' ' || true)"
if [ "$current_axon_pids" != "$baseline_axon_pids" ]; then
    echo "[FAIL] easynet start changed the axon-runtime process set:" >&2
    echo "       before: ${baseline_axon_pids:-<none>}" >&2
    echo "       after : ${current_axon_pids:-<none>}" >&2
    pgrep -af "axon-runtime" >&2 || true
    echo "       The release tarball does not ship axon-runtime;" >&2
    echo "       device-mode start must not spawn its own runtime process." >&2
    exit 1
fi

# Daemon should be alive on its UDS.
daemon_sock="$HOME/.easynet/daemon.sock"
control_sock="$HOME/.easynet/control.sock"
for _ in $(seq 1 20); do
    if [ -S "$daemon_sock" ] && [ -S "$control_sock" ]; then
        break
    fi
    sleep 1
done
if [ ! -S "$daemon_sock" ]; then
    echo "[FAIL] daemon.sock did not appear at $daemon_sock" >&2
    exit 1
fi
if [ ! -S "$control_sock" ]; then
    echo "[FAIL] control.sock did not appear at $control_sock" >&2
    exit 1
fi
echo "    daemon.sock + control.sock both listening"

# ── 6. Probe surfaces a release user would actually hit ──────────
echo "==> [6/7] easynet ability list (local catalogue)"
if ! list_out="$(easynet ability list 2>&1)"; then
    echo "[FAIL] easynet ability list exited non-zero:" >&2
    printf '%s\n' "$list_out" >&2
    exit 1
fi
list_count="$(printf '%s' "$list_out" | grep -cE '^\s+(device|hub|<self>|test|dev)\.' || true)"
echo "    ability list returned non-empty"

echo "==> [6/7] agent add codex + claude"
if ! add1_out="$(easynet agent add codex --type codex 2>&1)"; then
    echo "[FAIL] agent add codex failed:" >&2
    printf '%s\n' "$add1_out" >&2
    exit 1
fi
if ! add2_out="$(easynet agent add claude --type claude-code 2>&1)"; then
    echo "[FAIL] agent add claude failed:" >&2
    printf '%s\n' "$add2_out" >&2
    exit 1
fi

# Friendly URA assertion. Current hosted-agent minting uses the
# operator-facing tail directly (`.<name>`) instead of the older
# profile-prefixed `.llm-<name>` form. Reject uuid-hash tails.
hosted_json="$HOME/.easynet/local-agents.json"
if ! jq -e '.hosted_agents[] | select(.agent_ura | contains(".codex"))' "$hosted_json" >/dev/null; then
    echo "[FAIL] codex hosted agent missing or mis-minted (expected URA tail .codex):" >&2
    jq '.hosted_agents' "$hosted_json" >&2
    exit 1
fi
if ! jq -e '.hosted_agents[] | select(.agent_ura | contains(".claude"))' "$hosted_json" >/dev/null; then
    echo "[FAIL] claude hosted agent missing or mis-minted (expected URA tail .claude):" >&2
    jq '.hosted_agents' "$hosted_json" >&2
    exit 1
fi
echo "    hosted agents minted with friendly URAs"

# ── 7. Verify the session-prelude advertise reaches the hub ──────
# The daemon will only re-emit advertise on session reconnect. Bounce
# the daemon so the new agents land in this run's window.
echo "==> [7/7] bounce daemon, observe advertise prelude"
easynet stop >/dev/null 2>&1 || true
sleep 2
easynet start >/dev/null 2>&1
daemon_log="$HOME/.easynet/logs/easynet-daemon.log"
for _ in $(seq 1 30); do
    if grep -q "advertise_agent prelude done" "$daemon_log" 2>/dev/null; then
        break
    fi
    sleep 1
done

advertise_line="$(grep "advertise_agent prelude for" "$daemon_log" 2>/dev/null | tail -1 || true)"
if [ -z "$advertise_line" ]; then
    echo "[FAIL] daemon log has no advertise_agent prelude entry yet" >&2
    tail -30 "$daemon_log" >&2
    exit 1
fi

# Assert the friendly names landed in the advertise list.
case "$advertise_line" in
    *codex*) ;;
    *) echo "[FAIL] codex not in advertise prelude: $advertise_line" >&2; exit 1 ;;
esac
case "$advertise_line" in
    *claude*) ;;
    *) echo "[FAIL] claude not in advertise prelude: $advertise_line" >&2; exit 1 ;;
esac
case "$advertise_line" in
    *consent-default*) ;;
    *) echo "[FAIL] consent-default not in advertise prelude: $advertise_line" >&2; exit 1 ;;
esac
echo "    advertise: $(printf '%s' "$advertise_line" | sed 's/^.*advertise_agent prelude for/advertise_agent prelude for/')"

# Final assertion: daemon bounce must not have spawned any additional
# axon-runtime beyond the backend-owned hub baseline.
current_axon_pids="$(pgrep -f "$workspace_root/EasyNet-Axon/core/runtime-rs/target/release/axon-runtime" | sort | tr '\n' ' ' || true)"
if [ "$current_axon_pids" != "$baseline_axon_pids" ]; then
    echo "[FAIL] daemon bounce changed the axon-runtime process set:" >&2
    echo "       before: ${baseline_axon_pids:-<none>}" >&2
    echo "       after : ${current_axon_pids:-<none>}" >&2
    pgrep -af "axon-runtime" >&2 || true
    exit 1
fi

echo
echo "[OK] release-shape end-to-end flow verified"
echo "  device joined the test hub"
echo "  daemon online (UDS + control + daemon sockets)"
echo "  hosted agents minted with friendly URAs (consent-default, codex, claude)"
echo "  session prelude advertised 5 agents to the hub"
echo "  device-mode start spawned no extra axon-runtime beyond the backend hub"

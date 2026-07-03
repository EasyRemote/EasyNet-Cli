#!/usr/bin/env bash
# examples/public-routes-e2e/_lib.sh — shared helpers for the killer-demo scripts.
# Source this from each d{1..5}-*.sh: `source "$(dirname "$0")/_lib.sh"`.
#
# Convention:
#   USER_ID  = "alice"   (the daemon's EASYNET_PAGES_USER)
#   PORT     = 8787      (the in-daemon hub listener)
#   EASYNET  = the dev binary or whatever's first on PATH

USER_ID="${EASYNET_PAGES_USER:-alice}"
PORT="${EASYNET_PAGES_PORT:-8787}"
EASYNET="${EASYNET:-easynet}"
DAEMON_LOG="${DAEMON_LOG:-/tmp/easynet-daemon.log}"
WEBAPPS_DIR="$HOME/.easynet/web-apps"

# Color helpers (work in any tty, gracefully degrade in pipes).
if [ -t 1 ]; then
    BOLD=$'\033[1m'; DIM=$'\033[2m'; RED=$'\033[31m'; GRN=$'\033[32m'
    YEL=$'\033[33m'; BLU=$'\033[34m'; MAG=$'\033[35m'; CYN=$'\033[36m'
    RST=$'\033[0m'
else
    BOLD=""; DIM=""; RED=""; GRN=""; YEL=""; BLU=""; MAG=""; CYN=""; RST=""
fi

step()  { printf "%s━━ %s%s\n" "$BOLD" "$*" "$RST"; }
note()  { printf "%s  %s%s\n" "$DIM" "$*" "$RST"; }
ok()    { printf "%s  ✓ %s%s\n" "$GRN" "$*" "$RST"; }
warn()  { printf "%s  ⚠ %s%s\n" "$YEL" "$*" "$RST"; }
fail()  { printf "%s  ✗ %s%s\n" "$RED" "$*" "$RST" >&2; }

# Sanity preflight. Most demos need: easynet on PATH, daemon
# running on PORT, EASYNET_PAGES_USER set on the daemon side.
ensure_daemon() {
    if ! command -v "$EASYNET" >/dev/null 2>&1; then
        fail "easynet binary not found on PATH"
        note "  expected: $EASYNET"
        note "  fix:      ln -sf .../target/debug/easynet ~/.local/bin/easynet"
        exit 1
    fi
    if ! lsof -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
        fail "no daemon listening on :$PORT"
        note "  start one with:"
        note "    EASYNET_PAGES_PORT=$PORT EASYNET_PAGES_USER=$USER_ID easynet-daemon &"
        exit 1
    fi
    ok "daemon up on :$PORT (user=$USER_ID)"
}

# Run a CLI command with EASYNET_PAGES_USER set. Echoes the
# command first (dimmed) so silan can see what's running.
run() {
    printf "%s$ %s%s\n" "$DIM" "$*" "$RST"
    EASYNET_PAGES_USER="$USER_ID" "$@"
}

# Open a URL in the default browser (macOS).
open_browser() {
    local url="$1"
    if command -v open >/dev/null 2>&1; then
        ( open "$url" >/dev/null 2>&1 ) || true
    fi
}

pause() {
    if [ "${EASYNET_DEMO_NONINTERACTIVE:-0}" = "1" ]; then
        return
    fi
    printf "\n%s  press <enter> to continue...%s" "$DIM" "$RST"
    read -r _ || true
    printf "\n"
}

# Pretty curl with status + content-type + truncated body.
http_curl() {
    local method="$1"; shift
    local url="$1"; shift
    local extra=("$@")
    local out
    out=$(curl -sS -X "$method" -w '\n--HTTP-STATUS-- %{http_code}\n--CT-- %{content_type}\n' "${extra[@]}" "$url" 2>&1)
    printf "%s\n" "$out"
}

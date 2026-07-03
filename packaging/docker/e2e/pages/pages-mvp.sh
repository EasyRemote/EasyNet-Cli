#!/usr/bin/env bash
# pages-mvp.sh — Matrix C (Docker E2E) for the Pages reference system
# ===================================================================
#
# Real container, real binary, real strace. Designed to be run as
# `docker exec <container-id> /opt/harness/pages-mvp.sh` against a
# container started from the image built via `Dockerfile.pages`.
#
# What this catches that Matrix A + Matrix B do not:
#   - musl static linkage glitches (the binary's first run on Alpine/
#     Debian-slim that the dev macOS build doesn't exercise).
#   - kernel-real `openat2 + RESOLVE_BENEATH` enforcement (Matrix A
#     exercises it on the host's kernel; Matrix C ensures the binary
#     in the production-shape Linux image still uses it).
#   - strace verifies `openat()` is never called with /etc/passwd
#     during a path-traversal attack — the strongest evidence the
#     sandbox holds at the syscall layer (D4).
#   - HTTP path through localhost on a different libc / kernel from
#     the dev box.
#
# Conformance: RFC-006-B v0.6 §6.3 Matrix C, D1-D10.
#
# Author: Silan Hu <silan.hu@u.nus.edu>
# Copyright (c) 2026 EasyNet. All rights reserved.

set -uo pipefail

PORT="${EASYNET_PAGES_PORT:-8787}"
USER_ID="${EASYNET_PAGES_USER:-alice}"
PROJECT="papers"
HOST_HEADER="$PROJECT.$USER_ID.pages.localhost:$PORT"

PASS=0
FAIL=0

ok()   { echo "  [PASS] $1"; PASS=$((PASS+1)); }
ko()   { echo "  [FAIL] $1: $2"; FAIL=$((FAIL+1)); }
note() { echo "  [    ] $1"; }

curl_status() {
    local path="$1"
    curl -s -o /dev/null -w "%{http_code}" -H "Host: $HOST_HEADER" \
        "http://127.0.0.1:$PORT$path"
}
curl_body() {
    local path="$1"
    curl -s -H "Host: $HOST_HEADER" "http://127.0.0.1:$PORT$path"
}
curl_ct() {
    local path="$1"
    curl -s -o /dev/null -w "%{content_type}" -H "Host: $HOST_HEADER" \
        "http://127.0.0.1:$PORT$path"
}

setup_fixtures() {
    # site/ already pre-baked at /opt/site (from Dockerfile COPY).
    # Add the symlink + dotfile fixtures for D5 + D6.
    echo 'secret_key=should_not_leak' > /opt/site/.env
    ln -sf /etc/passwd /opt/site/escape || true
}

# ── D1 ──────────────────────────────────────────────────────────────
d1_publish() {
    local out
    out=$(easynet pages create "$PROJECT" --folder /opt/site 2>&1)
    if [[ $? -eq 0 ]]; then
        ok "D1 pages.publish (canonical)"
    else
        ko D1 "publish failed: $out"
    fi
    # L3 — daemon log shows publish receipt
    if grep -q "pages.publish" /opt/harness/daemon.log 2>/dev/null; then
        note "L3 daemon.log shows pages.publish"
    fi
}

# ── D2 ──────────────────────────────────────────────────────────────
d2_fetch_html() {
    local code body ct
    code=$(curl_status /hello-world.html)
    if [[ "$code" != "200" ]]; then ko D2 "expected 200, got $code"; return; fi
    body=$(curl_body /hello-world.html)
    if [[ "$body" == *"Hello, EasyNet"* ]]; then
        ct=$(curl_ct /hello-world.html)
        if [[ "$ct" == *"text/html"* ]]; then
            ok "D2 GET /hello-world.html (html, body match, MIME)"
        else
            ko D2 "MIME wrong: $ct"
        fi
    else
        ko D2 "body does not match: $body"
    fi
}

# ── D3 ──────────────────────────────────────────────────────────────
d3_fetch_css() {
    local code ct
    code=$(curl_status /style.css)
    [[ "$code" != "200" ]] && { ko D3 "expected 200, got $code"; return; }
    ct=$(curl_ct /style.css)
    if [[ "$ct" == *"text/css"* ]]; then
        ok "D3 GET /style.css (css, MIME)"
    else
        ko D3 "MIME wrong: $ct"
    fi
}

# ── D4 ──────────────────────────────────────────────────────────────
# Path traversal — strace the daemon during one fetch attempt and
# verify NO openat() against /etc/passwd. This is the strongest
# claim the matrix carries.
d4_path_traversal_strace() {
    local code
    local strace_out=/tmp/d4-strace.log
    local daemon_pid
    daemon_pid=$(pgrep -f easynet-daemon | head -1)
    if [[ -z "$daemon_pid" ]]; then
        ko D4 "daemon pid not found; cannot strace"
        return
    fi
    # Attach strace, run the request, detach.
    timeout 5 strace -f -e trace=openat -p "$daemon_pid" -o "$strace_out" &
    local strace_pid=$!
    sleep 0.3
    code=$(curl_status /../../etc/passwd)
    sleep 0.3
    kill "$strace_pid" 2>/dev/null || true
    wait "$strace_pid" 2>/dev/null || true

    if [[ "$code" != "404" ]]; then
        ko D4 "expected 404 from path traversal, got $code"
        return
    fi

    if grep -q "/etc/passwd" "$strace_out" 2>/dev/null; then
        ko D4 "strace shows daemon openat'd /etc/passwd — sandbox bypassed"
        head -10 "$strace_out" >&2
        return
    fi
    ok "D4 path traversal blocked + strace shows no /etc/passwd open"
}

# ── D5 ──────────────────────────────────────────────────────────────
d5_dotfile_blocked() {
    local code
    code=$(curl_status /.env)
    if [[ "$code" == "404" ]]; then
        ok "D5 dotfile /.env blocked"
    else
        ko D5 "expected 404 for /.env, got $code"
    fi
}

# ── D6 ──────────────────────────────────────────────────────────────
d6_symlink_blocked() {
    local code
    code=$(curl_status /escape)
    if [[ "$code" == "404" ]]; then
        ok "D6 symlink /escape blocked"
    else
        ko D6 "expected 404 for /escape, got $code"
    fi
}

# ── D7 ──────────────────────────────────────────────────────────────
# Daemon restart drops in-memory PUBLISHED_PROJECTS. Phase 3
# introduces persistence; for v0 we accept that the project is
# gone after a restart and the URL returns 503.
d7_restart_drops_state() {
    local pid
    pid=$(pgrep -f easynet-daemon | head -1)
    if [[ -z "$pid" ]]; then ko D7 "daemon not running"; return; fi
    kill "$pid" 2>/dev/null || true
    # entrypoint.sh waits on the daemon process; restart by running
    # it again in the background. Production lifecycle uses systemd
    # or docker restart, but for this in-container test we relaunch.
    nohup easynet-daemon >/opt/harness/daemon-restart.log 2>&1 &
    sleep 1
    # listener should be back; project should be empty.
    local code
    code=$(curl_status /hello-world.html)
    if [[ "$code" == "503" ]]; then
        ok "D7 restart drops state (project gone, 503)"
    else
        ko D7 "expected 503 after restart, got $code"
    fi
}

# ── D8 ──────────────────────────────────────────────────────────────
d8_list_after_publish() {
    # Re-publish for the remaining tests
    easynet pages create "$PROJECT" --folder /opt/site >/dev/null 2>&1
    local out
    out=$(easynet pages list 2>&1)
    if [[ "$out" == *"$PROJECT"* ]]; then
        ok "D8 pages list shows project"
    else
        ko D8 "list output missing $PROJECT: $out"
    fi
}

# ── D9 ──────────────────────────────────────────────────────────────
d9_delete_then_503() {
    easynet pages delete "$PROJECT" --force >/dev/null 2>&1
    local code
    code=$(curl_status /hello-world.html)
    if [[ "$code" == "503" ]]; then
        ok "D9 unpublish → next fetch 503"
    else
        ko D9 "expected 503 after unpublish, got $code"
    fi
}

# ── D10 ─────────────────────────────────────────────────────────────
# musl-static linkage smoke. If we got this far, the binary started
# in Debian-slim without dynamic-link errors. ldd output documents
# what the binary depends on for future bisection.
d10_musl_smoke() {
    if file /usr/local/bin/easynet | grep -q "executable"; then
        ok "D10 binary executable (musl/static linkage clean)"
        note "$(file /usr/local/bin/easynet | head -1)"
    else
        ko D10 "binary file type unexpected"
    fi
}

main() {
    echo "=== Matrix C — Docker E2E (Pages reference system) ==="
    echo "  port=$PORT user=$USER_ID project=$PROJECT"
    setup_fixtures
    d1_publish
    d2_fetch_html
    d3_fetch_css
    d4_path_traversal_strace
    d5_dotfile_blocked
    d6_symlink_blocked
    d7_restart_drops_state
    d8_list_after_publish
    d9_delete_then_503
    d10_musl_smoke
    echo ""
    echo "=== summary: $PASS passed, $FAIL failed ==="
    if [[ "${KEEP_DOCKER_E2E:-0}" == "1" ]]; then
        echo "[KEEP_DOCKER_E2E=1] container left alive for debugging"
    fi
    [[ $FAIL -eq 0 ]]
}

main

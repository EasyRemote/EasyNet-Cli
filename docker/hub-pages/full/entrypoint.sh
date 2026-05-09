#!/usr/bin/env bash
# entrypoint.sh — daemon + auto-publish boot sequence.
#
# Boots easynet-daemon in the background, waits for the IPC
# socket + HTTP listener, then walks /sites/ and publishes every
# subdir as a project named after the directory. Holds the
# foreground on the daemon process so SIGTERM from `docker stop`
# propagates cleanly.
set -euo pipefail

PORT="${EASYNET_PAGES_PORT:-8787}"

mkdir -p "$HOME/.easynet"
echo "[entrypoint] starting daemon (port=$PORT, user=${EASYNET_PAGES_USER}, realm=${EASYNET_PAGES_REALM})"

easynet-daemon &
DAEMON_PID=$!

# Wait for IPC sock + HTTP listener readiness.
for _ in $(seq 1 60); do
    if [ -S "$HOME/.easynet/control.sock" ] \
        && (echo > /dev/tcp/127.0.0.1/$PORT) 2>/dev/null; then
        echo "[entrypoint] daemon ready"
        break
    fi
    sleep 0.5
done

if ! [ -S "$HOME/.easynet/control.sock" ]; then
    echo "[entrypoint] daemon failed to bind IPC socket within 30s"
    exit 1
fi

# Auto-publish every /sites/<project_id>/ as a pages project.
if [ -d /sites ]; then
    for dir in /sites/*/; do
        [ -d "$dir" ] || continue
        slug=$(basename "$dir")
        echo "[entrypoint] publishing $slug from $dir"
        easynet pages create "$slug" --folder "$dir" 2>&1 | tail -5 || true
    done
fi

echo "[entrypoint] entering foreground"
wait $DAEMON_PID

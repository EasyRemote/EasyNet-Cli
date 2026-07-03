#!/usr/bin/env bash
# entrypoint.sh — start daemon in background, hold container alive.
#
# Usage at run time:
#   docker run -d -p 8787:8787 easynet-pages-e2e
#   docker exec <id> /opt/harness/pages-mvp.sh
#
# The harness expects the daemon to be ready on entry (listener on
# 8787 + IPC sock at ~/.easynet/control.sock).
set -euo pipefail

# Daemon writes IPC socket under $HOME/.easynet/.
mkdir -p "$HOME/.easynet"

# Start daemon in background with the Pages listener enabled. Logs
# stream to /opt/harness/daemon.log so the harness can scrape them
# during failure post-mortem.
nohup easynet-daemon >/opt/harness/daemon.log 2>&1 &
DAEMON_PID=$!

# Wait for IPC sock + listener; bail out loudly if either is missing
# after a generous timeout.
for _ in $(seq 1 30); do
    if [[ -S "$HOME/.easynet/control.sock" ]]; then
        if (echo > /dev/tcp/127.0.0.1/${EASYNET_PAGES_PORT:-8787}) 2>/dev/null; then
            echo "[entrypoint] daemon ready (pid=$DAEMON_PID)"
            break
        fi
    fi
    sleep 0.5
done

# If we reach here without finding both, that is the failure mode —
# tail the log so `docker logs` shows the operator what went wrong.
if ! [[ -S "$HOME/.easynet/control.sock" ]]; then
    echo "[entrypoint] daemon failed to bind IPC socket within 15s"
    cat /opt/harness/daemon.log >&2
    exit 1
fi

# Hold the container alive on the daemon process. SIGTERM
# from `docker stop` propagates and the daemon exits.
wait $DAEMON_PID

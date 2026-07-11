#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-daemon-key-service-boundary.sh"

bash "$CHECK" >/dev/null

echo "test_check_daemon_key_service_boundary ok"

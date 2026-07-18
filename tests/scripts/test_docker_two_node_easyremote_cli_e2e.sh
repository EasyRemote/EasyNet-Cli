#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

bash "$ROOT/tools/scripts/docker-two-node-easyremote-cli-e2e.sh" --self-test >/dev/null

echo "test_docker_two_node_easyremote_cli_e2e ok"

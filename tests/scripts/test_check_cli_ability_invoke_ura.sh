#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exec bash "$ROOT/engineering/tests/scripts/test_check_cli_ability_invoke_ura.sh" "$@"

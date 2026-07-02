#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exec bash "$ROOT/engineering/tests/scripts/test_check_system_ability_resource_refs.sh" "$@"

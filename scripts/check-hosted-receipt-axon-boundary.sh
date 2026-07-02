#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec "$ROOT/engineering/scripts/check-hosted-receipt-axon-boundary.sh" "$@"

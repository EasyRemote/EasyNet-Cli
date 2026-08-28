#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bash "$ROOT/tools/scripts/host-remoteapp-native-host-e2e.sh" --self-test


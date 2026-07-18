#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# The daemon process contract includes the key-service sidecar. Every hermetic
# product or SDK test must build both executables from the same source tree;
# relying on an older target/easynet-keyring makes clean-checkout tests
# nondeterministic.
exec cargo build \
  --manifest-path "$ROOT/Cargo.toml" \
  "$@" \
  --bin easynet-daemon \
  --bin easynet-keyring

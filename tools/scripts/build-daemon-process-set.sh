#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# The daemon process contract includes the key-service sidecar and the private
# RemoteApp target-observation and media hosts. Every hermetic product or SDK test must build all
# executables from the same source tree;
# relying on an older target/easynet-keyring makes clean-checkout tests
# nondeterministic.
exec cargo build \
  --manifest-path "$ROOT/Cargo.toml" \
  "$@" \
  -p easynet \
  --bin easynet-daemon \
  --bin easynet-keyring \
  -p easynet-remoteapp-native-host \
  --bin easynet-remoteapp-native-host \
  -p easynet-remoteapp-media-host \
  --bin easynet-remoteapp-media-host

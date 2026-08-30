#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
BUILD_SCRIPT="$REPO_ROOT/tools/scripts/build-linux-cli-artifact-bundle.sh"
VERIFY_SCRIPT="$REPO_ROOT/tools/scripts/verify-linux-cli-artifact-bundle.py"

bash -n "$BUILD_SCRIPT"
python3 -m py_compile "$VERIFY_SCRIPT"

headless_output="$(bash "$BUILD_SCRIPT" --media-profile headless --self-test)"
native_output="$(bash "$BUILD_SCRIPT" --media-profile native --self-test)"
native_dev_output="$(bash "$BUILD_SCRIPT" --media-profile native --build-profile dev --self-test)"

grep -q 'media profile: headless' <<<"$headless_output"
grep -q 'media profile: native' <<<"$native_output"
grep -q 'builder: zig' <<<"$headless_output"
grep -q 'builder: docker' <<<"$native_output"
grep -q 'build profile: release' <<<"$native_output"
grep -q 'build profile: dev' <<<"$native_dev_output"
grep -q 'runtime-build-profile.json' "$BUILD_SCRIPT"
grep -q 'media_profile' "$BUILD_SCRIPT"
grep -q 'builder' "$BUILD_SCRIPT"
grep -q 'provenance schema: 3' <<<"$native_output"
grep -q 'git_worktree_digest' "$BUILD_SCRIPT"
grep -q 'source changed while the artifact bundle was building' "$BUILD_SCRIPT"
grep -q 'worktree_sha256' "$BUILD_SCRIPT"
grep -q 'builder_identity' "$BUILD_SCRIPT"
grep -q 'ability-descriptors.*tree_sha256' "$BUILD_SCRIPT"
grep -q 'descriptor_tree_identity' "$VERIFY_SCRIPT"
grep -q 'require_clean_source' "$VERIFY_SCRIPT"
grep -q 'git_source_identity' "$VERIFY_SCRIPT"
grep -q 'expect-easynet-cli-source' "$VERIFY_SCRIPT"
for artifact in easynet easynet-daemon easynet-keyring easynet-remoteapp-native-host easynet-remoteapp-media-host libeasynet_cli.so libaxon_dendrite_bridge.so; do
  grep -Fq "\\\"$artifact\\\": {\\\"sha256\\\"" "$BUILD_SCRIPT"
  grep -Fq "\"$artifact\"" "$VERIFY_SCRIPT"
done

if bash "$BUILD_SCRIPT" --media-profile unsupported --self-test >/dev/null 2>&1; then
  echo '[FAIL] unsupported media profile was accepted' >&2
  exit 1
fi

if bash "$BUILD_SCRIPT" --builder unsupported --self-test >/dev/null 2>&1; then
  echo '[FAIL] unsupported artifact builder was accepted' >&2
  exit 1
fi

if bash "$BUILD_SCRIPT" --media-profile native --builder zig --self-test >/dev/null 2>&1; then
  echo '[FAIL] native media profile accepted the cross builder' >&2
  exit 1
fi

if bash "$BUILD_SCRIPT" --build-profile unsupported --self-test >/dev/null 2>&1; then
  echo '[FAIL] unsupported build profile was accepted' >&2
  exit 1
fi

echo 'test_build_linux_cli_artifact_bundle ok'

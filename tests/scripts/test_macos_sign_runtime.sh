#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/packaging/release/macos-sign-runtime.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

stage="$TMP/stage"
mock_bin="$TMP/bin"
mkdir -p "$stage" "$mock_bin"
for artifact in \
    libaxon_dendrite_bridge.dylib \
    libeasynet_cli.dylib \
    easynet-keyring \
    easynet-remoteapp-native-host \
    easynet-remoteapp-media-host \
    easynet-daemon \
    easynet; do
    : > "$stage/$artifact"
done

cat > "$mock_bin/uname" <<'SH'
#!/usr/bin/env bash
echo Darwin
SH
cat > "$mock_bin/codesign" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
artifact="${!#}"
name="$(basename "$artifact")"
case "$name" in
  libaxon_dendrite_bridge.dylib) identifier=run.easynet.dendrite-bridge ;;
  libeasynet_cli.dylib) identifier=run.easynet.runtime-c-abi ;;
  easynet-keyring) identifier=run.easynet.keyring ;;
  easynet-remoteapp-native-host) identifier=run.easynet.remoteapp.native-host ;;
  easynet-remoteapp-media-host) identifier=run.easynet.remoteapp.media-host ;;
  easynet-daemon) identifier=run.easynet.daemon ;;
  easynet) identifier=run.easynet.cli ;;
  *) exit 2 ;;
esac
printf '%s\n' "$*" >> "$MOCK_CODESIGN_LOG"
if [[ " $* " == *" -r- "* ]]; then
  echo "designated => identifier \"$identifier\" and anchor apple generic" >&2
elif [[ " $* " == *" -dv "* ]]; then
  echo "Identifier=$identifier" >&2
  echo "TeamIdentifier=${MOCK_CODESIGN_TEAM:-A1B2C3D4E5}" >&2
  echo "CodeDirectory v=20500 size=1 flags=0x10000(runtime) hashes=1+0 location=embedded" >&2
fi
SH
chmod +x "$mock_bin/uname" "$mock_bin/codesign"

export PATH="$mock_bin:$PATH"
export MOCK_CODESIGN_LOG="$TMP/codesign.log"
export EASYNET_MACOS_CODESIGN_IDENTITY='Developer ID Application: EasyNet Test (A1B2C3D4E5)'
export EASYNET_MACOS_TEAM_ID=A1B2C3D4E5
bash "$SCRIPT" --stage-dir "$stage" >/dev/null
grep -Fq -- '--identifier run.easynet.remoteapp.media-host --sign Developer ID Application: EasyNet Test (A1B2C3D4E5)' "$MOCK_CODESIGN_LOG"
grep -Fq -- '--identifier run.easynet.runtime-c-abi --sign Developer ID Application: EasyNet Test (A1B2C3D4E5)' "$MOCK_CODESIGN_LOG"
grep -Fq -- '--identifier run.easynet.daemon --sign Developer ID Application: EasyNet Test (A1B2C3D4E5)' "$MOCK_CODESIGN_LOG"

unset EASYNET_MACOS_CODESIGN_IDENTITY
if bash "$SCRIPT" --stage-dir "$stage" >/dev/null 2>&1; then
    echo 'test_macos_sign_runtime: missing signing identity unexpectedly passed' >&2
    exit 1
fi

export MOCK_CODESIGN_TEAM=Z9Y8X7W6V5
if bash "$SCRIPT" --stage-dir "$stage" --verify-only >/dev/null 2>&1; then
    echo 'test_macos_sign_runtime: wrong Team ID unexpectedly passed' >&2
    exit 1
fi

echo 'test_macos_sign_runtime: all cases passed'

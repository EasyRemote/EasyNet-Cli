#!/usr/bin/env bash
# Reproducible Linux/X11 RemoteApp provider + real Browser product proof.
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
WORKSPACE_ROOT="$(cd "$REPO_ROOT/.." && pwd)"
EASYNET_ROOT="${EASYNET_ROOT:-$WORKSPACE_ROOT/EasyNet}"
FRONTEND_ROOT="${EASYNET_FRONTEND_ROOT:-$EASYNET_ROOT/Frontend}"
ARTIFACT_ROOT="${EASYNET_CLI_ARTIFACT_ROOT:-$EASYNET_ROOT/target/dev-backend/cli-artifacts-v2}"
FRONTEND_URL="${EASYNET_REMOTEAPP_FRONTEND_URL:-http://127.0.0.1:3000}"
HUB_API="${EASYNET_REMOTEAPP_HUB_API:-http://127.0.0.1:8080}"
EMAIL="${EASYNET_REMOTEAPP_EMAIL:-dev@easynet.local}"
PASSWORD="${EASYNET_REMOTEAPP_PASSWORD:-dev-password}"
TARGET_KIND="both"
MODE="skip"
OUT_DIR="${EASYNET_REMOTEAPP_LINUX_PROVIDER_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-linux-provider-browser/$(date -u +%Y%m%d-%H%M%S)-$$}"
BASE_IMAGE="${EASYNET_REMOTEAPP_LINUX_PROVIDER_BASE_IMAGE:-easynet/device-e2e:local}"
PROVIDER_IMAGE="${EASYNET_REMOTEAPP_LINUX_PROVIDER_IMAGE:-easynet/remoteapp-linux-provider-e2e:local}"
CONTAINER="easynet-remoteapp-linux-provider-$$"
KEEP_CONTAINER="${EASYNET_REMOTEAPP_LINUX_PROVIDER_KEEP_CONTAINER:-0}"

usage() {
  cat <<'USAGE'
Usage:
  remoteapp-linux-provider-browser-e2e.sh --run [--target-kind window|application|both]
  remoteapp-linux-provider-browser-e2e.sh --self-test

Options:
  --run                 Build and pair a real Linux/X11 provider, then run Browser E2E.
  --self-test           Validate runner structure without Docker or network access.
  --target-kind KIND    window, application, or both (default).
  --frontend-url URL    Running EasyNet frontend URL.
  --hub-api URL         Running Hub REST URL.
  --artifact-root DIR   Provenance-bound native Linux CLI artifact bundle.
  --out-dir DIR         Durable evidence output directory.

The runner creates an isolated provider container, starts Xvfb/Openbox plus
selected and unrelated process-owned sentinel applications, pairs through the
public Hub API without storing the pairing token in Docker metadata, runs the
real Playwright lifecycle, and verifies Linux Window/Application input remains
explicitly view-only because XTest cannot isolate a press/release lifecycle to
one target. Set EASYNET_REMOTEAPP_LINUX_PROVIDER_KEEP_CONTAINER=1 to retain the
provider for diagnosis.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) MODE=self-test; shift ;;
    --target-kind) TARGET_KIND="${2:?missing target kind}"; shift 2 ;;
    --frontend-url) FRONTEND_URL="${2:?missing frontend URL}"; shift 2 ;;
    --hub-api) HUB_API="${2:?missing Hub API URL}"; shift 2 ;;
    --artifact-root) ARTIFACT_ROOT="${2:?missing artifact root}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing output directory}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

case "$TARGET_KIND" in window|application|both) ;; *) echo "invalid target kind: $TARGET_KIND" >&2; exit 64 ;; esac

if [[ "$MODE" == self-test ]]; then
  bash -n "$0"
  grep -q 'remoteapp-linux-x11-sentinel.py' "$REPO_ROOT/packaging/docker/e2e/linux-remoteapp-provider/Dockerfile"
  grep -q -- '--expected-input-mode view_only' "$0"
  if grep -Eq '^[[:space:]]*EASYNET_REMOTEAPP_BROWSER_REQUIRE_HOST_INPUT_EFFECTS=1' "$0"; then
    echo 'Linux target runner must not claim isolated Window/Application host input effects' >&2
    exit 1
  fi
  grep -q 'runtime-build-profile.json' "$0"
  grep -q 'docker exec -i' "$0"
  echo 'remoteapp-linux-provider-browser-e2e self-test ok'
  exit 0
fi
[[ "$MODE" == run ]] || { echo '[SKIP] pass --run to create live product evidence'; exit 0; }

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
PROVIDER_LOG="$OUT_DIR/provider.log"
PROVIDER_MANIFEST="$OUT_DIR/provider-runtime-build-profile.json"
DOCKERFILE="$REPO_ROOT/packaging/docker/e2e/linux-remoteapp-provider/Dockerfile"
VERIFY_BUNDLE="$SELF_DIR/verify-linux-cli-artifact-bundle.py"
BROWSER_RUNNER="$FRONTEND_ROOT/scripts/remoteapp-browser-lifecycle.mjs"
LEAF_VERIFIER="$SELF_DIR/frontend-remoteapp-browser-lifecycle-e2e.sh"
MATRIX_VERIFIER="$SELF_DIR/frontend-remoteapp-browser-lifecycle-matrix-e2e.sh"

cleanup() {
  if [[ "$KEEP_CONTAINER" != 1 ]]; then
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

for executable in docker curl jq python3; do command -v "$executable" >/dev/null || { echo "missing executable: $executable" >&2; exit 1; }; done
for path in "$DOCKERFILE" "$VERIFY_BUNDLE" "$BROWSER_RUNNER" "$LEAF_VERIFIER" "$MATRIX_VERIFIER"; do
  [[ -f "$path" ]] || { echo "missing runner dependency: $path" >&2; exit 1; }
done
curl -fsS "$HUB_API/api/v1/health" >/dev/null
curl -fsS "$FRONTEND_URL/" >/dev/null
"$VERIFY_BUNDLE" "$ARTIFACT_ROOT" \
  --expect-media-profile native \
  --expect-easynet-cli-source "$REPO_ROOT" \
  --expect-easynet-axon-source "$WORKSPACE_ROOT/EasyNet-Axon"

echo '==> building isolated Linux/X11 RemoteApp provider image'
docker build \
  --build-arg "BASE_IMAGE=$BASE_IMAGE" \
  --file "$DOCKERFILE" \
  --tag "$PROVIDER_IMAGE" \
  "$REPO_ROOT" >"$OUT_DIR/provider-image-build.log"

docker run -d \
  --name "$CONTAINER" \
  --hostname easynet-remoteapp-linux-provider \
  --add-host host.docker.internal:host-gateway \
  --env HOME=/srv/easynet \
  --env DISPLAY=:99 \
  "$PROVIDER_IMAGE" >/dev/null

docker exec "$CONTAINER" sh -ec '
  nohup socat TCP-LISTEN:8080,bind=127.0.0.1,reuseaddr,fork TCP:host.docker.internal:8080 >/tmp/socat-http.log 2>&1 &
  nohup socat TCP-LISTEN:50443,bind=127.0.0.1,reuseaddr,fork TCP:host.docker.internal:50443 >/tmp/socat-tls.log 2>&1 &
  nohup Xvfb :99 -screen 0 1280x800x24 -ac +extension RANDR >/tmp/xvfb.log 2>&1 &
  i=0; until DISPLAY=:99 xdpyinfo >/dev/null 2>&1; do i=$((i + 1)); [ "$i" -lt 100 ] || exit 1; sleep 0.1; done
  nohup env DISPLAY=:99 openbox >/tmp/openbox.log 2>&1 &
  nohup env DISPLAY=:99 \
    EASYNET_REMOTEAPP_LINUX_SENTINEL_STATE=/tmp/selected-state.json \
    EASYNET_REMOTEAPP_LINUX_SENTINEL_CLASS=EasyNetRemoteAppSelected \
    EASYNET_REMOTEAPP_LINUX_SENTINEL_TITLE_PREFIX="EasyNet RemoteApp Selected" \
    EASYNET_REMOTEAPP_LINUX_SENTINEL_ROLE=selected_target \
    python3 /opt/easynet/e2e/remoteapp-linux-x11-sentinel.py >/tmp/selected-sentinel.log 2>&1 &
  echo $! >/tmp/selected-sentinel.pid
  nohup env DISPLAY=:99 \
    EASYNET_REMOTEAPP_LINUX_SENTINEL_STATE=/tmp/unrelated-state.json \
    EASYNET_REMOTEAPP_LINUX_SENTINEL_CLASS=EasyNetRemoteAppUnrelated \
    EASYNET_REMOTEAPP_LINUX_SENTINEL_TITLE_PREFIX="EasyNet RemoteApp Unrelated" \
    EASYNET_REMOTEAPP_LINUX_SENTINEL_ROLE=unrelated_process \
    EASYNET_REMOTEAPP_LINUX_SENTINEL_PRIMARY_GEOMETRY=360x240+40+520 \
    EASYNET_REMOTEAPP_LINUX_SENTINEL_SECONDARY_GEOMETRY=360x240+440+520 \
    python3 /opt/easynet/e2e/remoteapp-linux-x11-sentinel.py >/tmp/unrelated-sentinel.log 2>&1 &
  echo $! >/tmp/unrelated-sentinel.pid
'

for _ in $(seq 1 100); do
  if docker exec "$CONTAINER" sh -ec "jq -e '.windows | length == 2' /tmp/selected-state.json >/dev/null 2>&1 && jq -e '.windows | length == 2' /tmp/unrelated-state.json >/dev/null 2>&1"; then
    break
  fi
  sleep 0.1
done
docker exec "$CONTAINER" jq -e '.windows | length == 2' /tmp/selected-state.json >/dev/null
docker exec "$CONTAINER" jq -e '.windows | length == 2' /tmp/unrelated-state.json >/dev/null

auth_json="$(curl -fsS -H 'Content-Type: application/json' -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" "$HUB_API/api/v1/auth/login")"
auth_token="$(jq -r '.access_token // .token // .data.access_token // .data.token // empty' <<<"$auth_json")"
[[ -n "$auth_token" ]] || { echo 'Hub login did not return an access token' >&2; exit 1; }
pairing_json="$(curl -fsS -X POST -H "Authorization: Bearer $auth_token" -H 'Content-Type: application/json' -d '{}' "$HUB_API/api/v1/devices/pairing")"
pairing_token="$(jq -r '.pairing_token // empty' <<<"$pairing_json")"
[[ -n "$pairing_token" ]] || { echo 'Hub did not return a pairing token' >&2; exit 1; }

echo '==> pairing Linux provider and starting current native daemon'
if ! printf '%s\n' "$pairing_token" | docker exec -i "$CONTAINER" sh -ec '
  IFS= read -r pairing_token
  easynet device join "$pairing_token" --hub http://127.0.0.1:8080 --yes --boot yes
' >"$PROVIDER_LOG" 2>&1; then
  docker exec "$CONTAINER" sh -ec 'cat /tmp/xvfb.log /tmp/openbox.log /tmp/selected-sentinel.log /tmp/unrelated-sentinel.log 2>/dev/null || true' >>"$PROVIDER_LOG"
  echo "Linux provider join failed; see $PROVIDER_LOG" >&2
  exit 1
fi
unset pairing_token pairing_json

device_id="$(docker exec "$CONTAINER" easynet status --json | jq -r '.connection.node_id // empty')"
[[ -n "$device_id" ]] || { echo 'paired provider status has no node id' >&2; exit 1; }
online=0
for _ in $(seq 1 120); do
  state="$(curl -fsS -H "Authorization: Bearer $auth_token" "$HUB_API/api/v1/devices" | jq -r --arg id "$device_id" '(.items // .data.items // [])[] | select(.node_id == $id) | .state')"
  if [[ "$state" == ONLINE ]]; then online=1; break; fi
  sleep 0.5
done
[[ "$online" == 1 ]] || { echo "provider $device_id did not become ONLINE" >&2; exit 1; }
docker cp "$CONTAINER:/opt/easynet/runtime-build-profile.json" "$PROVIDER_MANIFEST"
cmp -s "$PROVIDER_MANIFEST" "$ARTIFACT_ROOT/runtime-build-profile.json" || {
  echo 'provider image runtime manifest does not match the verified artifact bundle; rebuild the EasyNet E2E images' >&2
  exit 1
}

runner_command="node $BROWSER_RUNNER"

run_leaf() {
  local kind="$1"
  local leaf="$OUT_DIR/$kind"
  local target_label
  case "$kind" in
    window) target_label='EasyNet RemoteApp Selected' ;;
    application) target_label='Easynetremoteappselected' ;;
    *) echo "unsupported RemoteApp target kind: $kind" >&2; return 64 ;;
  esac
  mkdir -p "$leaf"
  EASYNET_REMOTEAPP_BROWSER_DEVICE_ID="$device_id" \
  EASYNET_REMOTEAPP_BROWSER_EMAIL="$EMAIL" \
  EASYNET_REMOTEAPP_BROWSER_PASSWORD="$PASSWORD" \
  EASYNET_REMOTEAPP_BROWSER_TARGET_KIND="$kind" \
  EASYNET_REMOTEAPP_BROWSER_TARGET_LABEL="$target_label" \
  "$LEAF_VERIFIER" --run --frontend-url "$FRONTEND_URL" --runner-cmd "$runner_command" --out-dir "$leaf"
}

case "$TARGET_KIND" in
  window) run_leaf window ;;
  application) run_leaf application ;;
  both)
    run_leaf window
    run_leaf application
    "$MATRIX_VERIFIER" --run \
      --window-report "$OUT_DIR/window/report.json" \
      --application-report "$OUT_DIR/application/report.json" \
      --expected-input-mode view_only \
      --out-dir "$OUT_DIR/matrix"
    ;;
esac

jq -n \
  --arg device_id "$device_id" \
  --arg container "$CONTAINER" \
  --arg image "$PROVIDER_IMAGE" \
  --arg manifest "$PROVIDER_MANIFEST" \
  --arg manifest_sha256 "$(shasum -a 256 "$PROVIDER_MANIFEST" | awk '{print $1}')" \
  --arg target_kind "$TARGET_KIND" \
  '{status:"passed",proof_mode:"current_build_linux_x11_provider_browser",device_id:$device_id,provider_container:$container,provider_image:$image,target_kind:$target_kind,runtime_build_profile:$manifest,runtime_build_profile_sha256:$manifest_sha256,product_complete_claim:false}' \
  >"$OUT_DIR/provider-report.json"
echo "[OK] Linux RemoteApp provider Browser evidence: $OUT_DIR"

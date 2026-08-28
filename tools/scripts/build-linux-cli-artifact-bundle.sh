#!/usr/bin/env bash
# Build the Linux CLI runtime artifact bundle consumed by Docker E2E images.
#
# This script is intentionally host-driven: Docker image assembly receives a
# complete runtime bundle and does not compile Rust crates as a hidden fallback.
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
WORKSPACE_ROOT="$(cd "$REPO_ROOT/.." && pwd)"
AXON_ROOT="${EASYNET_AXON_ROOT:-$WORKSPACE_ROOT/EasyNet-Axon}"
BRIDGE_ROOT="$AXON_ROOT/core/runtime-rs/dendrite-bridge"
CARGO_BIN="${CARGO_BIN:-}"
OUT_DIR="${EASYNET_CLI_ARTIFACT_OUT_DIR:-}"
TARGET="${EASYNET_CLI_LINUX_TARGET:-}"
MEDIA_PROFILE="${EASYNET_CLI_MEDIA_PROFILE:-headless}"
BUILDER="${EASYNET_CLI_ARTIFACT_BUILDER:-auto}"
BUILD_PROFILE="${EASYNET_CLI_BUILD_PROFILE:-release}"
NATIVE_BUILD_DOCKERFILE="$REPO_ROOT/packaging/docker/build/linux-native/Dockerfile"
SELF_TEST=0

usage() {
  cat <<'USAGE'
Usage:
  tools/scripts/build-linux-cli-artifact-bundle.sh [options]

Options:
  --out-dir DIR      Destination artifact bundle directory.
  --target TRIPLE    Linux Rust target. Defaults to host-matching Linux GNU.
  --media-profile P  Media implementation profile: headless (default) or native.
  --builder B        Build strategy: auto (default), zig, or docker.
  --build-profile P  Cargo build profile: release (default) or dev.
  --self-test        Validate script structure without compiling.
  -h, --help         Show this help.

Environment:
  EASYNET_CLI_ARTIFACT_OUT_DIR  Same as --out-dir.
  EASYNET_CLI_LINUX_TARGET      Same as --target.
  EASYNET_CLI_MEDIA_PROFILE     Same as --media-profile.
  EASYNET_CLI_ARTIFACT_BUILDER  Same as --builder.
  EASYNET_CLI_BUILD_PROFILE     Same as --build-profile.
  EASYNET_AXON_ROOT             Sibling EasyNet-Axon repository root.
  CARGO_BIN                     Cargo executable. Defaults to PATH lookup, then
                                common rustup installation paths.
  ZIG                           Zig executable used by cargo-zigbuild. Defaults
                                to PATH lookup, then common Homebrew paths.
  EASYNET_CLI_MIN_FD_LIMIT      Minimum soft file descriptor limit requested
                                before cargo-zigbuild. Defaults to 4096.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    --target) TARGET="${2:?missing value for --target}"; shift 2 ;;
    --media-profile) MEDIA_PROFILE="${2:?missing value for --media-profile}"; shift 2 ;;
    --builder) BUILDER="${2:?missing value for --builder}"; shift 2 ;;
    --build-profile) BUILD_PROFILE="${2:?missing value for --build-profile}"; shift 2 ;;
    --self-test) SELF_TEST=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

die() {
  echo "[FAIL] $*" >&2
  exit 1
}

sha256_stream() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 | awk '{print $1}'
  else
    die "missing SHA-256 implementation: expected sha256sum or shasum"
  fi
}

sha256_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
  else
    die "missing SHA-256 implementation: expected sha256sum or shasum"
  fi
}

file_size_bytes() {
  local path="$1"
  wc -c <"$path" | tr -d '[:space:]'
}

git_revision() {
  local repository="$1"
  git -C "$repository" rev-parse HEAD
}

git_dirty_json() {
  local repository="$1"
  if [[ -n "$(git -C "$repository" status --porcelain --untracked-files=normal)" ]]; then
    printf 'true\n'
  else
    printf 'false\n'
  fi
}

# Bind an artifact to the complete repository state used to compile it, not
# merely HEAD. `git diff HEAD` covers all tracked staged/unstaged bytes; every
# untracked path is paired with Git's content hash. Ignored build outputs are
# intentionally excluded because they are not compiler inputs.
git_worktree_digest() {
  local repository="$1"
  {
    printf 'revision\0%s\0' "$(git_revision "$repository")"
    printf 'tracked-diff\0'
    git -C "$repository" diff --binary HEAD --
    printf '\0untracked\0'
    while IFS= read -r -d '' relative_path; do
      printf '%s\0%s\0' \
        "$relative_path" \
        "$(git -C "$repository" hash-object -- "$relative_path")"
    done < <(git -C "$repository" ls-files --others --exclude-standard -z)
  } | sha256_stream
}

directory_tree_digest() {
  local directory="$1"
  {
    while IFS= read -r path; do
      local relative_path="${path#"$directory"/}"
      printf '%s\0%s\0' "$relative_path" "$(sha256_file "$path")"
    done < <(find "$directory" -type f -print | LC_ALL=C sort)
  } | sha256_stream
}

directory_file_count() {
  local directory="$1"
  find "$directory" -type f -print | wc -l | tr -d '[:space:]'
}

resolve_cargo() {
  if [[ -n "${CARGO_BIN:-}" ]]; then
    [[ -x "$CARGO_BIN" ]] || die "CARGO_BIN is not executable: $CARGO_BIN"
    printf '%s\n' "$CARGO_BIN"
    return 0
  fi
  local candidate
  for candidate in \
    cargo \
    "$HOME/.cargo/bin/cargo" \
    /usr/local/bin/cargo \
    /opt/homebrew/bin/cargo
  do
    if [[ "$candidate" == */* ]]; then
      if [[ -x "$candidate" ]]; then
        printf '%s\n' "$candidate"
        return 0
      fi
    elif command -v "$candidate" >/dev/null 2>&1; then
      command -v "$candidate"
      return 0
    fi
  done
  die "missing cargo executable: cargo"
}

resolve_zig() {
  if [[ -n "${ZIG:-}" ]]; then
    [[ -x "$ZIG" ]] || die "ZIG is not executable: $ZIG"
    printf '%s\n' "$ZIG"
    return 0
  fi
  local candidate
  for candidate in \
    zig \
    /opt/homebrew/bin/zig \
    /usr/local/bin/zig
  do
    if [[ "$candidate" == */* ]]; then
      if [[ -x "$candidate" ]]; then
        printf '%s\n' "$candidate"
        return 0
      fi
    elif command -v "$candidate" >/dev/null 2>&1; then
      command -v "$candidate"
      return 0
    fi
  done
  die "missing zig executable: install zig or set ZIG=/path/to/zig"
}

raise_fd_limit() {
  local desired="${EASYNET_CLI_MIN_FD_LIMIT:-4096}"
  [[ "$desired" =~ ^[0-9]+$ ]] || die "EASYNET_CLI_MIN_FD_LIMIT must be numeric: $desired"

  local current hard target
  current="$(ulimit -n 2>/dev/null || echo 0)"
  hard="$(ulimit -H -n 2>/dev/null || echo "$current")"
  [[ "$current" =~ ^[0-9]+$ ]] || return 0
  (( current >= desired )) && return 0

  target="$desired"
  if [[ "$hard" =~ ^[0-9]+$ ]] && (( hard < target )); then
    target="$hard"
  fi
  if [[ "$target" =~ ^[0-9]+$ ]] && (( target > current )); then
    if ! ulimit -n "$target" 2>/dev/null; then
      echo "[WARN] unable to raise file descriptor limit from $current to $target" >&2
      return 0
    fi
  fi
}

host_default_target() {
  case "$(uname -m)" in
    arm64|aarch64) echo "aarch64-unknown-linux-gnu" ;;
    x86_64|amd64) echo "x86_64-unknown-linux-gnu" ;;
    *) return 1 ;;
  esac
}

TARGET="${TARGET:-$(host_default_target || true)}"
[[ -n "$TARGET" ]] || die "unsupported host arch: $(uname -m)"
case "$TARGET" in
  aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu) ;;
  *) die "unsupported Linux artifact target: $TARGET" ;;
esac
case "$MEDIA_PROFILE" in
  headless) MEDIA_FEATURE="headless-media" ;;
  native) MEDIA_FEATURE="native-media" ;;
  *) die "unsupported media profile: $MEDIA_PROFILE (expected headless or native)" ;;
esac
case "$BUILDER" in
  auto)
    if [[ "$MEDIA_PROFILE" == "native" ]]; then
      BUILDER="docker"
    else
      BUILDER="zig"
    fi
    ;;
  zig|docker) ;;
  *) die "unsupported artifact builder: $BUILDER (expected auto, zig, or docker)" ;;
esac
if [[ "$MEDIA_PROFILE" == "native" && "$BUILDER" != "docker" ]]; then
  die "native media artifacts require the Docker Linux-native builder"
fi
case "$BUILD_PROFILE" in
  release)
    CARGO_PROFILE_ARGS=(--release)
    CARGO_PROFILE_DIR="release"
    ;;
  dev)
    CARGO_PROFILE_ARGS=()
    CARGO_PROFILE_DIR="debug"
    ;;
  *) die "unsupported build profile: $BUILD_PROFILE (expected release or dev)" ;;
esac

[[ -d "$BRIDGE_ROOT" ]] || die "EasyNet-Axon dendrite bridge not found: $BRIDGE_ROOT"
[[ -d "$REPO_ROOT/ability-descriptors/system" ]] || die "missing system ability descriptors"

if [[ "$SELF_TEST" == "1" ]]; then
  bash -n "$0"
  grep -q "resolve_cargo" "$0"
  grep -q "resolve_zig" "$0"
  grep -q "raise_fd_limit" "$0"
  grep -q "cargo zigbuild" "$0"
  grep -q "libeasynet_cli.so" "$0"
  grep -q "libaxon_dendrite_bridge.so" "$0"
  grep -q "ability-descriptors" "$0"
  grep -q -- "--media-profile" "$0"
  grep -q -- "--builder" "$0"
  grep -q -- "--build-profile" "$0"
  grep -q "runtime-build-profile.json" "$0"
  grep -q '"schema_version": 3' "$0"
  [[ "$(printf provenance | sha256_stream)" =~ ^[0-9a-f]{64}$ ]]
  [[ "$(git_worktree_digest "$REPO_ROOT")" =~ ^[0-9a-f]{64}$ ]]
  echo "build-linux-cli-artifact-bundle media profile: $MEDIA_PROFILE"
  echo "build-linux-cli-artifact-bundle builder: $BUILDER"
  echo "build-linux-cli-artifact-bundle build profile: $BUILD_PROFILE"
  echo "build-linux-cli-artifact-bundle provenance schema: 3"
  echo "build-linux-cli-artifact-bundle self-test ok"
  exit 0
fi

[[ -n "$OUT_DIR" ]] || die "--out-dir or EASYNET_CLI_ARTIFACT_OUT_DIR is required"
CLI_SOURCE_REVISION="$(git_revision "$REPO_ROOT")"
CLI_SOURCE_DIRTY="$(git_dirty_json "$REPO_ROOT")"
CLI_SOURCE_DIGEST="$(git_worktree_digest "$REPO_ROOT")"
AXON_SOURCE_REVISION="$(git_revision "$AXON_ROOT")"
AXON_SOURCE_DIRTY="$(git_dirty_json "$AXON_ROOT")"
AXON_SOURCE_DIGEST="$(git_worktree_digest "$AXON_ROOT")"
BUILDER_IMAGE_NAME=""
BUILDER_IMAGE_ID=""
rm -rf "$OUT_DIR.tmp"
mkdir -p "$OUT_DIR.tmp"

echo "==> building EasyNet CLI artifact bundle for $TARGET ($MEDIA_PROFILE media, $BUILDER builder, $BUILD_PROFILE profile)"
if [[ "$BUILDER" == "zig" ]]; then
  CARGO_BIN="$(resolve_cargo)"
  ZIG="$(resolve_zig)"
  export ZIG
  CARGO_BIN_DIR="$(dirname "$CARGO_BIN")"
  ZIG_BIN_DIR="$(dirname "$ZIG")"
  export PATH="$CARGO_BIN_DIR:$ZIG_BIN_DIR:$PATH"
  "$CARGO_BIN" zigbuild --help >/dev/null 2>&1 || die "cargo zigbuild is required"
  raise_fd_limit
  (
    cd "$REPO_ROOT"
    "$CARGO_BIN" zigbuild "${CARGO_PROFILE_ARGS[@]}" --target "$TARGET" -p easynet --lib \
      --bin easynet --bin easynet-daemon --bin easynet-keyring \
      --no-default-features \
      --features "axon-pb,$MEDIA_FEATURE,remote-desktop"
    "$CARGO_BIN" zigbuild "${CARGO_PROFILE_ARGS[@]}" --target "$TARGET" \
      -p easynet-remoteapp-native-host --bin easynet-remoteapp-native-host \
      --no-default-features --features "$MEDIA_FEATURE"
    "$CARGO_BIN" zigbuild "${CARGO_PROFILE_ARGS[@]}" --target "$TARGET" \
      -p easynet-remoteapp-media-host --bin easynet-remoteapp-media-host \
      --no-default-features --features "$MEDIA_FEATURE"
  )
  (
    cd "$BRIDGE_ROOT"
    "$CARGO_BIN" zigbuild "${CARGO_PROFILE_ARGS[@]}" --target "$TARGET"
  )
  cp "$REPO_ROOT/target/$TARGET/$CARGO_PROFILE_DIR/easynet" "$OUT_DIR.tmp/easynet"
  cp "$REPO_ROOT/target/$TARGET/$CARGO_PROFILE_DIR/easynet-daemon" "$OUT_DIR.tmp/easynet-daemon"
  cp "$REPO_ROOT/target/$TARGET/$CARGO_PROFILE_DIR/easynet-keyring" "$OUT_DIR.tmp/easynet-keyring"
  cp "$REPO_ROOT/target/$TARGET/$CARGO_PROFILE_DIR/easynet-remoteapp-native-host" "$OUT_DIR.tmp/easynet-remoteapp-native-host"
  cp "$REPO_ROOT/target/$TARGET/$CARGO_PROFILE_DIR/easynet-remoteapp-media-host" "$OUT_DIR.tmp/easynet-remoteapp-media-host"
  cp "$REPO_ROOT/target/$TARGET/$CARGO_PROFILE_DIR/libeasynet_cli.so" "$OUT_DIR.tmp/libeasynet_cli.so"
  cp "$BRIDGE_ROOT/target/$TARGET/$CARGO_PROFILE_DIR/libaxon_dendrite_bridge.so" \
    "$OUT_DIR.tmp/libaxon_dendrite_bridge.so"
else
  command -v docker >/dev/null 2>&1 || die "Docker is required for native Linux media artifacts"
  docker info >/dev/null 2>&1 || die "Docker daemon is not reachable"
  [[ -f "$NATIVE_BUILD_DOCKERFILE" ]] || die "missing native build Dockerfile: $NATIVE_BUILD_DOCKERFILE"
  case "$TARGET" in
    aarch64-unknown-linux-gnu) DOCKER_ARCH="arm64" ;;
    x86_64-unknown-linux-gnu) DOCKER_ARCH="amd64" ;;
  esac
  BUILD_IMAGE="easynet-cli-linux-native-build:bookworm-$DOCKER_ARCH"
  docker build \
    --platform "linux/$DOCKER_ARCH" \
    --tag "$BUILD_IMAGE" \
    --file "$NATIVE_BUILD_DOCKERFILE" \
    "$REPO_ROOT/packaging/docker/build/linux-native"
  BUILDER_IMAGE_NAME="$BUILD_IMAGE"
  BUILDER_IMAGE_ID="$(docker image inspect --format '{{.Id}}' "$BUILD_IMAGE")"
  RLIB_MANIFEST="$OUT_DIR.tmp/Cargo.rlib.toml"
  CDYLIB_MANIFEST="$OUT_DIR.tmp/Cargo.cdylib.toml"
  sed 's/^crate-type = \["rlib", "cdylib", "staticlib"\]$/crate-type = ["rlib"]/' \
    "$REPO_ROOT/Cargo.toml" >"$RLIB_MANIFEST"
  sed 's/^crate-type = \["rlib", "cdylib", "staticlib"\]$/crate-type = ["cdylib"]/' \
    "$REPO_ROOT/Cargo.toml" >"$CDYLIB_MANIFEST"
  grep -q '^crate-type = \["rlib"\]$' "$RLIB_MANIFEST" \
    || die "failed to project rlib-only native build manifest"
  grep -q '^crate-type = \["cdylib"\]$' "$CDYLIB_MANIFEST" \
    || die "failed to project cdylib-only native build manifest"

  DOCKER_RUN_ARGS=(
    run --rm
    --platform "linux/$DOCKER_ARCH"
    --volume "$WORKSPACE_ROOT:/work:ro"
    --volume "$OUT_DIR.tmp:/out"
    --volume "easynet-cli-cargo-registry-$DOCKER_ARCH:/usr/local/cargo/registry"
    --volume "easynet-cli-cargo-git-$DOCKER_ARCH:/usr/local/cargo/git"
    --volume "easynet-cli-target-$DOCKER_ARCH:/build/cli-target"
    --volume "easynet-bridge-target-$DOCKER_ARCH:/build/bridge-target"
    --workdir /work/EasyNet-Cli
    --env "CLI_TARGET=$TARGET"
    --env "CLI_MEDIA_FEATURE=$MEDIA_FEATURE"
    --env "CLI_BUILD_PROFILE=$BUILD_PROFILE"
    --env "CLI_PROFILE_DIR=$CARGO_PROFILE_DIR"
    --env CARGO_BUILD_JOBS=1
    --env CARGO_PROFILE_RELEASE_CODEGEN_UNITS=32
    --env CARGO_PROFILE_DEV_DEBUG=0
    --env CARGO_PROFILE_DEV_CODEGEN_UNITS=1024
    --env CARGO_INCREMENTAL=0
    --env 'RUSTFLAGS=-C linker=clang -C link-arg=-fuse-ld=lld'
  )

  docker "${DOCKER_RUN_ARGS[@]}" \
    --volume "$RLIB_MANIFEST:/work/EasyNet-Cli/Cargo.toml:ro" \
    "$BUILD_IMAGE" \
    bash -euo pipefail -c '
      profile_args=()
      [[ "$CLI_BUILD_PROFILE" == "release" ]] && profile_args+=(--release)
      CARGO_TARGET_DIR=/build/cli-target cargo build --locked "${profile_args[@]}" --target "$CLI_TARGET" -p easynet \
        --bin easynet --bin easynet-daemon --bin easynet-keyring \
        --no-default-features \
        --features "axon-pb,$CLI_MEDIA_FEATURE,remote-desktop"
      CARGO_TARGET_DIR=/build/cli-target cargo build --locked "${profile_args[@]}" --target "$CLI_TARGET" \
        -p easynet-remoteapp-native-host --bin easynet-remoteapp-native-host \
        --no-default-features --features "$CLI_MEDIA_FEATURE"
      CARGO_TARGET_DIR=/build/cli-target cargo build --locked "${profile_args[@]}" --target "$CLI_TARGET" \
        -p easynet-remoteapp-media-host --bin easynet-remoteapp-media-host \
        --no-default-features --features "$CLI_MEDIA_FEATURE"
      CARGO_TARGET_DIR=/build/bridge-target cargo build \
        --manifest-path /work/EasyNet-Axon/core/runtime-rs/dendrite-bridge/Cargo.toml \
        --locked "${profile_args[@]}" --target "$CLI_TARGET"
      install -m 0755 "/build/cli-target/$CLI_TARGET/$CLI_PROFILE_DIR/easynet" /out/easynet
      install -m 0755 "/build/cli-target/$CLI_TARGET/$CLI_PROFILE_DIR/easynet-daemon" /out/easynet-daemon
      install -m 0755 "/build/cli-target/$CLI_TARGET/$CLI_PROFILE_DIR/easynet-keyring" /out/easynet-keyring
      install -m 0755 "/build/cli-target/$CLI_TARGET/$CLI_PROFILE_DIR/easynet-remoteapp-native-host" /out/easynet-remoteapp-native-host
      install -m 0755 "/build/cli-target/$CLI_TARGET/$CLI_PROFILE_DIR/easynet-remoteapp-media-host" /out/easynet-remoteapp-media-host
      install -m 0644 "/build/bridge-target/$CLI_TARGET/$CLI_PROFILE_DIR/libaxon_dendrite_bridge.so" /out/libaxon_dendrite_bridge.so
    '
  docker "${DOCKER_RUN_ARGS[@]}" \
    --volume "$CDYLIB_MANIFEST:/work/EasyNet-Cli/Cargo.toml:ro" \
    "$BUILD_IMAGE" \
    bash -euo pipefail -c '
      profile_args=()
      [[ "$CLI_BUILD_PROFILE" == "release" ]] && profile_args+=(--release)
      CARGO_TARGET_DIR=/build/cli-target cargo build --locked "${profile_args[@]}" --target "$CLI_TARGET" -p easynet --lib \
        --no-default-features \
        --features "axon-pb,$CLI_MEDIA_FEATURE,remote-desktop"
      install -m 0644 "/build/cli-target/$CLI_TARGET/$CLI_PROFILE_DIR/libeasynet_cli.so" /out/libeasynet_cli.so
    '
  rm -f "$RLIB_MANIFEST" "$CDYLIB_MANIFEST"
fi

[[ "$(git_worktree_digest "$REPO_ROOT")" == "$CLI_SOURCE_DIGEST" ]] \
  || die "EasyNet-Cli source changed while the artifact bundle was building"
[[ "$(git_worktree_digest "$AXON_ROOT")" == "$AXON_SOURCE_DIGEST" ]] \
  || die "EasyNet-Axon source changed while the artifact bundle was building"

cp -R "$REPO_ROOT/ability-descriptors" "$OUT_DIR.tmp/ability-descriptors"
EASYNET_SHA256="$(sha256_file "$OUT_DIR.tmp/easynet")"
EASYNET_SIZE="$(file_size_bytes "$OUT_DIR.tmp/easynet")"
DAEMON_SHA256="$(sha256_file "$OUT_DIR.tmp/easynet-daemon")"
DAEMON_SIZE="$(file_size_bytes "$OUT_DIR.tmp/easynet-daemon")"
KEYRING_SHA256="$(sha256_file "$OUT_DIR.tmp/easynet-keyring")"
KEYRING_SIZE="$(file_size_bytes "$OUT_DIR.tmp/easynet-keyring")"
REMOTEAPP_NATIVE_HOST_SHA256="$(sha256_file "$OUT_DIR.tmp/easynet-remoteapp-native-host")"
REMOTEAPP_NATIVE_HOST_SIZE="$(file_size_bytes "$OUT_DIR.tmp/easynet-remoteapp-native-host")"
REMOTEAPP_MEDIA_HOST_SHA256="$(sha256_file "$OUT_DIR.tmp/easynet-remoteapp-media-host")"
REMOTEAPP_MEDIA_HOST_SIZE="$(file_size_bytes "$OUT_DIR.tmp/easynet-remoteapp-media-host")"
C_ABI_SHA256="$(sha256_file "$OUT_DIR.tmp/libeasynet_cli.so")"
C_ABI_SIZE="$(file_size_bytes "$OUT_DIR.tmp/libeasynet_cli.so")"
BRIDGE_SHA256="$(sha256_file "$OUT_DIR.tmp/libaxon_dendrite_bridge.so")"
BRIDGE_SIZE="$(file_size_bytes "$OUT_DIR.tmp/libaxon_dendrite_bridge.so")"
DESCRIPTORS_SHA256="$(directory_tree_digest "$OUT_DIR.tmp/ability-descriptors")"
DESCRIPTORS_COUNT="$(directory_file_count "$OUT_DIR.tmp/ability-descriptors")"
printf '%s\n' \
  '{' \
  '  "schema_version": 3,' \
  "  \"target\": \"$TARGET\"," \
  "  \"media_profile\": \"$MEDIA_PROFILE\"," \
  "  \"builder\": \"$BUILDER\"," \
  "  \"build_profile\": \"$BUILD_PROFILE\"," \
  "  \"cargo_features\": [\"axon-pb\", \"$MEDIA_FEATURE\", \"remote-desktop\"]," \
  '  "source": {' \
  "    \"easynet_cli\": {\"revision\": \"$CLI_SOURCE_REVISION\", \"dirty\": $CLI_SOURCE_DIRTY, \"worktree_sha256\": \"$CLI_SOURCE_DIGEST\"}," \
  "    \"easynet_axon\": {\"revision\": \"$AXON_SOURCE_REVISION\", \"dirty\": $AXON_SOURCE_DIRTY, \"worktree_sha256\": \"$AXON_SOURCE_DIGEST\"}" \
  '  },' \
  '  "builder_identity": {' \
  "    \"image\": \"$BUILDER_IMAGE_NAME\"," \
  "    \"image_id\": \"$BUILDER_IMAGE_ID\"" \
  '  },' \
  '  "artifacts": {' \
  "    \"easynet\": {\"sha256\": \"$EASYNET_SHA256\", \"bytes\": $EASYNET_SIZE}," \
  "    \"easynet-daemon\": {\"sha256\": \"$DAEMON_SHA256\", \"bytes\": $DAEMON_SIZE}," \
  "    \"easynet-keyring\": {\"sha256\": \"$KEYRING_SHA256\", \"bytes\": $KEYRING_SIZE}," \
  "    \"easynet-remoteapp-native-host\": {\"sha256\": \"$REMOTEAPP_NATIVE_HOST_SHA256\", \"bytes\": $REMOTEAPP_NATIVE_HOST_SIZE}," \
  "    \"easynet-remoteapp-media-host\": {\"sha256\": \"$REMOTEAPP_MEDIA_HOST_SHA256\", \"bytes\": $REMOTEAPP_MEDIA_HOST_SIZE}," \
  "    \"libeasynet_cli.so\": {\"sha256\": \"$C_ABI_SHA256\", \"bytes\": $C_ABI_SIZE}," \
  "    \"libaxon_dendrite_bridge.so\": {\"sha256\": \"$BRIDGE_SHA256\", \"bytes\": $BRIDGE_SIZE}," \
  "    \"ability-descriptors\": {\"tree_sha256\": \"$DESCRIPTORS_SHA256\", \"files\": $DESCRIPTORS_COUNT}" \
  '  }' \
  '}' >"$OUT_DIR.tmp/runtime-build-profile.json"

for required in \
  easynet \
  easynet-daemon \
  easynet-keyring \
  easynet-remoteapp-native-host \
  easynet-remoteapp-media-host \
  libeasynet_cli.so \
  libaxon_dendrite_bridge.so \
  runtime-build-profile.json
do
  [[ -f "$OUT_DIR.tmp/$required" ]] || die "artifact missing after build: $required"
done
[[ -d "$OUT_DIR.tmp/ability-descriptors/system" ]] || die "artifact missing ability descriptors"

rm -rf "$OUT_DIR"
mv "$OUT_DIR.tmp" "$OUT_DIR"
echo "[OK] CLI artifact bundle: $OUT_DIR"
echo "  target: $TARGET"
echo "  media profile: $MEDIA_PROFILE"
echo "  builder: $BUILDER"
echo "  build profile: $BUILD_PROFILE"

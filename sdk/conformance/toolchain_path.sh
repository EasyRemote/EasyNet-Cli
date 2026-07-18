#!/usr/bin/env bash

# Resolve the released SDK toolchains from their standard installation roots.
# Desktop and CI launchers frequently provide a minimal PATH; conformance must
# attest the locked compiler versions, not fail based on the launcher's shell
# initialization policy.
resolve_sdk_toolchain_path() {
  local source_root="$1"
  local contract="$source_root/sdk/conformance/toolchains.json"
  local node_version=""
  local prefix=()
  local directory

  if [[ -f "$contract" ]]; then
    node_version="$(
      awk -F'"' '/"node"[[:space:]]*:/{print $4; exit}' "$contract"
    )"
  fi

  for directory in \
    "$HOME/.cargo/bin" \
    "/opt/homebrew/bin" \
    "/usr/local/go/bin" \
    "$HOME/go/bin" \
    "${node_version:+$HOME/.nvm/versions/node/v$node_version/bin}" \
    "$HOME/.sdkman/candidates/java/current/bin" \
    "$HOME/.sdkman/candidates/maven/current/bin"
  do
    if [[ -n "$directory" && -d "$directory" ]]; then
      prefix+=("$directory")
    fi
  done

  if ((${#prefix[@]} > 0)); then
    local joined
    joined="$(IFS=:; printf '%s' "${prefix[*]}")"
    PATH="$joined:$PATH"
    export PATH
  fi
}

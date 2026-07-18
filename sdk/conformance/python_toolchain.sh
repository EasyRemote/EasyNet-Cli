#!/usr/bin/env bash

resolve_sdk_python_toolchain() {
  local source_root="$1"
  shift
  local candidates=()
  local candidate resolved
  local module available

  if [[ -n "${SDK_CONFORMANCE_PYTHON:-}" ]]; then
    candidates+=("$SDK_CONFORMANCE_PYTHON")
  else
    if [[ -n "${PYTHON:-}" ]]; then
      candidates+=("$PYTHON")
    fi
    candidates+=("$source_root/sdk/python/.venv/bin/python")
    command -v python3 >/dev/null 2>&1 && candidates+=("$(command -v python3)")
    command -v python >/dev/null 2>&1 && candidates+=("$(command -v python)")
  fi

  for candidate in "${candidates[@]}"; do
    resolved="$candidate"
    if [[ "$resolved" != */* ]]; then
      resolved="$(command -v "$resolved" 2>/dev/null || true)"
    fi
    if [[ -z "$resolved" || ! -x "$resolved" ]]; then
      continue
    fi
    available=1
    for module in "$@"; do
      if ! "$resolved" -m "$module" --version >/dev/null 2>&1; then
        available=0
        break
      fi
    done
    if [[ "$available" -eq 1 ]]; then
      SDK_CONFORMANCE_PYTHON="$resolved"
      PATH="$(dirname "$resolved"):$PATH"
      export SDK_CONFORMANCE_PYTHON
      export PATH
      return 0
    fi
  done

  echo "sdk-conformance: no Python interpreter satisfies required modules: $*" >&2
  return 1
}

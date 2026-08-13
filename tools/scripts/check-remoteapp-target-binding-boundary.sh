#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_TARGET_BINDING_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
REMOTE_ROOT="$ROOT/plugins/remote-desktop/src"

fail() {
  printf 'check-remoteapp-target-binding-boundary: %s\n' "$1" >&2
  exit 1
}

require() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  rg -q -- "$pattern" "$path" || fail "$message"
}

reject() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if rg -q -- "$pattern" "$path"; then
    fail "$message"
  fi
}

[[ -d "$REMOTE_ROOT" ]] || fail "missing remote desktop source root"

is_test_context() {
  local file="$1"
  local line="$2"
  awk -v line="$line" '
    NR > line { exit(test_context ? 0 : 1) }
    /#\[cfg\(test\)\]/ { saw_cfg_test = 1; next }
    saw_cfg_test && /mod tests/ { test_context = 1; saw_cfg_test = 0; next }
    saw_cfg_test && $0 !~ /^[[:space:]]*$/ { saw_cfg_test = 0 }
    END {
      if (NR <= line) {
        exit(test_context ? 0 : 1)
      }
    }
  ' "$file"
}

# SPEC: after target_binding exists, production media/input paths must not
# convert ResourceEntry directly into native capture or input targets.
reject 'target_for_entry\s*\(' "$REMOTE_ROOT" \
  'production must not resolve native capture targets from ResourceEntry'
reject 'input_policy_for_entry\s*\(' "$REMOTE_ROOT" \
  'production input policy must consume RemoteAppTargetBinding'
reject 'pointer_target_for_entry\s*\(' "$REMOTE_ROOT" \
  'production pointer targeting must consume RemoteAppTargetBinding plus tracker snapshot'

require 'ResourceEntryTargetResolver\.resolve_for_session\(' \
  "$REMOTE_ROOT/session_creation.rs" \
  'session creation workflow must be the ResourceEntry-to-target_binding boundary'
require 'verify_target_binding_for_session\(' \
  "$REMOTE_ROOT/session_creation.rs" \
  'session creation workflow must verify the resolved target binding before session insertion'
require 'RemoteDesktopSessionCreationWorkflow::start' \
  "$REMOTE_ROOT/handlers/create_session.rs" \
  'create_session handler must delegate pre-row lifecycle to RemoteDesktopSessionCreationWorkflow'
require 'session\.target_binding\(\)\.clone\(\)' \
  "$REMOTE_ROOT/transport/webrtc_negotiation.rs" \
  'WebRTC negotiation must consume the session-owned target binding'
require 'target_for_binding\(' \
  "$REMOTE_ROOT/transport/webrtc_native_media.rs" \
  'native media must start from RemoteAppTargetBinding'
require 'trait RemoteAppMediaSourceFactory' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC media source selection must use the RemoteAppMediaSourceFactory boundary'
require 'fn start_from_binding\(' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC media source selection must start from RemoteAppTargetBinding'
require 'if binding\.target_kind\(\) == RemoteDesktopTargetKind::Display' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC display baseline plan must be guarded as display-only'
require 'RemoteAppMediaSource::DisplayBaseline' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC display baseline must be an explicit media-source selection'
require 'TargetResolutionError::DisplayFallbackForbidden' \
  "$REMOTE_ROOT/transport/media_source.rs" \
  'direct WebRTC baseline guard must fail app/window sessions with typed display_fallback_forbidden reason'
require 'input_policy_for_binding\(' \
  "$REMOTE_ROOT/transport/webrtc_negotiation.rs" \
  'WebRTC input policy must consume RemoteAppTargetBinding'
require 'target_binding\(\)' \
  "$REMOTE_ROOT/handlers/attach.rs" \
  'diagnostic attach must consume the session-owned target binding'

while IFS=: read -r file line _match; do
  case "${file#"$ROOT/"}" in
    plugins/remote-desktop/src/target.rs|\
    plugins/remote-desktop/src/handlers/create_session.rs|\
    plugins/remote-desktop/src/session_creation.rs|\
    plugins/remote-desktop/src/test_support.rs)
      continue
      ;;
  esac
  if awk -v line="$line" '
      BEGIN { found = 0 }
      NR >= line - 8 && NR < line && /#\[cfg\(test\)\]/ { found = 1 }
      END { exit(found ? 0 : 1) }
    ' "$file"; then
    continue
  fi
  if is_test_context "$file" "$line"; then
    continue
  fi
  fail "${file#"$ROOT/"}:$line uses ResourceEntryTargetResolver outside the creation/test boundary"
done < <(rg -n -- 'ResourceEntryTargetResolver' "$REMOTE_ROOT" || true)

while IFS=: read -r file line _match; do
  if awk -v line="$line" '
      BEGIN { found = 0 }
      NR >= line - 3 && NR < line && /#\[cfg\(test\)\]/ { found = 1 }
      END { exit(found ? 0 : 1) }
    ' "$file"; then
    continue
  fi
  fail "${file#"$ROOT/"}:$line declares entry-based backend selection outside #[cfg(test)]"
done < <(rg -n -- 'fn (production_backend_for_entry|webrtc_transport_backend_for_entry|select_builtin_h264_backend)\s*\(' "$REMOTE_ROOT" || true)

printf 'check-remoteapp-target-binding-boundary: ok\n'

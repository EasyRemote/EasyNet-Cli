#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

fail() {
  printf 'check-remoteapp-picker-subject-boundary: %s\n' "$1" >&2
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

CLI_ABILITY="$ROOT/src/cli/commands/groups/ability.rs"
LOCAL_INVOKE="$ROOT/src/support/platform/local_invoke.rs"
CREATE_SESSION="$ROOT/plugins/remote-desktop/src/handlers/create_session.rs"
SCHEMA="$ROOT/plugins/remote-desktop/src/schema.rs"
RESOURCE_SUBJECT="$ROOT/plugins/remote-desktop/src/resource.rs"

for file in "$CLI_ABILITY" "$LOCAL_INVOKE" "$CREATE_SESSION" "$SCHEMA" "$RESOURCE_SUBJECT"; do
  [[ -f "$file" ]] || fail "missing required source ${file#"$ROOT/"}"
done

require 'LocalRemoteTargetInventoryIssuer' "$LOCAL_INVOKE" \
  'remote target pickers need a named live inventory issuer'
require 'RESOURCE_REFRESH_REMOTE_TARGETS' "$LOCAL_INVOKE" \
  'live remote target inventory issuer must call resource.refresh_remote_targets'
require 'Target pickers that need live display/window/application rows must invoke' "$LOCAL_INVOKE" \
  'live picker contract must be documented at the issuer boundary'

require 'AbilityAction::RefreshRemoteTargets' "$CLI_ABILITY" \
  'CLI must expose a live remote target refresh action for picker/debug flows'
require 'LocalRemoteTargetInventoryIssuer::refresh_remote_targets' "$CLI_ABILITY" \
  'CLI refresh action must use the live inventory issuer, not meta.list_resources'
require 'resource_ura' "$CLI_ABILITY" \
  'CLI refresh output must surface selectable resource_ura subjects'

require 'resolve_screen_resource_from_envelope\(ABILITY_CREATE_SESSION, &env, &args\)' "$CREATE_SESSION" \
  'create_session must resolve the selected resource from Invocation.subject'
require 'create_session_rejects_subject_in_args' "$CREATE_SESSION" \
  'create_session must have a regression test rejecting args.subject'
require 'subject_in_args' "$CREATE_SESSION" \
  'create_session must fail closed when callers put subject in args'
require 'Subject MUST be the resource_ura in the invocation envelope' "$SCHEMA" \
  'create_session public contract must say subject is the envelope resource_ura'
require 'additionalProperties": false' "$SCHEMA" \
  'create_session schema must reject undeclared subject fields'

schema_body="$(awk '
  /pub fn create_session_input_schema\(\)/ { in_fn = 1 }
  in_fn { print }
  in_fn && /^}/ { exit }
' "$SCHEMA")"
if grep -q '"subject"' <<<"$schema_body"; then
  fail 'create_session input schema must not declare args.subject'
fi
if grep -q '"resource_ura"' <<<"$schema_body"; then
  fail 'create_session input schema must not accept resource_ura in args; use Invocation.subject'
fi

require 'resolve_required_resource_subject' "$RESOURCE_SUBJECT" \
  'remote desktop resource boundary must delegate subject validation to resource_subject'
require 'ResourceType::Application' "$RESOURCE_SUBJECT" \
  'remote desktop resource boundary must admit application targets'
require 'ResourceType::Window' "$RESOURCE_SUBJECT" \
  'remote desktop resource boundary must admit window targets'

# A live picker may render cached rows after it has invoked refresh/watch, but
# code that names the remote-target picker path must not use meta.list_resources
# as the live discovery primitive.
scan_roots=()
for candidate in "$ROOT/src" "$ROOT/plugins"; do
  [[ -d "$candidate" ]] && scan_roots+=("$candidate")
done

while IFS=: read -r file line text; do
  relative="${file#"$ROOT/"}"
  case "$relative" in
    src/cli/commands/groups/ability.rs|\
    src/support/platform/local_invoke.rs|\
    src/daemon/ability/builtins/resources/list.rs|\
    src/daemon/ability/builtins/resources/refresh_remote_targets.rs|\
    src/daemon/ability/builtins/resources/watch_remote_targets.rs|\
    src/daemon/ability/names/resources.rs|\
    src/daemon/ability/catalog/*|\
    tests/*|\
    tools/scripts/check-remoteapp-picker-subject-boundary.sh)
      continue
      ;;
  esac
  haystack="$relative $text"
  if grep -qiE 'remote.?target|remote.?desktop|display.*window|application.*window|picker' <<<"$haystack"; then
    fail "$relative:$line uses meta.list_resources language in a remote target picker path; use resource.refresh_remote_targets/watch_remote_targets"
  fi
done < <(if ((${#scan_roots[@]} > 0)); then rg -n -- 'meta\.list_resources|META_LIST_RESOURCES' "${scan_roots[@]}"; fi 2>/dev/null || true)

printf 'check-remoteapp-picker-subject-boundary: ok\n'

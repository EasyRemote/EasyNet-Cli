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

require_multiline() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  perl -0ne "exit(($pattern) ? 0 : 1)" "$path" || fail "$message"
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
SESSION_CREATION="$ROOT/plugins/remote-desktop/src/session_creation.rs"
SCHEMA="$ROOT/plugins/remote-desktop/src/schema.rs"
RESOURCE_SUBJECT="$ROOT/plugins/remote-desktop/src/resource.rs"
RESOURCE_PROJECTION="$ROOT/src/daemon/resources/projection.rs"
RESOURCE_STORE="$ROOT/src/daemon/persistence/resources.rs"
RESOURCE_BOOTSTRAP="$ROOT/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs"
WATCH_TARGETS="$ROOT/src/daemon/ability/builtins/resources/watch_remote_targets.rs"
META_LIST_RESOURCES="$ROOT/src/daemon/ability/builtins/resources/list.rs"

for file in "$CLI_ABILITY" "$LOCAL_INVOKE" "$CREATE_SESSION" "$SESSION_CREATION" "$SCHEMA" "$RESOURCE_SUBJECT" "$RESOURCE_PROJECTION" "$RESOURCE_STORE" "$RESOURCE_BOOTSTRAP" "$WATCH_TARGETS" "$META_LIST_RESOURCES"; do
  [[ -f "$file" ]] || fail "missing required source ${file#"$ROOT/"}"
done

require 'LocalRemoteTargetInventoryIssuer' "$LOCAL_INVOKE" \
  'remote target pickers need a named live inventory issuer'
require 'RESOURCE_REFRESH_REMOTE_TARGETS' "$LOCAL_INVOKE" \
  'live remote target inventory issuer must call resource.refresh_remote_targets'
require 'RESOURCE_WATCH_REMOTE_TARGETS' "$LOCAL_INVOKE" \
  'live remote target inventory issuer must call resource.watch_remote_targets for streaming pickers'
require 'watch_remote_targets' "$LOCAL_INVOKE" \
  'live remote target inventory issuer must expose a watch_remote_targets stream facade'
require 'Target pickers that need live display/window/application rows must invoke' "$LOCAL_INVOKE" \
  'live picker contract must be documented at the issuer boundary'
require 'LocalRemoteDesktopSessionIssuer' "$LOCAL_INVOKE" \
  'remote desktop session creation must have a named selected-resource session issuer'
require 'remote_desktop\.grant_consent' "$LOCAL_INVOKE" \
  'selected-resource session issuer must grant consent before creating a session'
require 'remote_desktop\.create_session' "$LOCAL_INVOKE" \
  'selected-resource session issuer must create the session after consent'
require 'causal_parent' "$LOCAL_INVOKE" \
  'selected-resource session issuer must chain create_session to a verified consent receipt'
require 'create_session_args_with_consent_ticket' "$LOCAL_INVOKE" \
  'selected-resource session issuer must inject daemon-issued consent ticket instead of accepting caller-supplied subject args'

require 'AbilityAction::RefreshRemoteTargets' "$CLI_ABILITY" \
  'CLI must expose a live remote target refresh action for picker/debug flows'
require 'LocalRemoteTargetInventoryIssuer::refresh_remote_targets' "$CLI_ABILITY" \
  'CLI refresh action must use the live inventory issuer, not meta.list_resources'
require 'AbilityAction::WatchRemoteTargets' "$CLI_ABILITY" \
  'CLI must expose a live remote target watch action for picker/debug flows'
require 'LocalRemoteTargetInventoryIssuer::watch_remote_targets' "$CLI_ABILITY" \
  'CLI watch action must use the live inventory issuer, not meta.list_resources'
require 'AbilityAction::CreateRemoteDesktopSession' "$CLI_ABILITY" \
  'CLI/frontend adapter must expose selected-resource remote desktop session creation'
require 'LocalRemoteDesktopSessionIssuer::create_session' "$CLI_ABILITY" \
  'CLI/frontend adapter must use the selected-resource session issuer'
require 'create_remote_desktop_session_request' "$CLI_ABILITY" \
  'CLI/frontend adapter must build create_session args without copying selected subject into args'
require 'resource_ura' "$CLI_ABILITY" \
  'CLI refresh output must surface selectable resource_ura subjects'

require 'validate_remote_target_freshness' "$RESOURCE_PROJECTION" \
  'remote target picker projection must fail closed without freshness metadata'
require 'cache_projection' "$RESOURCE_PROJECTION" \
  'meta.list_resources remote target cache rows must expose a cache-only projection marker'
require 'cached_requires_live_refresh' "$RESOURCE_PROJECTION" \
  'meta.list_resources remote target cache rows must require live refresh before picker selection'
require 'live_refresh_required' "$RESOURCE_PROJECTION" \
  'meta.list_resources remote target cache rows must be machine-readable as live-refresh-required'
require 'resource\.refresh_remote_targets' "$RESOURCE_PROJECTION" \
  'cache-only remote target rows must point clients at resource.refresh_remote_targets'
require 'resource\.watch_remote_targets' "$RESOURCE_PROJECTION" \
  'cache-only remote target rows must point clients at resource.watch_remote_targets'
require 'cache projections; live target pickers must use' "$META_LIST_RESOURCES" \
  'meta.list_resources description must not present cached remote targets as live picker rows'
require 'owner_agent: "easynet:///r/acme/agent/device\.dev-1\.media"' "$RESOURCE_PROJECTION" \
  'remote target picker projection fixtures must use a device-sponsored SystemAgent owner_agent'
reject '"owner_agent"[[:space:]]*:[[:space:]]*"easynet:///r/[^"]*/device/' "$RESOURCE_PROJECTION" \
  'remote target picker projection must not model owner_agent as a Device URA'
require '"freshness"' "$RESOURCE_BOOTSTRAP" \
  'live target refresh must annotate picker rows with metadata.freshness'
require 'stale_after_ms' "$RESOURCE_BOOTSTRAP" \
  'live target freshness must expose stale_after_ms for picker staleness decisions'
require 'map\.remove\("freshness"\)' "$RESOURCE_BOOTSTRAP" \
  'remote target cache signature must ignore freshness-only metadata'
require 'map\.remove\("freshness"\)' "$WATCH_TARGETS" \
  'remote target watch signatures must ignore freshness-only metadata'
require 'trait RemoteTargetInventorySource' "$WATCH_TARGETS" \
  'remote target watch must use an injectable inventory source instead of binding tests to host-local discovery'
require 'DaemonRemoteTargetInventorySource' "$WATCH_TARGETS" \
  'remote target watch production source must be explicitly named'
require 'handler_with_source' "$WATCH_TARGETS" \
  'remote target watch handler must be testable with injected inventory source'
require 'run_watch_loop' "$WATCH_TARGETS" \
  'remote target watch stream loop must be an explicit deterministic lifecycle worker'
require 'watch_handler_emits_snapshot_delta_and_stops_at_max_events' "$WATCH_TARGETS" \
  'remote target watch must test snapshot/delta emission and max_events terminal closure'
require 'watch_handler_returns_source_error_as_terminal_stream_error' "$WATCH_TARGETS" \
  'remote target watch must test source errors as deterministic terminal stream errors'
require_multiline 'm/inventory_hash\(\s*response\.screen_target_discovery_available,\s*&signatures\s*\)/s' "$WATCH_TARGETS" \
  'remote target watch identity must include discovery availability instead of coalescing outages'
require 'unavailable_inventory_delta_does_not_report_targets_removed' "$WATCH_TARGETS" \
  'temporary discovery outages must not be projected as definitive target removals'
require 'discovery_availability_participates_in_inventory_hash' "$WATCH_TARGETS" \
  'availability-only inventory transitions must have regression coverage'
require 'upsert_resources_indexed' "$RESOURCE_STORE" \
  'resource persistence must expose indexed batch upsert for live target refresh'
require 'upsert_resources_indexed' "$RESOURCE_BOOTSTRAP" \
  'live remote target refresh must use indexed batch upsert, not per-target linear upsert'
reject 'for resource in live_targets' "$RESOURCE_BOOTSTRAP" \
  'live remote target refresh must not call linear upsert once per discovered target'

require 'RemoteDesktopSessionCreationWorkflow::start' "$CREATE_SESSION" \
  'create_session handler must delegate subject validation to RemoteDesktopSessionCreationWorkflow'
require 'resolve_screen_resource_from_envelope\(ABILITY_CREATE_SESSION, env, args\)' "$SESSION_CREATION" \
  'create_session workflow must resolve the selected resource from Invocation.subject'
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

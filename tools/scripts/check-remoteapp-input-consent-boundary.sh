#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_INPUT_CONSENT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
REMOTE_ROOT="$ROOT/plugins/remote-desktop"

fail() {
  printf 'check-remoteapp-input-consent-boundary: %s\n' "$1" >&2
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

CONSENT_REGISTRY="$REMOTE_ROOT/src/consent_registry.rs"
GRANT_HANDLER="$REMOTE_ROOT/src/handlers/grant_consent.rs"
GRANT_DESCRIPTOR="$REMOTE_ROOT/abilities/remote_desktop.grant_consent.ability.toml"
SESSION_CONSENT="$REMOTE_ROOT/src/session_consent.rs"
SESSION_CREATION="$REMOTE_ROOT/src/session_creation.rs"
TARGET="$REMOTE_ROOT/src/target.rs"
VIEW="$REMOTE_ROOT/src/view.rs"
CREATE_HANDLER="$REMOTE_ROOT/src/handlers/create_session.rs"
HANDLERS_MOD="$REMOTE_ROOT/src/handlers/mod.rs"

for file in \
  "$CONSENT_REGISTRY" \
  "$GRANT_HANDLER" \
  "$GRANT_DESCRIPTOR" \
  "$SESSION_CONSENT" \
  "$SESSION_CREATION" \
  "$TARGET" \
  "$VIEW" \
  "$CREATE_HANDLER" \
  "$HANDLERS_MOD"; do
  [[ -f "$file" ]] || fail "missing ${file#"$ROOT/"}"
done

require 'input_control_granted: bool' "$CONSENT_REGISTRY" \
  'consent authorization and pending tickets must carry explicit input-control scope'
require 'fn issue_with_grants' "$CONSENT_REGISTRY" \
  'consent registry must mint scoped tickets instead of overloading media consent'
require 'input_control_granted,' "$CONSENT_REGISTRY" \
  'consent consume must preserve the explicit input-control grant'
require 'consent_ticket_preserves_explicit_input_control_grant' "$CONSENT_REGISTRY" \
  'consent registry tests must prove input-control grant preservation'

require '\[input_schema\.properties\.input_control\]' "$GRANT_DESCRIPTOR" \
  'grant_consent descriptor must expose optional input_control'
require_multiline '/input_control = optional_bool\(&args, "input_control", ABILITY_GRANT_CONSENT\)\?[\s\S]*allow_remote_focus = optional_bool\(&args, "allow_remote_focus", ABILITY_GRANT_CONSENT\)\?[\s\S]*issue_with_grants\(\s*env\.caller\(\),\s*&entry\.resource_ura,\s*intent,\s*input_control,\s*allow_remote_focus,\s*\)/s' "$GRANT_HANDLER" \
  'grant_consent handler must bind input_control into the minted ticket'
require '"input_control": input_control' "$GRANT_HANDLER" \
  'grant_consent response must project the granted input_control scope'
require 'allow_remote_focus && !input_control' "$GRANT_HANDLER" \
  'grant_consent must reject remote-focus scope without input-control consent'
require '"remote_focus": allow_remote_focus' "$GRANT_HANDLER" \
  'grant_consent response must project the granted remote-focus scope'
require 'grant_consent_projects_explicit_input_control_scope' "$HANDLERS_MOD" \
  'handler tests must cover explicit input-control consent projection'

require 'input_control_granted: bool' "$SESSION_CONSENT" \
  'session consent grant must persist input-control scope'
require 'permits_input_control' "$SESSION_CONSENT" \
  'session creation must consume input-control scope through a domain method'
require '"input_control": self\.input_control_granted' "$SESSION_CONSENT" \
  'session consent view must audit the input-control grant scope'

require 'permits_input_control' "$SESSION_CREATION" \
  'create_session workflow must read input-control grant from consumed consent'
require 'resolve_for_session_with_input_consent' "$SESSION_CREATION" \
  'create_session workflow must pass input-control scope into target binding resolution'
require 'with_input_control_consent_ticket' "$CREATE_HANDLER" \
  'create_session tests must mint an explicit input-control consent ticket'
require 'create_session_uses_explicit_input_control_consent_for_display_interactive_scope' "$CREATE_HANDLER" \
  'create_session tests must cover display interactive scope with input-control consent'

require 'InputControlGranted' "$TARGET" \
  'target binding must distinguish input-control grant from media-only consent'
require 'input_control_granted' "$TARGET" \
  'target binding resolution must receive explicit input-control grant state'
require 'InputScope::DisplayGlobal' "$TARGET" \
  'display targets with explicit input-control consent must be able to resolve display-global input scope'
require 'TargetScopedInputUnsafe' "$TARGET" \
  'window/application targets must remain fail-closed until target-scoped dispatch is safe'
require 'display_interactive_with_input_consent_projects_display_global_scope' "$TARGET" \
  'target tests must prove input-control consent opens only display-global scope'
require 'display_interactive_downgrades_until_input_consent_exists' "$TARGET" \
  'target tests must prove media-only consent remains view-only'

require 'effective_mode": if interactive_ready \{ "interactive" \} else \{ "view_only" \}' "$VIEW" \
  'session view must not report interactive effective mode until runtime input is truly ready'
require 'session\.target_snapshot\(\)\.input_blocked_reason\(\)' "$VIEW" \
  'session view input_readiness must consume the target tracker typed blocker state'
require_multiline '/else if let Some\(reason\) = session\.target_snapshot\(\)\.input_blocked_reason\(\) \{\s*json!\(reason\)/s' "$VIEW" \
  'session view must project the exact target tracker blocker instead of a generic input state'
require 'input_injection_unavailable' "$VIEW" \
  'session view must expose OS input-permission blockage separately from consent scope'
require 'session_view_blocks_input_readiness_when_target_tracking_disables_input' "$VIEW" \
  'session view tests must prove target-tracker input loss blocks interactive readiness'

printf 'check-remoteapp-input-consent-boundary: ok\n'

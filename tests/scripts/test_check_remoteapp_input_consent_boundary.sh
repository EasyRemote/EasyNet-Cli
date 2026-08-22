#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-input-consent-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

REMOTE_ROOT="$SANDBOX/plugins/remote-desktop"
mkdir -p \
  "$REMOTE_ROOT/src/handlers" \
  "$REMOTE_ROOT/abilities"

cat >"$REMOTE_ROOT/src/consent_registry.rs" <<'RS'
pub struct RemoteDesktopConsentAuthorization {
    input_control_granted: bool,
}

struct PendingConsent {
    input_control_granted: bool,
}

impl RemoteDesktopConsentRegistry {
    fn issue_with_grants(input_control_granted: bool) {}
    fn consume() -> RemoteDesktopConsentAuthorization {
        RemoteDesktopConsentAuthorization {
            input_control_granted,
        }
    }
}

fn consent_ticket_preserves_explicit_input_control_grant() {}
RS

cat >"$REMOTE_ROOT/src/handlers/grant_consent.rs" <<'RS'
fn handle(args: Value) {
    let input_control = optional_bool(&args, "input_control", ABILITY_GRANT_CONSENT)?;
    plugin.consent_registry().issue_with_grants(env.caller(), &entry.resource_ura, intent, input_control,)?;
    json!({
        "grant_scope": {
            "input_control": input_control,
        }
    });
}
RS

cat >"$REMOTE_ROOT/abilities/remote_desktop.grant_consent.ability.toml" <<'TOML'
[input_schema.properties.input_control]
type = "boolean"
TOML

cat >"$REMOTE_ROOT/src/session_consent.rs" <<'RS'
struct RemoteDesktopConsentGrant {
    input_control_granted: bool,
}

impl RemoteDesktopConsentGrant {
    fn permits_input_control(&self) -> bool {
        self.input_control_granted
    }
    fn to_value(&self) {
        json!({
            "grant_scope": {
                "input_control": self.input_control_granted,
            }
        });
    }
}
RS

cat >"$REMOTE_ROOT/src/session_creation.rs" <<'RS'
fn resolve_target_with_verifier() {
    let input_control_granted = self.consent.as_ref().is_some_and(RemoteDesktopConsentGrant::permits_input_control);
    ResourceEntryTargetResolver.resolve_for_session_with_input_consent(
        ABILITY_CREATE_SESSION,
        &self.entry,
        &self.mode,
        1,
        input_control_granted,
    );
}
RS

cat >"$REMOTE_ROOT/src/target.rs" <<'RS'
enum InputScopeReason {
    InputControlGranted,
    InputConsentRequired,
    TargetScopedInputUnsafe,
}

impl ResourceEntryTargetResolver {
    fn resolve_for_session_with_input_consent(input_control_granted: bool) {
        if input_control_granted {
            InputScope::DisplayGlobal;
        }
    }
}

fn display_interactive_with_input_consent_projects_display_global_scope() {}
fn display_interactive_downgrades_until_input_consent_exists() {}
RS

cat >"$REMOTE_ROOT/src/view.rs" <<'RS'
fn session_view_blocks_input_readiness_when_target_tracking_disables_input() {}

fn input_readiness_view() {
    let blocked_reason = if !session.target_snapshot().input_enabled() {
        json!("target_input_not_ready")
    } else {
        json!("input_injection_unavailable")
    };
    json!({
        "effective_mode": if interactive_ready { "interactive" } else { "view_only" },
    });
}
RS

cat >"$REMOTE_ROOT/src/handlers/create_session.rs" <<'RS'
fn create_session_uses_explicit_input_control_consent_for_display_interactive_scope() {
    with_input_control_consent_ticket();
}
RS

cat >"$REMOTE_ROOT/src/handlers/mod.rs" <<'RS'
fn grant_consent_projects_explicit_input_control_scope() {}
RS

CHECK_REMOTEAPP_INPUT_CONSENT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/\[input_schema\.properties\.input_control\]/[input_schema.properties.input_ignored]/' \
  "$REMOTE_ROOT/abilities/remote_desktop.grant_consent.ability.toml"
if CHECK_REMOTEAPP_INPUT_CONSENT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp input consent checker accepted missing descriptor input_control" >&2
  exit 1
fi
perl -0pi -e 's/\[input_schema\.properties\.input_ignored\]/[input_schema.properties.input_control]/' \
  "$REMOTE_ROOT/abilities/remote_desktop.grant_consent.ability.toml"

perl -0pi -e 's/issue_with_grants\(env\.caller\(\), &entry\.resource_ura, intent, input_control,\)/issue_with_grants(env.caller(), &entry.resource_ura, intent, false,)/' \
  "$REMOTE_ROOT/src/handlers/grant_consent.rs"
if CHECK_REMOTEAPP_INPUT_CONSENT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp input consent checker accepted grant_consent that drops input_control" >&2
  exit 1
fi
perl -0pi -e 's/issue_with_grants\(env\.caller\(\), &entry\.resource_ura, intent, false,\)/issue_with_grants(env.caller(), &entry.resource_ura, intent, input_control,)/' \
  "$REMOTE_ROOT/src/handlers/grant_consent.rs"

perl -0pi -e 's/InputScope::DisplayGlobal/InputScope::ViewOnly/' \
  "$REMOTE_ROOT/src/target.rs"
if CHECK_REMOTEAPP_INPUT_CONSENT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp input consent checker accepted display input consent that stays view-only" >&2
  exit 1
fi
perl -0pi -e 's/InputScope::ViewOnly/InputScope::DisplayGlobal/' \
  "$REMOTE_ROOT/src/target.rs"

perl -0pi -e 's/"effective_mode": if interactive_ready \{ "interactive" \} else \{ "view_only" \}/"effective_mode": "interactive"/' \
  "$REMOTE_ROOT/src/view.rs"
if CHECK_REMOTEAPP_INPUT_CONSENT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp input consent checker accepted unconditional interactive effective mode" >&2
  exit 1
fi
perl -0pi -e 's/    let blocked_reason = if !session\.target_snapshot\(\)\.input_enabled\(\) \{\n        json!\("target_input_not_ready"\)\n    \} else \{\n        json!\("input_injection_unavailable"\)\n    \};/    let blocked_reason = json!("input_injection_unavailable");/s' \
  "$REMOTE_ROOT/src/view.rs"
if CHECK_REMOTEAPP_INPUT_CONSENT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp input consent checker accepted input readiness that ignores target tracker input_enabled" >&2
  exit 1
fi

printf 'test_check_remoteapp_input_consent_boundary.sh: all cases passed\n'

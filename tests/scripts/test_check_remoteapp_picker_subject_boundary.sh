#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-picker-subject-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p \
  "$SANDBOX/src/cli/commands/groups" \
  "$SANDBOX/src/support/platform" \
  "$SANDBOX/plugins/remote-desktop/src/handlers" \
  "$SANDBOX/plugins/remote-desktop/src"

cat >"$SANDBOX/src/support/platform/local_invoke.rs" <<'RS'
pub struct LocalRemoteTargetInventoryIssuer;
impl LocalRemoteTargetInventoryIssuer {
    pub fn refresh_remote_targets(args: Value) -> anyhow::Result<Value> {
        LocalDaemonSystemAbilityIssuer::invoke(
            crate::daemon::ability::names::resources::RESOURCE_REFRESH_REMOTE_TARGETS,
            args,
        )
    }
    pub fn watch_remote_targets(args: Value) -> anyhow::Result<Vec<LocalStreamFrame>> {
        LocalDaemonSystemAbilityIssuer::stream(
            crate::daemon::ability::names::resources::RESOURCE_WATCH_REMOTE_TARGETS,
            args,
        )
    }
}
/// Target pickers that need live display/window/application rows must invoke
/// resource.refresh_remote_targets or resource.watch_remote_targets through
/// this issuer before presenting rows.
RS

cat >"$SANDBOX/src/cli/commands/groups/ability.rs" <<'RS'
fn run(action: AbilityAction) {
    match action {
        AbilityAction::RefreshRemoteTargets(args) => {
            let response = LocalRemoteTargetInventoryIssuer::refresh_remote_targets(args).unwrap();
            println!("{}", response["resources"][0]["resource_ura"]);
        }
        AbilityAction::WatchRemoteTargets(args) => {
            let frames = LocalRemoteTargetInventoryIssuer::watch_remote_targets(args).unwrap();
            println!("{}", frames.len());
        }
    }
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs" <<'RS'
fn handle(env: EnvelopeContext, args: Value) {
    let entry = resolve_screen_resource_from_envelope(ABILITY_CREATE_SESSION, &env, &args).unwrap();
}

#[test]
fn create_session_rejects_subject_in_args() {
    assert!("subject_in_args".contains("subject_in_args"));
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/schema.rs" <<'RS'
pub fn create_session_description() -> &'static str {
    "Subject MUST be the resource_ura in the invocation envelope"
}

pub fn create_session_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["consent_ticket"],
        "properties": {
            "consent_ticket": { "type": "string" }
        }
    })
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/resource.rs" <<'RS'
fn screen_resource_subject_spec() {
    resolve_required_resource_subject();
    ResourceType::Application;
    ResourceType::Window;
}
RS

CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

cp "$SANDBOX/src/support/platform/local_invoke.rs" "$SANDBOX/src/support/platform/local_invoke.rs.good"
perl -0pi -e 's/\n    pub fn watch_remote_targets\(args: Value\) -> anyhow::Result<Vec<LocalStreamFrame>> \{\n        LocalDaemonSystemAbilityIssuer::stream\(\n            crate::daemon::ability::names::resources::RESOURCE_WATCH_REMOTE_TARGETS,\n            args,\n        \)\n    \}//' \
  "$SANDBOX/src/support/platform/local_invoke.rs"
if CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp picker subject checker accepted missing watch_remote_targets issuer" >&2
  exit 1
fi
mv "$SANDBOX/src/support/platform/local_invoke.rs.good" "$SANDBOX/src/support/platform/local_invoke.rs"

perl -0pi -e 's/"consent_ticket": \{ "type": "string" \}/"consent_ticket": { "type": "string" }, "subject": { "type": "string" }/' \
  "$SANDBOX/plugins/remote-desktop/src/schema.rs"
if CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp picker subject checker accepted args.subject" >&2
  exit 1
fi

perl -0pi -e 's/, "subject": \{ "type": "string" \}//' \
  "$SANDBOX/plugins/remote-desktop/src/schema.rs"
mkdir -p "$SANDBOX/src/frontend"
cat >"$SANDBOX/src/frontend/remote_target_picker.ts" <<'TS'
export function loadRemoteDesktopPicker() {
  return invoke("meta.list_resources", { types: ["window", "application"] });
}
TS
if CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp picker subject checker accepted meta.list_resources as live picker" >&2
  exit 1
fi

echo "test_check_remoteapp_picker_subject_boundary.sh: all cases passed"

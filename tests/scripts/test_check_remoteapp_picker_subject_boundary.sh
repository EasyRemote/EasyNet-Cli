#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-picker-subject-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p \
  "$SANDBOX/src/cli/commands/groups" \
  "$SANDBOX/src/daemon/ability/builtins/resources/media" \
  "$SANDBOX/src/daemon/ability/builtins/resources" \
  "$SANDBOX/src/daemon/persistence" \
  "$SANDBOX/src/daemon/resources" \
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
pub struct LocalRemoteDesktopSessionIssuer;
impl LocalRemoteDesktopSessionIssuer {
    pub fn create_session(resource_ura: &str, args: Value) -> anyhow::Result<(Value, VerifiedLocalInvocationMeta)> {
        let grant = LocalDaemonSystemAbilityIssuer::invoke("remote_desktop.grant_consent", json!({ "intent": "remote_desktop_session" }))?;
        let parent = VerifiedLocalInvocationMeta(grant).causal_parent()?;
        let args = create_session_args_with_consent_ticket(args, "ticket")?;
        LocalDaemonSystemAbilityIssuer::invoke_with_parent("remote_desktop.create_session", resource_ura, args, parent)
    }
}
impl VerifiedLocalInvocationMeta {
    pub fn causal_parent(&self) -> anyhow::Result<Value> {
        Ok(json!({ "receipt_ura": "r", "receipt_hash": "h" }))
    }
}
fn create_session_args_with_consent_ticket(args: Value, ticket: &str) -> anyhow::Result<Value> {
    Ok(args)
}
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
        AbilityAction::CreateRemoteDesktopSession(args) => {
            let request = create_remote_desktop_session_request(&args);
            let response = LocalRemoteDesktopSessionIssuer::create_session(&args.subject, request).unwrap();
            println!("{}", response.0);
        }
    }
}
fn create_remote_desktop_session_request(args: &CreateRemoteDesktopSessionArgs) -> Value {
    json!({ "mode": args.mode })
}
RS

cat >"$SANDBOX/src/daemon/resources/projection.rs" <<'RS'
fn validate_remote_target_freshness(entry: ResourceEntry) {
    entry.metadata["freshness"]["observed_at_ms"];
    entry.metadata["freshness"]["stale_after_ms"];
    entry.metadata["freshness"]["source"];
}

fn cache_projection_for_remote_target() {
    json!({
        "cache_projection": {
            "selection_state": "cached_requires_live_refresh",
            "live_refresh_required": true,
            "refresh_ability": "resource.refresh_remote_targets",
            "watch_ability": "resource.watch_remote_targets",
        }
    });
}

fn resource_entry() -> ResourceEntry {
    ResourceEntry {
        owner_agent: "easynet:///r/acme/agent/device.dev-1.media".to_string(),
    }
}
RS

cat >"$SANDBOX/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs" <<'RS'
fn apply_remote_target_refresh(resources: Vec<DiscoveredResource>) {
    upsert_resources_indexed(resources);
}

fn annotate_live_remote_target(metadata: &mut Map) {
    metadata.insert("freshness", json!({
        "observed_at_ms": 10,
        "stale_after_ms": 5010,
        "source": "live_refresh",
    }));
}

fn stable_remote_target_entry_signature(map: &mut Map) {
    map.remove("freshness");
}
RS

cat >"$SANDBOX/src/daemon/persistence/resources.rs" <<'RS'
fn upsert_resources_indexed(resources: Vec<ResourceUpsert>) {
    let mut index = HashMap::new();
}
RS

cat >"$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs" <<'RS'
trait RemoteTargetInventorySource {}
struct DaemonRemoteTargetInventorySource;

fn handler_with_source() {
    run_watch_loop();
}

fn run_watch_loop() {}

fn stable_resource_signature(map: &mut Map) {
    map.remove("freshness");
}

fn snapshot(response: Response) {
    inventory_hash(response.screen_target_discovery_available, &signatures);
}

#[test]
fn watch_handler_emits_snapshot_delta_and_stops_at_max_events() {}

#[test]
fn watch_handler_returns_source_error_as_terminal_stream_error() {}

#[test]
fn unavailable_inventory_delta_does_not_report_targets_removed() {}

#[test]
fn discovery_availability_participates_in_inventory_hash() {}
RS

cat >"$SANDBOX/src/daemon/ability/builtins/resources/list.rs" <<'RS'
fn description() -> &'static str {
    "Display/window/application rows are cache projections; live target pickers must use resource.refresh_remote_targets or resource.watch_remote_targets."
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs" <<'RS'
fn handle(env: EnvelopeContext, args: Value) {
    let workflow = RemoteDesktopSessionCreationWorkflow::start(&env, &args).unwrap();
}

#[test]
fn create_session_rejects_subject_in_args() {
    assert!("subject_in_args".contains("subject_in_args"));
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/session_creation.rs" <<'RS'
impl RemoteDesktopSessionCreationWorkflow {
    fn start(env: &EnvelopeContext, args: &Value) {
        let entry = resolve_screen_resource_from_envelope(ABILITY_CREATE_SESSION, env, args).unwrap();
    }
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

cp "$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs" \
  "$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs.good"
perl -0pi -e 's/response\.screen_target_discovery_available/true/' \
  "$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs"
if CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp picker subject checker accepted inventory hash without discovery availability" >&2
  exit 1
fi
mv "$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs.good" \
  "$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs"

cp "$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs" \
  "$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs.good"
perl -0pi -e 's/discovery_availability_participates_in_inventory_hash/discovery_availability_is_not_observable/' \
  "$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs"
if CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp picker subject checker accepted missing discovery availability regression" >&2
  exit 1
fi
mv "$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs.good" \
  "$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs"

cp "$SANDBOX/src/daemon/resources/projection.rs" "$SANDBOX/src/daemon/resources/projection.rs.good"
cat >"$SANDBOX/src/daemon/resources/projection.rs" <<'RS'
fn validate_remote_target_freshness(entry: ResourceEntry) {
    entry.metadata["freshness"]["observed_at_ms"];
    entry.metadata["freshness"]["stale_after_ms"];
    entry.metadata["freshness"]["source"];
}

fn resource_entry() -> ResourceEntry {
    ResourceEntry {
        owner_agent: "easynet:///r/acme/agent/device.dev-1.media".to_string(),
    }
}
RS
if CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp picker subject checker accepted missing cache-only projection marker" >&2
  exit 1
fi
mv "$SANDBOX/src/daemon/resources/projection.rs.good" "$SANDBOX/src/daemon/resources/projection.rs"

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

rm "$SANDBOX/src/daemon/resources/projection.rs"
cat >"$SANDBOX/src/daemon/resources/projection.rs" <<'RS'
fn remote_target_projection_without_freshness(entry: ResourceEntry) {
    entry.metadata["observed_at_ms"];
}
RS
if CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp picker subject checker accepted missing freshness projection contract" >&2
  exit 1
fi

cat >"$SANDBOX/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs" <<'RS'
fn apply_remote_target_refresh(resources: Vec<DiscoveredResource>) {
    for resource in live_targets {
        apply_discovered_resource(resource);
    }
}

fn annotate_live_remote_target(metadata: &mut Map) {
    metadata.insert("freshness", json!({
        "observed_at_ms": 10,
        "stale_after_ms": 5010,
        "source": "live_refresh",
    }));
}

fn stable_remote_target_entry_signature(map: &mut Map) {
    map.remove("freshness");
}
RS
if CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp picker subject checker accepted per-target linear refresh upsert" >&2
  exit 1
fi

cat >"$SANDBOX/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs" <<'RS'
fn apply_remote_target_refresh(resources: Vec<DiscoveredResource>) {
    upsert_resources_indexed(resources);
}

fn annotate_live_remote_target(metadata: &mut Map) {
    metadata.insert("freshness", json!({
        "observed_at_ms": 10,
        "stale_after_ms": 5010,
        "source": "live_refresh",
    }));
}

fn stable_remote_target_entry_signature(map: &mut Map) {
    map.remove("freshness");
}
RS

cat >"$SANDBOX/src/daemon/resources/projection.rs" <<'RS'
fn validate_remote_target_freshness(entry: ResourceEntry) {
    entry.metadata["freshness"]["observed_at_ms"];
    entry.metadata["freshness"]["stale_after_ms"];
    entry.metadata["freshness"]["source"];
}

fn resource_entry() -> ResourceEntry {
    ResourceEntry {
        owner_agent: "easynet:///r/acme/device/dev-1".to_string(),
    }
}
RS
if CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp picker subject checker accepted Device URA owner_agent fixture" >&2
  exit 1
fi

cat >"$SANDBOX/src/daemon/ability/builtins/resources/watch_remote_targets.rs" <<'RS'
fn stable_resource_signature(map: &mut Map) {
    map.remove("freshness");
}
RS
if CHECK_REMOTEAPP_PICKER_SUBJECT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp picker subject checker accepted non-injectable watch target stream" >&2
  exit 1
fi

echo "test_check_remoteapp_picker_subject_boundary.sh: all cases passed"

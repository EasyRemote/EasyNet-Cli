// EasyNet CLI — resource.refresh_remote_targets ability handler
// =============================================================
//
// Daemon-owned live inventory refresh for display/window/application targets.
// This is deliberately separate from remote_desktop.* session abilities:
// resource inventory owns ResourceEntry persistence and freshness; remote
// desktop consumes a selected resource subject and resolves it into a session
// target binding.

use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::daemon::ability::builtins::resources::media::resource_bootstrap;
use crate::daemon::ability::descriptors::AdmissionAction;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::persistence::resources::ResourceType;
use crate::daemon::resources::projection::RemoteTargetListEntry;

pub const ABILITY_RESOURCE_REFRESH_REMOTE_TARGETS: &str =
    crate::daemon::ability::names::resources::RESOURCE_REFRESH_REMOTE_TARGETS;

const REMOTE_TARGET_TYPES: &[ResourceType] = &[
    ResourceType::Display,
    ResourceType::Application,
    ResourceType::Window,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTargetInventoryContext {
    realm: String,
    owner_agent: String,
    host_device_ura: String,
}

impl RemoteTargetInventoryContext {
    pub fn from_device_ura(device_ura: &str) -> anyhow::Result<Self> {
        let parsed = crate::core::ura::parse_ura(device_ura)?;
        if parsed.kind != crate::core::ura::URAKind::Device {
            anyhow::bail!("remote target inventory owner must be a Device URA, got {device_ura}");
        }
        let device_id = parsed.device_id().ok_or_else(|| {
            anyhow::anyhow!("remote target inventory owner Device URA is missing device id")
        })?;
        Ok(Self {
            owner_agent: crate::core::ura::device_agent_ura(
                &parsed.realm,
                device_id,
                crate::daemon::ability::names::resources::MEDIA_SYSTEM_AGENT_ID,
            ),
            realm: parsed.realm,
            host_device_ura: device_ura.to_string(),
        })
    }

    pub fn owner_agent(&self) -> &str {
        &self.owner_agent
    }

    pub fn host_device_ura(&self) -> &str {
        &self.host_device_ura
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RemoteTargetRefreshResponse {
    pub observed_at_ms: u64,
    pub freshness_ttl_ms: u64,
    pub retired_count: usize,
    pub screen_target_discovery_available: bool,
    pub resources: Vec<RemoteTargetListEntry>,
}

pub fn register(reg: &mut AxonAbilityCatalog, context: RemoteTargetInventoryContext) {
    reg.register_rpc_with_owner_and_action(
        ABILITY_RESOURCE_REFRESH_REMOTE_TARGETS,
        OwnerKind::media_system(),
        AdmissionAction::Manage,
        Arc::new(move |args| handler(args, &context)),
    );
}

fn handler(args: Value, context: &RemoteTargetInventoryContext) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(refresh_response(args, context)?)?)
}

pub(in crate::daemon::ability::builtins::resources) fn refresh_response(
    args: Value,
    context: &RemoteTargetInventoryContext,
) -> anyhow::Result<RemoteTargetRefreshResponse> {
    remote_target_response(args, context, resource_bootstrap::refresh_remote_targets)
}

pub(in crate::daemon::ability::builtins::resources) fn watch_response(
    args: Value,
    context: &RemoteTargetInventoryContext,
) -> anyhow::Result<RemoteTargetRefreshResponse> {
    remote_target_response(
        args,
        context,
        resource_bootstrap::watch_remote_target_inventory,
    )
}

fn remote_target_response(
    args: Value,
    context: &RemoteTargetInventoryContext,
    refresh: impl FnOnce(&str, &str) -> anyhow::Result<resource_bootstrap::RemoteTargetInventoryRefresh>,
) -> anyhow::Result<RemoteTargetRefreshResponse> {
    let kinds = parse_target_kinds(args.get("types"))?;
    let refresh = refresh(&context.realm, &context.owner_agent)?;
    let resources = refresh
        .resources
        .iter()
        .filter(|entry| kinds.is_empty() || kinds.contains(&entry.kind))
        .map(RemoteTargetListEntry::from_resource_entry)
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(RemoteTargetRefreshResponse {
        observed_at_ms: refresh.observed_at_ms,
        freshness_ttl_ms: refresh.freshness_ttl_ms,
        retired_count: refresh.retired_count,
        screen_target_discovery_available: refresh.screen_target_discovery_available,
        resources,
    })
}

pub(in crate::daemon::ability::builtins::resources) fn parse_target_kinds(
    raw: Option<&Value>,
) -> anyhow::Result<Vec<ResourceType>> {
    let Some(value) = raw else {
        return Ok(Vec::new());
    };
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                let value = value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("`types[]` entries must be strings"))?;
                let kind = ResourceType::from_str(value)?;
                if REMOTE_TARGET_TYPES.contains(&kind) {
                    Ok(kind)
                } else {
                    anyhow::bail!(
                        "resource.refresh_remote_targets only supports display, application, and window targets; got {value}"
                    )
                }
            })
            .collect(),
        other => anyhow::bail!("`types` must be an array of strings, got {other}"),
    }
}

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "types": {
                "type": "array",
                "description": "Optional filter for refreshed remote targets. Absent or empty returns display, application, and window rows.",
                "items": {
                    "type": "string",
                    "enum": ["display", "application", "window"],
                }
            }
        }
    })
}

pub fn description() -> &'static str {
    "Refresh the daemon-local live inventory of display, application, and \
     window resources. The handler owns the host-local discovery pass, \
     atomically updates the resource cache, and returns a live projection with \
     freshness metadata for target pickers. `meta.list_resources` remains a \
     read-only cache projection."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_requires_device_ura() {
        let err = RemoteTargetInventoryContext::from_device_ura("easynet:///r/acme/authority")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Device URA"), "unexpected error: {err}");
    }

    #[test]
    fn context_projects_device_to_media_system_agent_owner() {
        let context =
            RemoteTargetInventoryContext::from_device_ura("easynet:///r/acme/device/dev-a")
                .expect("context");

        assert_eq!(
            context.owner_agent(),
            "easynet:///r/acme/agent/device.dev-a.media"
        );
        assert_eq!(context.host_device_ura(), "easynet:///r/acme/device/dev-a");
    }

    #[test]
    fn parse_target_kinds_rejects_non_remote_target_types() {
        let err = parse_target_kinds(Some(&json!(["camera"])))
            .unwrap_err()
            .to_string();
        assert!(err.contains("camera"), "unexpected error: {err}");
    }

    #[test]
    fn registration_makes_refresh_dispatchable_under_media_system() {
        let mut reg = AxonAbilityCatalog::new_test_metadata_for_device_authority(
            "easynet:///r/test/device/resource-refresh",
        );
        register(
            &mut reg,
            RemoteTargetInventoryContext::from_device_ura(
                "easynet:///r/test/device/resource-refresh",
            )
            .unwrap(),
        );
        assert!(reg
            .get_rpc(ABILITY_RESOURCE_REFRESH_REMOTE_TARGETS)
            .is_some());
        assert_eq!(
            reg.control_plane_owner(ABILITY_RESOURCE_REFRESH_REMOTE_TARGETS),
            Some(OwnerKind::media_system())
        );
    }
}

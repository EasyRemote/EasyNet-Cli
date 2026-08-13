// EasyNet CLI — resource.watch_remote_targets ability handler
// ===========================================================
//
// Daemon-owned inventory stream for display/window/application targets.
// The remote desktop plugin consumes selected resource subjects; it does not
// own host target enumeration or resource cache persistence.

use std::{collections::BTreeMap, sync::Arc, thread, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};

use crate::daemon::ability::builtins::resources::refresh_remote_targets::{
    parse_target_kinds, watch_response, RemoteTargetInventoryContext,
};
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind, StreamSource};
use crate::daemon::persistence::resources::ResourceType;
use crate::daemon::resources::projection::RemoteTargetListEntry;

pub const ABILITY_RESOURCE_WATCH_REMOTE_TARGETS: &str =
    crate::daemon::ability::names::resources::RESOURCE_WATCH_REMOTE_TARGETS;

const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;
const MIN_POLL_INTERVAL_MS: u64 = 250;
const MAX_POLL_INTERVAL_MS: u64 = 10_000;
const WATCH_CHANNEL_CAPACITY: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RemoteTargetWatchEvent {
    pub event_id: u64,
    pub event_type: String,
    pub inventory_hash: String,
    pub observed_at_ms: u64,
    pub freshness_ttl_ms: u64,
    pub retired_count: usize,
    pub screen_target_discovery_available: bool,
    pub added: Vec<RemoteTargetListEntry>,
    pub updated: Vec<RemoteTargetListEntry>,
    pub removed_resource_uras: Vec<String>,
    pub resources: Vec<RemoteTargetListEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WatchConfig {
    types: Vec<ResourceType>,
    poll_interval_ms: u64,
    max_events: Option<u64>,
}

impl WatchConfig {
    fn parse(args: &Value) -> anyhow::Result<Self> {
        if !args.is_null() && !args.is_object() {
            anyhow::bail!("resource.watch_remote_targets args must be an object");
        }
        let types = parse_target_kinds(args.get("types"))?;
        let poll_interval_ms = match args.get("poll_interval_ms") {
            Some(Value::Null) | None => DEFAULT_POLL_INTERVAL_MS,
            Some(Value::Number(value)) => value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("`poll_interval_ms` must be an unsigned integer"))?,
            Some(other) => {
                anyhow::bail!("`poll_interval_ms` must be an unsigned integer, got {other}")
            }
        }
        .clamp(MIN_POLL_INTERVAL_MS, MAX_POLL_INTERVAL_MS);
        let max_events = match args.get("max_events") {
            Some(Value::Null) | None => None,
            Some(Value::Number(value)) => {
                let max_events = value
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("`max_events` must be an unsigned integer"))?;
                if max_events == 0 {
                    anyhow::bail!("`max_events` must be greater than zero");
                }
                Some(max_events)
            }
            Some(other) => anyhow::bail!("`max_events` must be an unsigned integer, got {other}"),
        };
        Ok(Self {
            types,
            poll_interval_ms,
            max_events,
        })
    }

    fn refresh_args(&self) -> Value {
        if self.types.is_empty() {
            json!({})
        } else {
            json!({
                "types": self.types.iter().map(|kind| kind.as_str()).collect::<Vec<_>>()
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RemoteTargetInventorySnapshot {
    observed_at_ms: u64,
    freshness_ttl_ms: u64,
    retired_count: usize,
    screen_target_discovery_available: bool,
    inventory_hash: String,
    signatures: BTreeMap<String, String>,
    resources_by_ura: BTreeMap<String, RemoteTargetListEntry>,
}

impl RemoteTargetInventorySnapshot {
    fn refresh(
        context: &RemoteTargetInventoryContext,
        config: &WatchConfig,
    ) -> anyhow::Result<Self> {
        let response = watch_response(config.refresh_args(), context)?;
        let mut signatures = BTreeMap::new();
        let mut resources_by_ura = BTreeMap::new();
        for resource in response.resources {
            let signature = stable_resource_signature(&resource)?;
            signatures.insert(resource.resource_ura.clone(), signature);
            resources_by_ura.insert(resource.resource_ura.clone(), resource);
        }
        let inventory_hash = inventory_hash(&signatures);
        Ok(Self {
            observed_at_ms: response.observed_at_ms,
            freshness_ttl_ms: response.freshness_ttl_ms,
            retired_count: response.retired_count,
            screen_target_discovery_available: response.screen_target_discovery_available,
            inventory_hash,
            signatures,
            resources_by_ura,
        })
    }

    fn snapshot_event(&self, event_id: u64) -> RemoteTargetWatchEvent {
        let resources = self.resources();
        RemoteTargetWatchEvent {
            event_id,
            event_type: "target_inventory_snapshot".to_string(),
            inventory_hash: self.inventory_hash.clone(),
            observed_at_ms: self.observed_at_ms,
            freshness_ttl_ms: self.freshness_ttl_ms,
            retired_count: self.retired_count,
            screen_target_discovery_available: self.screen_target_discovery_available,
            added: resources.clone(),
            updated: Vec::new(),
            removed_resource_uras: Vec::new(),
            resources,
        }
    }

    fn delta_event(
        &self,
        previous: &RemoteTargetInventorySnapshot,
        event_id: u64,
    ) -> Option<RemoteTargetWatchEvent> {
        if self.inventory_hash == previous.inventory_hash {
            return None;
        }

        let mut added = Vec::new();
        let mut updated = Vec::new();
        let mut removed_resource_uras = Vec::new();

        for (resource_ura, signature) in &self.signatures {
            match previous.signatures.get(resource_ura) {
                None => {
                    if let Some(resource) = self.resources_by_ura.get(resource_ura) {
                        added.push(resource.clone());
                    }
                }
                Some(previous_signature) if previous_signature != signature => {
                    if let Some(resource) = self.resources_by_ura.get(resource_ura) {
                        updated.push(resource.clone());
                    }
                }
                Some(_) => {}
            }
        }

        for resource_ura in previous.signatures.keys() {
            if !self.signatures.contains_key(resource_ura) {
                removed_resource_uras.push(resource_ura.clone());
            }
        }

        Some(RemoteTargetWatchEvent {
            event_id,
            event_type: "target_inventory_delta".to_string(),
            inventory_hash: self.inventory_hash.clone(),
            observed_at_ms: self.observed_at_ms,
            freshness_ttl_ms: self.freshness_ttl_ms,
            retired_count: self.retired_count,
            screen_target_discovery_available: self.screen_target_discovery_available,
            added,
            updated,
            removed_resource_uras,
            resources: self.resources(),
        })
    }

    fn resources(&self) -> Vec<RemoteTargetListEntry> {
        self.resources_by_ura.values().cloned().collect()
    }
}

pub fn register(reg: &mut AxonAbilityCatalog, context: RemoteTargetInventoryContext) {
    reg.register_stream_with_spec(
        ABILITY_RESOURCE_WATCH_REMOTE_TARGETS,
        OwnerKind::media_system(),
        crate::daemon::ability::catalog::system_manifest::registry_manifest(
            ABILITY_RESOURCE_WATCH_REMOTE_TARGETS,
            description(),
            input_schema(),
        ),
        Arc::new(move |args| handler(args, &context)),
    );
}

fn handler(args: Value, context: &RemoteTargetInventoryContext) -> anyhow::Result<StreamSource> {
    let config = WatchConfig::parse(&args)?;
    let context = context.clone();
    let (tx, rx) = tokio::sync::mpsc::channel(WATCH_CHANNEL_CAPACITY);

    thread::Builder::new()
        .name("easynet-remote-target-inventory-watch".to_string())
        .spawn(move || {
            let mut previous = None;
            let mut next_event_id = 1_u64;
            let mut emitted = 0_u64;

            loop {
                let snapshot = match RemoteTargetInventorySnapshot::refresh(&context, &config) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        let _ = tx.blocking_send(Err(error));
                        break;
                    }
                };

                let event = match previous.as_ref() {
                    None => Some(snapshot.snapshot_event(next_event_id)),
                    Some(previous) => snapshot.delta_event(previous, next_event_id),
                };

                if let Some(event) = event {
                    let value = match serde_json::to_value(event) {
                        Ok(value) => value,
                        Err(error) => {
                            let _ = tx.blocking_send(Err(error.into()));
                            break;
                        }
                    };
                    if tx.blocking_send(Ok(value)).is_err() {
                        break;
                    }
                    emitted = emitted.saturating_add(1);
                    next_event_id = next_event_id.saturating_add(1);
                    if config
                        .max_events
                        .is_some_and(|max_events| emitted >= max_events)
                    {
                        break;
                    }
                }

                previous = Some(snapshot);
                thread::sleep(Duration::from_millis(config.poll_interval_ms));
            }
        })
        .map_err(|error| anyhow::anyhow!("spawn remote target inventory watch thread: {error}"))?;

    Ok(StreamSource::Finite(rx))
}

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "types": {
                "type": "array",
                "description": "Optional filter for watched remote targets. Absent or empty returns display, application, and window rows.",
                "items": {
                    "type": "string",
                    "enum": ["display", "application", "window"],
                }
            },
            "poll_interval_ms": {
                "type": "integer",
                "minimum": MIN_POLL_INTERVAL_MS,
                "maximum": MAX_POLL_INTERVAL_MS,
                "default": DEFAULT_POLL_INTERVAL_MS,
                "description": "Bounded host-local polling interval for inventory diffs. Values outside the supported range are clamped."
            },
            "max_events": {
                "type": "integer",
                "minimum": 1,
                "description": "Optional bounded event count for diagnostics and tests. Omit for an open-ended watch."
            }
        }
    })
}

pub fn description() -> &'static str {
    "Watch the daemon-local display, application, and window resource \
     inventory. The first stream frame is a fresh inventory snapshot; \
     subsequent frames are emitted only when the stable inventory signature \
     changes. Discovery and cache ownership stay in the daemon resource \
     inventory layer instead of the remote desktop plugin."
}

fn stable_resource_signature(resource: &RemoteTargetListEntry) -> anyhow::Result<String> {
    let mut metadata = resource.metadata.clone();
    if let Value::Object(map) = &mut metadata {
        map.remove("observed_at_ms");
        map.remove("freshness_ttl_ms");
    }
    serde_json::to_string(&json!({
        "resource_ura": resource.resource_ura,
        "owner_agent": resource.owner_agent,
        "host_device_ura": resource.host_device_ura,
        "type": resource.entry_type,
        "binding": resource.binding,
        "display_name": resource.display_name,
        "availability": resource.availability,
        "stale_reason": resource.stale_reason,
        "metadata": metadata,
    }))
    .map_err(Into::into)
}

fn inventory_hash(signatures: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    for (resource_ura, signature) in signatures {
        hasher.update(resource_ura.as_bytes());
        hasher.update([0]);
        hasher.update(signature.as_bytes());
        hasher.update([0xff]);
    }
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_makes_watch_dispatchable_under_media_system() {
        let mut reg = AxonAbilityCatalog::new_test_metadata_for_device_authority(
            "easynet:///r/test/device/resource-watch",
        );
        register(
            &mut reg,
            RemoteTargetInventoryContext::from_device_ura(
                "easynet:///r/test/device/resource-watch",
            )
            .unwrap(),
        );
        assert!(reg
            .get_stream(ABILITY_RESOURCE_WATCH_REMOTE_TARGETS)
            .is_some());
        assert_eq!(
            reg.control_plane_owner(ABILITY_RESOURCE_WATCH_REMOTE_TARGETS),
            Some(OwnerKind::media_system())
        );
    }

    #[test]
    fn watch_event_schema_is_snapshot_not_remote_desktop_session_state() {
        let event = RemoteTargetWatchEvent {
            event_id: 1,
            event_type: "target_inventory_snapshot".to_string(),
            inventory_hash: "sha256:test".to_string(),
            observed_at_ms: 10,
            freshness_ttl_ms: 5_000,
            retired_count: 0,
            screen_target_discovery_available: true,
            added: Vec::new(),
            updated: Vec::new(),
            removed_resource_uras: Vec::new(),
            resources: Vec::new(),
        };
        let value = serde_json::to_value(event).expect("event serializes");
        assert_eq!(value["event_type"], "target_inventory_snapshot");
        assert!(value.get("session_id").is_none());
        assert!(value.get("target_binding").is_none());
    }

    #[test]
    fn stable_signature_ignores_freshness_metadata() {
        let first = remote_target_entry("res-a", "Window A", 10);
        let second = remote_target_entry("res-a", "Window A", 20);

        assert_eq!(
            stable_resource_signature(&first).unwrap(),
            stable_resource_signature(&second).unwrap()
        );
    }

    #[test]
    fn delta_event_reports_added_updated_and_removed_targets() {
        let previous = snapshot(vec![
            remote_target_entry("res-a", "Window A", 10),
            remote_target_entry("res-b", "Window B", 10),
        ]);
        let current = snapshot(vec![
            remote_target_entry("res-a", "Window A moved", 20),
            remote_target_entry("res-c", "Window C", 20),
        ]);

        let event = current
            .delta_event(&previous, 2)
            .expect("inventory changed");

        assert_eq!(event.event_type, "target_inventory_delta");
        assert_eq!(
            event
                .added
                .iter()
                .map(|resource| resource.resource_ura.as_str())
                .collect::<Vec<_>>(),
            vec!["easynet:///r/test/resource/res-c"]
        );
        assert_eq!(
            event
                .updated
                .iter()
                .map(|resource| resource.resource_ura.as_str())
                .collect::<Vec<_>>(),
            vec!["easynet:///r/test/resource/res-a"]
        );
        assert_eq!(
            event.removed_resource_uras,
            vec!["easynet:///r/test/resource/res-b"]
        );
    }

    #[test]
    fn identical_inventory_has_no_delta_event() {
        let previous = snapshot(vec![remote_target_entry("res-a", "Window A", 10)]);
        let current = snapshot(vec![remote_target_entry("res-a", "Window A", 20)]);

        assert!(current.delta_event(&previous, 2).is_none());
    }

    fn snapshot(resources: Vec<RemoteTargetListEntry>) -> RemoteTargetInventorySnapshot {
        let mut signatures = BTreeMap::new();
        let mut resources_by_ura = BTreeMap::new();
        for resource in resources {
            signatures.insert(
                resource.resource_ura.clone(),
                stable_resource_signature(&resource).unwrap(),
            );
            resources_by_ura.insert(resource.resource_ura.clone(), resource);
        }
        RemoteTargetInventorySnapshot {
            observed_at_ms: 0,
            freshness_ttl_ms: 5_000,
            retired_count: 0,
            screen_target_discovery_available: true,
            inventory_hash: inventory_hash(&signatures),
            signatures,
            resources_by_ura,
        }
    }

    fn remote_target_entry(
        resource_id: &str,
        display_name: &str,
        observed_at_ms: u64,
    ) -> RemoteTargetListEntry {
        RemoteTargetListEntry {
            resource_ura: format!("easynet:///r/test/resource/{resource_id}"),
            owner_agent: "easynet:///r/test/agent/device.dev.media".to_string(),
            host_device_ura: "easynet:///r/test/device/dev".to_string(),
            entry_type: "window".to_string(),
            binding: "local_device".to_string(),
            display_name: display_name.to_string(),
            availability: "available".to_string(),
            observed_at_ms,
            freshness_ttl_ms: 5_000,
            stale_reason: None,
            metadata: json!({
                "window_id": resource_id,
                "observed_at_ms": observed_at_ms,
                "freshness_ttl_ms": 5_000,
            }),
        }
    }
}

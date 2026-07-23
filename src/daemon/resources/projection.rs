// EasyNet CLI — daemon resource response projections
// ==================================================
//
// File: src/daemon/resources/projection.rs
// Description: Public DTO projections for daemon-owned resource discovery.
//
// Protocol Responsibility
// -----------------------
// Keep public resource discovery responses separate from the daemon's on-disk
// resource registry. Resource persistence may carry hardware and audit fields;
// remote callers receive only the canonical runtime-facing resource facts.
//
// Implementation Approach
// -----------------------
// Projection constructors copy only public fields from persistence records and
// use typed DTO structs with fail-closed serde boundaries.
//
// Usage Contract
// --------------
// Ability handlers should call these constructors instead of assembling raw
// JSON response objects. Persistence-only fields such as `hardware_id` and
// `first_seen_at` must not cross this boundary.
//
// Architectural Position
// ----------------------
// Daemon resource projection layer. Depends on daemon persistence records but
// contains no ability dispatch, route, admission, or product lifecycle logic.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::daemon::persistence::resources::ResourceEntry;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceListEntry {
    pub resource_ura: String,
    pub owner_agent: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub binding: String,
    pub display_name: String,
    pub metadata: Value,
}

impl ResourceListEntry {
    pub fn from_resource_entry(entry: &ResourceEntry) -> Self {
        Self {
            resource_ura: entry.resource_ura.clone(),
            owner_agent: entry.owner_agent.clone(),
            entry_type: entry.kind.as_str().to_string(),
            binding: entry.binding.as_str().to_string(),
            display_name: entry.display_name.clone(),
            metadata: entry.metadata.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceListResponse {
    pub resources: Vec<ResourceListEntry>,
}

impl ResourceListResponse {
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = &'a ResourceEntry>) -> Self {
        Self {
            resources: entries
                .into_iter()
                .map(ResourceListEntry::from_resource_entry)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::persistence::resources::{ResourceBinding, ResourceType};
    use serde_json::json;

    fn resource_entry() -> ResourceEntry {
        ResourceEntry {
            resource_ura: "easynet:///r/acme/resource/device.dev-1/streams/camera.front"
                .to_string(),
            owner_agent: "easynet:///r/acme/device/dev-1".to_string(),
            kind: ResourceType::Camera,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "platform-camera-0".to_string(),
            display_name: "Front Camera".to_string(),
            metadata: json!({"width": 1920, "height": 1080}),
            first_seen_at: "2026-07-24T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn resource_list_entry_preserves_public_shape_without_persistence_fields() {
        let entry = ResourceListEntry::from_resource_entry(&resource_entry());
        let wire = serde_json::to_value(&entry).expect("entry serializes");

        assert_eq!(
            wire["resource_ura"],
            "easynet:///r/acme/resource/device.dev-1/streams/camera.front"
        );
        assert_eq!(wire["owner_agent"], "easynet:///r/acme/device/dev-1");
        assert_eq!(wire["type"], "camera");
        assert_eq!(wire["binding"], "local_device");
        assert_eq!(wire["display_name"], "Front Camera");
        assert_eq!(wire["metadata"]["width"], 1920);
        assert!(wire.get("hardware_id").is_none());
        assert!(wire.get("first_seen_at").is_none());
    }

    #[test]
    fn resource_list_response_preserves_public_shape() {
        let entry = resource_entry();
        let response = ResourceListResponse::from_entries([&entry]);
        let wire = serde_json::to_value(&response).expect("response serializes");

        assert_eq!(wire["resources"][0]["type"], "camera");
        assert_eq!(wire["resources"][0]["binding"], "local_device");
    }

    #[test]
    fn resource_list_entry_rejects_unknown_fields() {
        let error = serde_json::from_value::<ResourceListEntry>(json!({
            "resource_ura": "easynet:///r/acme/resource/device.dev-1/streams/camera.front",
            "owner_agent": "easynet:///r/acme/device/dev-1",
            "type": "camera",
            "binding": "local_device",
            "display_name": "Front Camera",
            "metadata": {},
            "hardware_id": "platform-camera-0"
        }))
        .expect_err("resource-list entry must reject persistence-only fields");

        assert!(
            error.to_string().contains("hardware_id"),
            "strict entry error should name unknown field: {error}"
        );
    }

    #[test]
    fn resource_list_response_rejects_unknown_fields() {
        let error = serde_json::from_value::<ResourceListResponse>(json!({
            "resources": [],
            "legacy_resources": []
        }))
        .expect_err("resource-list response must reject legacy envelopes");

        assert!(
            error.to_string().contains("legacy_resources"),
            "strict response error should name unknown field: {error}"
        );
    }
}

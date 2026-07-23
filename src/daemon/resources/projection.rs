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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FilesPutResponse {
    pub ura: String,
    pub sha256: String,
    pub size: u64,
    pub content_type: String,
    pub filename: String,
}

impl FilesPutResponse {
    pub fn success(
        ura: impl Into<String>,
        sha256: impl Into<String>,
        size: u64,
        content_type: impl Into<String>,
        filename: impl Into<String>,
    ) -> Self {
        Self {
            ura: ura.into(),
            sha256: sha256.into(),
            size,
            content_type: content_type.into(),
            filename: filename.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FilesGetResponse {
    pub bytes_b64: String,
    pub content_type: String,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
}

impl FilesGetResponse {
    pub fn success(
        bytes_b64: impl Into<String>,
        content_type: impl Into<String>,
        filename: impl Into<String>,
        sha256: impl Into<String>,
        size: u64,
    ) -> Self {
        Self {
            bytes_b64: bytes_b64.into(),
            content_type: content_type.into(),
            filename: filename.into(),
            sha256: sha256.into(),
            size,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FilesListItem {
    pub sha256: String,
    pub size: u64,
    pub filename: String,
    pub content_type: String,
    pub ura: String,
}

impl FilesListItem {
    pub fn new(
        sha256: impl Into<String>,
        size: u64,
        filename: impl Into<String>,
        content_type: impl Into<String>,
        ura: impl Into<String>,
    ) -> Self {
        Self {
            sha256: sha256.into(),
            size,
            filename: filename.into(),
            content_type: content_type.into(),
            ura: ura.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FilesListResponse {
    pub items: Vec<FilesListItem>,
}

impl FilesListResponse {
    pub fn from_items(items: Vec<FilesListItem>) -> Self {
        Self { items }
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

    #[test]
    fn files_put_response_preserves_public_shape() {
        let response = FilesPutResponse::success(
            "easynet:///r/acme/resource/alice.files/abc",
            "abc",
            12,
            "text/plain",
            "a.txt",
        );
        let wire = serde_json::to_value(&response).expect("put response serializes");

        assert_eq!(wire["ura"], "easynet:///r/acme/resource/alice.files/abc");
        assert_eq!(wire["sha256"], "abc");
        assert_eq!(wire["size"], 12);
        assert_eq!(wire["content_type"], "text/plain");
        assert_eq!(wire["filename"], "a.txt");
    }

    #[test]
    fn files_get_response_preserves_public_shape() {
        let response = FilesGetResponse::success("aGVsbG8=", "text/plain", "a.txt", "abc", 5);
        let wire = serde_json::to_value(&response).expect("get response serializes");

        assert_eq!(wire["bytes_b64"], "aGVsbG8=");
        assert_eq!(wire["content_type"], "text/plain");
        assert_eq!(wire["filename"], "a.txt");
        assert_eq!(wire["sha256"], "abc");
        assert_eq!(wire["size"], 5);
    }

    #[test]
    fn files_list_response_preserves_public_shape() {
        let response = FilesListResponse::from_items(vec![FilesListItem::new(
            "abc",
            12,
            "a.txt",
            "text/plain",
            "easynet:///r/acme/resource/alice.files/abc",
        )]);
        let wire = serde_json::to_value(&response).expect("list response serializes");

        assert_eq!(wire["items"][0]["sha256"], "abc");
        assert_eq!(wire["items"][0]["size"], 12);
        assert_eq!(wire["items"][0]["filename"], "a.txt");
        assert_eq!(wire["items"][0]["content_type"], "text/plain");
        assert_eq!(
            wire["items"][0]["ura"],
            "easynet:///r/acme/resource/alice.files/abc"
        );
    }

    #[test]
    fn files_store_response_dtos_reject_unknown_fields() {
        let put_error = serde_json::from_value::<FilesPutResponse>(json!({
            "ura": "easynet:///r/acme/resource/alice.files/abc",
            "sha256": "abc",
            "size": 12,
            "content_type": "text/plain",
            "filename": "a.txt",
            "path": "/tmp/a.txt"
        }))
        .expect_err("put response must reject local path leaks");
        assert!(
            put_error.to_string().contains("path"),
            "strict put response error should name unknown field: {put_error}"
        );

        let get_error = serde_json::from_value::<FilesGetResponse>(json!({
            "bytes_b64": "aGVsbG8=",
            "content_type": "text/plain",
            "filename": "a.txt",
            "sha256": "abc",
            "size": 5,
            "metadata_path": "/tmp/a.metadata.json"
        }))
        .expect_err("get response must reject local metadata path leaks");
        assert!(
            get_error.to_string().contains("metadata_path"),
            "strict get response error should name unknown field: {get_error}"
        );

        let list_error = serde_json::from_value::<FilesListResponse>(json!({
            "items": [],
            "legacy_items": []
        }))
        .expect_err("list response must reject legacy envelopes");
        assert!(
            list_error.to_string().contains("legacy_items"),
            "strict list response error should name unknown field: {list_error}"
        );
    }
}

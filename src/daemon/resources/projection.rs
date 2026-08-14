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

use crate::daemon::persistence::resources::{ResourceEntry, ResourceType};

const REMOTE_TARGET_REFRESH_ABILITY: &str = "resource.refresh_remote_targets";
const REMOTE_TARGET_WATCH_ABILITY: &str = "resource.watch_remote_targets";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_projection: Option<ResourceCacheProjection>,
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
            cache_projection: ResourceCacheProjection::for_resource_entry(entry),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceCacheProjection {
    pub source: String,
    pub selection_state: String,
    pub live_refresh_required: bool,
    pub refresh_ability: String,
    pub watch_ability: String,
    pub availability: Option<String>,
    pub freshness: Option<Value>,
}

impl ResourceCacheProjection {
    fn for_resource_entry(entry: &ResourceEntry) -> Option<Self> {
        if !matches!(
            entry.kind,
            ResourceType::Display | ResourceType::Application | ResourceType::Window
        ) {
            return None;
        }
        Some(Self {
            source: "meta.list_resources.cache_projection".to_string(),
            selection_state: "cached_requires_live_refresh".to_string(),
            live_refresh_required: true,
            refresh_ability: REMOTE_TARGET_REFRESH_ABILITY.to_string(),
            watch_ability: REMOTE_TARGET_WATCH_ABILITY.to_string(),
            availability: optional_metadata_str(entry, "availability"),
            freshness: entry.metadata.get("freshness").cloned(),
        })
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RemoteTargetListEntry {
    pub resource_ura: String,
    pub owner_agent: String,
    pub host_device_ura: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub binding: String,
    pub display_name: String,
    pub availability: String,
    pub observed_at_ms: u64,
    pub freshness_ttl_ms: u64,
    pub stale_reason: Option<String>,
    pub metadata: Value,
}

impl RemoteTargetListEntry {
    pub fn from_resource_entry(entry: &ResourceEntry) -> anyhow::Result<Self> {
        let host_device_ura = required_metadata_str(entry, "host_device_ura")?;
        let parsed = crate::core::ura::parse_ura(&host_device_ura)?;
        if parsed.kind != crate::core::ura::URAKind::Device {
            anyhow::bail!(
                "remote target {:?} host_device_ura must be a Device URA, got {host_device_ura}",
                entry.resource_ura
            );
        }
        validate_remote_target_freshness(entry)?;
        Ok(Self {
            resource_ura: entry.resource_ura.clone(),
            owner_agent: entry.owner_agent.clone(),
            host_device_ura,
            entry_type: entry.kind.as_str().to_string(),
            binding: entry.binding.as_str().to_string(),
            display_name: entry.display_name.clone(),
            availability: required_metadata_str(entry, "availability")?,
            observed_at_ms: required_metadata_u64(entry, "observed_at_ms")?,
            freshness_ttl_ms: required_metadata_u64(entry, "freshness_ttl_ms")?,
            stale_reason: optional_metadata_str(entry, "stale_reason"),
            metadata: entry.metadata.clone(),
        })
    }
}

fn validate_remote_target_freshness(entry: &ResourceEntry) -> anyhow::Result<()> {
    let freshness = entry
        .metadata
        .get("freshness")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "remote target {:?} metadata must include freshness object",
                entry.resource_ura
            )
        })?;
    let observed_at_ms = freshness
        .get("observed_at_ms")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "remote target {:?} freshness must include observed_at_ms",
                entry.resource_ura
            )
        })?;
    let stale_after_ms = freshness
        .get("stale_after_ms")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "remote target {:?} freshness must include stale_after_ms",
                entry.resource_ura
            )
        })?;
    if stale_after_ms < observed_at_ms {
        anyhow::bail!(
            "remote target {:?} freshness stale_after_ms must be >= observed_at_ms",
            entry.resource_ura
        );
    }
    required_nested_metadata_str(entry, freshness, "freshness", "source")?;
    Ok(())
}

fn required_nested_metadata_str(
    entry: &ResourceEntry,
    object: &serde_json::Map<String, Value>,
    object_key: &str,
    key: &str,
) -> anyhow::Result<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "remote target {:?} metadata.{object_key} must include non-empty string field {key:?}",
                entry.resource_ura
            )
        })
}

fn required_metadata_str(entry: &ResourceEntry, key: &str) -> anyhow::Result<String> {
    optional_metadata_str(entry, key).ok_or_else(|| {
        anyhow::anyhow!(
            "remote target {:?} metadata must include non-empty string field {key:?}",
            entry.resource_ura
        )
    })
}

fn optional_metadata_str(entry: &ResourceEntry, key: &str) -> Option<String> {
    entry
        .metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn required_metadata_u64(entry: &ResourceEntry, key: &str) -> anyhow::Result<u64> {
    entry
        .metadata
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "remote target {:?} metadata must include u64 field {key:?}",
                entry.resource_ura
            )
        })
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PagesProjectListItem {
    pub user: String,
    pub project_id: String,
    pub folder: String,
    pub visibility: String,
    pub started_at_ms: u64,
    pub url_root: String,
    pub dev_listener_url_root: String,
}

impl PagesProjectListItem {
    pub fn new(
        user: impl Into<String>,
        project_id: impl Into<String>,
        folder: impl Into<String>,
        visibility: impl Into<String>,
        started_at_ms: u64,
        url_root: impl Into<String>,
        dev_listener_url_root: impl Into<String>,
    ) -> Self {
        Self {
            user: user.into(),
            project_id: project_id.into(),
            folder: folder.into(),
            visibility: visibility.into(),
            started_at_ms,
            url_root: url_root.into(),
            dev_listener_url_root: dev_listener_url_root.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PagesProjectListResponse {
    pub projects: Vec<PagesProjectListItem>,
}

impl PagesProjectListResponse {
    pub fn from_projects(projects: Vec<PagesProjectListItem>) -> Self {
        Self { projects }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PagesProjectDetailResponse {
    pub user: String,
    pub project_id: String,
    pub project_ura: String,
    pub folder: String,
    pub visibility: String,
    pub started_at_ms: u64,
    pub url_root: String,
    pub dev_listener_url_root: String,
    pub file_size_cap: u64,
}

impl PagesProjectDetailResponse {
    #[allow(clippy::too_many_arguments)]
    pub fn success(
        user: impl Into<String>,
        project_id: impl Into<String>,
        project_ura: impl Into<String>,
        folder: impl Into<String>,
        visibility: impl Into<String>,
        started_at_ms: u64,
        url_root: impl Into<String>,
        dev_listener_url_root: impl Into<String>,
        file_size_cap: u64,
    ) -> Self {
        Self {
            user: user.into(),
            project_id: project_id.into(),
            project_ura: project_ura.into(),
            folder: folder.into(),
            visibility: visibility.into(),
            started_at_ms,
            url_root: url_root.into(),
            dev_listener_url_root: dev_listener_url_root.into(),
            file_size_cap,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PagesUnpublishResponse {
    pub user: String,
    pub project_id: String,
    pub removed: bool,
}

impl PagesUnpublishResponse {
    pub fn success(user: impl Into<String>, project_id: impl Into<String>) -> Self {
        Self {
            user: user.into(),
            project_id: project_id.into(),
            removed: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PagesHealthCheck {
    pub name: String,
    pub state: String,
    pub ready: bool,
    pub message: Option<String>,
    pub latency_ms: u64,
    pub metadata: Value,
}

impl PagesHealthCheck {
    pub fn new(
        name: impl Into<String>,
        state: impl Into<String>,
        ready: bool,
        message: Option<String>,
        metadata: Value,
    ) -> Self {
        Self {
            name: name.into(),
            state: state.into(),
            ready,
            message,
            latency_ms: 0,
            metadata,
        }
    }

    pub fn pages_registry() -> Self {
        Self::new(
            "pages_registry",
            "ready",
            true,
            None,
            serde_json::json!({"source": "PUBLISHED_PROJECTS"}),
        )
    }

    pub fn project(project_id: Option<&str>, project_found: bool) -> Self {
        Self::new(
            "project",
            if project_found { "ready" } else { "missing" },
            project_found,
            (!project_found).then(|| "project is not published".to_string()),
            serde_json::json!({
                "project_id": project_id,
                "requested": project_id.is_some()
            }),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PagesHealthResponse {
    pub state: String,
    pub ready: bool,
    pub owner_ura: String,
    pub surface_ref: String,
    pub page_count: usize,
    pub checks: Vec<PagesHealthCheck>,
}

impl PagesHealthResponse {
    pub fn new(
        state: impl Into<String>,
        ready: bool,
        owner_ura: impl Into<String>,
        surface_ref: impl Into<String>,
        page_count: usize,
        checks: Vec<PagesHealthCheck>,
    ) -> Self {
        Self {
            state: state.into(),
            ready,
            owner_ura: owner_ura.into(),
            surface_ref: surface_ref.into(),
            page_count,
            checks,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PagesFetchResponse {
    pub bytes_b64: String,
    pub content_type: String,
    pub size_bytes: usize,
    pub force_attachment: bool,
    pub sha256: String,
}

impl PagesFetchResponse {
    pub fn success(
        bytes_b64: impl Into<String>,
        content_type: impl Into<String>,
        size_bytes: usize,
        force_attachment: bool,
        sha256: impl Into<String>,
    ) -> Self {
        Self {
            bytes_b64: bytes_b64.into(),
            content_type: content_type.into(),
            size_bytes,
            force_attachment,
            sha256: sha256.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PagesPublishResponse {
    pub project_ura: String,
    pub url_root: String,
    pub user: String,
    pub project_id: String,
    pub visibility: String,
}

impl PagesPublishResponse {
    pub fn success(
        project_ura: impl Into<String>,
        url_root: impl Into<String>,
        user: impl Into<String>,
        project_id: impl Into<String>,
        visibility: impl Into<String>,
    ) -> Self {
        Self {
            project_ura: project_ura.into(),
            url_root: url_root.into(),
            user: user.into(),
            project_id: project_id.into(),
            visibility: visibility.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PagesApiResponse {
    pub status: u16,
    pub body: Value,
    pub content_type: String,
}

impl PagesApiResponse {
    pub fn json_ok(body: Value) -> Self {
        Self {
            status: 200,
            body,
            content_type: "application/json; charset=utf-8".to_string(),
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
            owner_agent: "easynet:///r/acme/agent/device.dev-1.media".to_string(),
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
        assert_eq!(
            wire["owner_agent"],
            "easynet:///r/acme/agent/device.dev-1.media"
        );
        assert_eq!(wire["type"], "camera");
        assert_eq!(wire["binding"], "local_device");
        assert_eq!(wire["display_name"], "Front Camera");
        assert_eq!(wire["metadata"]["width"], 1920);
        assert!(wire.get("cache_projection").is_none());
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
    fn resource_list_entry_marks_remote_targets_as_cache_only() {
        let mut entry = resource_entry();
        entry.kind = ResourceType::Window;
        entry.resource_ura =
            "easynet:///r/acme/resource/device.dev-1/streams/window.cursor".to_string();
        entry.metadata = json!({
            "availability": "available",
            "freshness": {
                "observed_at_ms": 123_456,
                "stale_after_ms": 128_456,
                "source": "live_refresh",
            },
        });

        let cached = ResourceListEntry::from_resource_entry(&entry);
        let wire = serde_json::to_value(&cached).expect("entry serializes");

        assert_eq!(
            wire["cache_projection"]["source"],
            "meta.list_resources.cache_projection"
        );
        assert_eq!(
            wire["cache_projection"]["selection_state"],
            "cached_requires_live_refresh"
        );
        assert_eq!(wire["cache_projection"]["live_refresh_required"], true);
        assert_eq!(
            wire["cache_projection"]["refresh_ability"],
            "resource.refresh_remote_targets"
        );
        assert_eq!(
            wire["cache_projection"]["watch_ability"],
            "resource.watch_remote_targets"
        );
        assert_eq!(wire["cache_projection"]["availability"], "available");
        assert_eq!(
            wire["cache_projection"]["freshness"]["stale_after_ms"],
            128_456
        );
    }

    #[test]
    fn remote_target_list_entry_promotes_live_inventory_contract() {
        let mut entry = resource_entry();
        entry.kind = ResourceType::Window;
        entry.resource_ura =
            "easynet:///r/acme/resource/device.dev-1/streams/window.cursor".to_string();
        entry.metadata = json!({
            "host_device_ura": "easynet:///r/acme/device/dev-1",
            "availability": "available",
            "observed_at_ms": 123_456,
            "freshness_ttl_ms": 5_000,
            "freshness": {
                "observed_at_ms": 123_456,
                "stale_after_ms": 128_456,
                "source": "live_refresh",
            },
            "stale_reason": null,
            "window_id": 42,
        });

        let target =
            RemoteTargetListEntry::from_resource_entry(&entry).expect("remote target projection");
        let wire = serde_json::to_value(&target).expect("target serializes");

        assert_eq!(wire["type"], "window");
        assert_eq!(wire["host_device_ura"], "easynet:///r/acme/device/dev-1");
        assert_eq!(wire["availability"], "available");
        assert_eq!(wire["observed_at_ms"], 123_456);
        assert_eq!(wire["freshness_ttl_ms"], 5_000);
        assert_eq!(wire["metadata"]["freshness"]["source"], "live_refresh");
        assert_eq!(wire["metadata"]["freshness"]["observed_at_ms"], 123_456);
        assert_eq!(wire["metadata"]["freshness"]["stale_after_ms"], 128_456);
        assert_eq!(wire["metadata"]["window_id"], 42);
        assert!(wire.get("hardware_id").is_none());
        assert!(wire.get("first_seen_at").is_none());
    }

    #[test]
    fn remote_target_list_entry_requires_host_device_ura() {
        let mut entry = resource_entry();
        entry.kind = ResourceType::Window;
        entry.metadata = json!({
            "availability": "available",
            "observed_at_ms": 123_456,
            "freshness_ttl_ms": 5_000,
            "freshness": {
                "observed_at_ms": 123_456,
                "stale_after_ms": 128_456,
                "source": "live_refresh",
            },
        });

        let error = RemoteTargetListEntry::from_resource_entry(&entry)
            .expect_err("remote target projection must fail without host Device URA");

        assert!(
            error.to_string().contains("host_device_ura"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn remote_target_list_entry_requires_picker_freshness_contract() {
        let mut entry = resource_entry();
        entry.kind = ResourceType::Window;
        entry.metadata = json!({
            "host_device_ura": "easynet:///r/acme/device/dev-1",
            "availability": "available",
            "observed_at_ms": 123_456,
            "freshness_ttl_ms": 5_000,
        });

        let error = RemoteTargetListEntry::from_resource_entry(&entry)
            .expect_err("remote target projection must fail without freshness contract");

        assert!(
            error.to_string().contains("freshness"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn resource_list_entry_rejects_unknown_fields() {
        let error = serde_json::from_value::<ResourceListEntry>(json!({
            "resource_ura": "easynet:///r/acme/resource/device.dev-1/streams/camera.front",
            "owner_agent": "easynet:///r/acme/agent/device.dev-1.media",
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

    #[test]
    fn pages_project_list_response_preserves_public_shape() {
        let response = PagesProjectListResponse::from_projects(vec![PagesProjectListItem::new(
            "alice",
            "docs",
            "/srv/docs",
            "public",
            123,
            "https://example/web/alice/docs/",
            "http://docs.alice.pages.localhost:8787/",
        )]);
        let wire = serde_json::to_value(&response).expect("pages list response serializes");

        assert_eq!(wire["projects"][0]["user"], "alice");
        assert_eq!(wire["projects"][0]["project_id"], "docs");
        assert_eq!(wire["projects"][0]["folder"], "/srv/docs");
        assert_eq!(wire["projects"][0]["visibility"], "public");
        assert_eq!(wire["projects"][0]["started_at_ms"], 123);
        assert_eq!(
            wire["projects"][0]["url_root"],
            "https://example/web/alice/docs/"
        );
        assert_eq!(
            wire["projects"][0]["dev_listener_url_root"],
            "http://docs.alice.pages.localhost:8787/"
        );
    }

    #[test]
    fn pages_project_detail_response_preserves_public_shape() {
        let response = PagesProjectDetailResponse::success(
            "alice",
            "docs",
            "easynet:///r/example/resource/alice.docs",
            "/srv/docs",
            "public",
            123,
            "https://example/web/alice/docs/",
            "http://docs.alice.pages.localhost:8787/",
            1048576,
        );
        let wire = serde_json::to_value(&response).expect("pages detail response serializes");

        assert_eq!(wire["user"], "alice");
        assert_eq!(wire["project_id"], "docs");
        assert_eq!(
            wire["project_ura"],
            "easynet:///r/example/resource/alice.docs"
        );
        assert_eq!(wire["folder"], "/srv/docs");
        assert_eq!(wire["visibility"], "public");
        assert_eq!(wire["started_at_ms"], 123);
        assert_eq!(wire["url_root"], "https://example/web/alice/docs/");
        assert_eq!(
            wire["dev_listener_url_root"],
            "http://docs.alice.pages.localhost:8787/"
        );
        assert_eq!(wire["file_size_cap"], 1048576);
    }

    #[test]
    fn pages_unpublish_response_preserves_public_shape() {
        let response = PagesUnpublishResponse::success("alice", "docs");
        let wire = serde_json::to_value(&response).expect("pages unpublish response serializes");

        assert_eq!(wire["user"], "alice");
        assert_eq!(wire["project_id"], "docs");
        assert_eq!(wire["removed"], true);
    }

    #[test]
    fn pages_management_response_dtos_reject_unknown_fields() {
        let list_error = serde_json::from_value::<PagesProjectListResponse>(json!({
            "projects": [],
            "legacy_projects": []
        }))
        .expect_err("pages list response must reject legacy envelopes");
        assert!(
            list_error.to_string().contains("legacy_projects"),
            "strict pages list response error should name unknown field: {list_error}"
        );

        let detail_error = serde_json::from_value::<PagesProjectDetailResponse>(json!({
            "user": "alice",
            "project_id": "docs",
            "project_ura": "easynet:///r/example/resource/alice.docs",
            "folder": "/srv/docs",
            "visibility": "public",
            "started_at_ms": 123,
            "url_root": "https://example/web/alice/docs/",
            "dev_listener_url_root": "http://docs.alice.pages.localhost:8787/",
            "file_size_cap": 1048576,
            "local_fd": 7
        }))
        .expect_err("pages detail response must reject local fd leaks");
        assert!(
            detail_error.to_string().contains("local_fd"),
            "strict pages detail response error should name unknown field: {detail_error}"
        );

        let unpublish_error = serde_json::from_value::<PagesUnpublishResponse>(json!({
            "user": "alice",
            "project_id": "docs",
            "removed": true,
            "registry_path": "/tmp/pages-published-alice.json"
        }))
        .expect_err("pages unpublish response must reject registry path leaks");
        assert!(
            unpublish_error.to_string().contains("registry_path"),
            "strict pages unpublish response error should name unknown field: {unpublish_error}"
        );
    }

    #[test]
    fn pages_health_response_preserves_public_shape() {
        let response = PagesHealthResponse::new(
            "ready",
            true,
            "easynet:///r/example/agent/alice.pages",
            "easynet:///r/example/resource/alice.pages",
            1,
            vec![
                PagesHealthCheck::new(
                    "pages_registry",
                    "ready",
                    true,
                    None,
                    json!({"source": "PUBLISHED_PROJECTS"}),
                ),
                PagesHealthCheck::new(
                    "project",
                    "ready",
                    true,
                    None,
                    json!({"project_id": "docs", "requested": true}),
                ),
            ],
        );
        let wire = serde_json::to_value(&response).expect("pages health response serializes");

        assert_eq!(wire["state"], "ready");
        assert_eq!(wire["ready"], true);
        assert_eq!(wire["owner_ura"], "easynet:///r/example/agent/alice.pages");
        assert_eq!(
            wire["surface_ref"],
            "easynet:///r/example/resource/alice.pages"
        );
        assert_eq!(wire["page_count"], 1);
        assert_eq!(wire["checks"][0]["name"], "pages_registry");
        assert_eq!(wire["checks"][0]["message"], Value::Null);
        assert_eq!(wire["checks"][0]["latency_ms"], 0);
        assert_eq!(
            wire["checks"][0]["metadata"]["source"],
            "PUBLISHED_PROJECTS"
        );
        assert_eq!(wire["checks"][1]["metadata"]["project_id"], "docs");
        assert_eq!(wire["checks"][1]["metadata"]["requested"], true);
    }

    #[test]
    fn pages_health_response_dtos_reject_unknown_fields() {
        let check_error = serde_json::from_value::<PagesHealthCheck>(json!({
            "name": "project",
            "state": "missing",
            "ready": false,
            "message": "project is not published",
            "latency_ms": 0,
            "metadata": {},
            "registry_path": "/tmp/pages-published-alice.json"
        }))
        .expect_err("pages health check must reject registry path leaks");
        assert!(
            check_error.to_string().contains("registry_path"),
            "strict pages health check error should name unknown field: {check_error}"
        );

        let response_error = serde_json::from_value::<PagesHealthResponse>(json!({
            "state": "ready",
            "ready": true,
            "owner_ura": "easynet:///r/example/agent/alice.pages",
            "surface_ref": "easynet:///r/example/resource/alice.pages",
            "page_count": 1,
            "checks": [],
            "legacy_checks": []
        }))
        .expect_err("pages health response must reject legacy check envelopes");
        assert!(
            response_error.to_string().contains("legacy_checks"),
            "strict pages health response error should name unknown field: {response_error}"
        );
    }

    #[test]
    fn pages_fetch_response_preserves_public_shape() {
        let response = PagesFetchResponse::success(
            "PGgxPkhlbGxvPC9oMT4=",
            "text/html; charset=utf-8",
            14,
            false,
            "abc123",
        );
        let wire = serde_json::to_value(&response).expect("pages fetch response serializes");

        assert_eq!(wire["bytes_b64"], "PGgxPkhlbGxvPC9oMT4=");
        assert_eq!(wire["content_type"], "text/html; charset=utf-8");
        assert_eq!(wire["size_bytes"], 14);
        assert_eq!(wire["force_attachment"], false);
        assert_eq!(wire["sha256"], "abc123");
    }

    #[test]
    fn pages_fetch_response_rejects_unknown_fields() {
        let error = serde_json::from_value::<PagesFetchResponse>(json!({
            "bytes_b64": "PGgxPkhlbGxvPC9oMT4=",
            "content_type": "text/html; charset=utf-8",
            "size_bytes": 14,
            "force_attachment": false,
            "sha256": "abc123",
            "local_path": "/tmp/index.html"
        }))
        .expect_err("pages fetch response must reject local path leaks");

        assert!(
            error.to_string().contains("local_path"),
            "strict pages fetch response error should name unknown field: {error}"
        );
    }

    #[test]
    fn pages_publish_response_preserves_public_shape() {
        let response = PagesPublishResponse::success(
            "easynet:///r/example/resource/alice.docs",
            "https://example/web/alice/docs/",
            "alice",
            "docs",
            "public",
        );
        let wire = serde_json::to_value(&response).expect("pages publish response serializes");

        assert_eq!(
            wire["project_ura"],
            "easynet:///r/example/resource/alice.docs"
        );
        assert_eq!(wire["url_root"], "https://example/web/alice/docs/");
        assert_eq!(wire["user"], "alice");
        assert_eq!(wire["project_id"], "docs");
        assert_eq!(wire["visibility"], "public");
    }

    #[test]
    fn pages_publish_response_rejects_unknown_fields() {
        let error = serde_json::from_value::<PagesPublishResponse>(json!({
            "project_ura": "easynet:///r/example/resource/alice.docs",
            "url_root": "https://example/web/alice/docs/",
            "user": "alice",
            "project_id": "docs",
            "visibility": "public",
            "canonical_root": "/tmp/site"
        }))
        .expect_err("pages publish response must reject local path leaks");

        assert!(
            error.to_string().contains("canonical_root"),
            "strict pages publish response error should name unknown field: {error}"
        );
    }

    #[test]
    fn pages_api_response_preserves_public_shape() {
        let response = PagesApiResponse::json_ok(json!({"pong": true}));
        let wire = serde_json::to_value(&response).expect("pages api response serializes");

        assert_eq!(wire["status"], 200);
        assert_eq!(wire["body"]["pong"], true);
        assert_eq!(wire["content_type"], "application/json; charset=utf-8");
    }

    #[test]
    fn pages_api_response_rejects_unknown_fields() {
        let error = serde_json::from_value::<PagesApiResponse>(json!({
            "status": 200,
            "body": {"pong": true},
            "content_type": "application/json; charset=utf-8",
            "manifest_path": "/tmp/site/api/ping.toml"
        }))
        .expect_err("pages api response must reject manifest path leaks");

        assert!(
            error.to_string().contains("manifest_path"),
            "strict pages api response error should name unknown field: {error}"
        );
    }
}

// EasyNet CLI — skill record response projection
// =================================================
//
// File: src/daemon/resources/skills/projection.rs
// Description: Public daemon/CLI view over strict skill install records.
//
// Protocol Responsibility:
//   Keeps skill ability responses separate from `.easynet/install.json`
//   persistence. Response-only fields such as `resource_ura` belong here,
//   never in the store record.
//
// Implementation Approach:
//   Wraps the canonical `InstallRecord` fields in a strict serializable
//   projection. The projection intentionally preserves the public
//   `content_hash` wire field while exposing the Rust-side semantic name
//   `skill_tree_hash`.
//
// Usage Contract:
//   Daemon skill abilities return `InstalledSkillProjection`; CLI skill
//   commands decode `InstalledSkillProjection`. Store helpers remain the only
//   code that reads or writes `InstallRecord`.
//
// Architectural Position:
//   Resource-domain response projection. This is not an SDK model and not a
//   product lifecycle abstraction.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::store::{InstallRecord, SkillSource};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledSkillProjection {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub agent_id: String,
    pub source: SkillSource,
    #[serde(rename = "content_hash")]
    pub skill_tree_hash: String,
    pub size_bytes: u64,
    pub installed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
    #[serde(default)]
    pub upgrade_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_ura: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRecordResponse {
    pub ok: bool,
    pub record: InstalledSkillProjection,
}

impl SkillRecordResponse {
    pub fn ok(record: InstalledSkillProjection) -> Self {
        Self { ok: true, record }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillListResponse {
    #[serde(default)]
    pub items: Vec<InstalledSkillProjection>,
}

impl SkillListResponse {
    pub fn from_items(items: Vec<InstalledSkillProjection>) -> Self {
        Self { items }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRemoveReceipt {
    pub ok: bool,
    pub name: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_ura: Option<String>,
}

impl SkillRemoveReceipt {
    pub fn success(
        name: impl Into<String>,
        agent: impl Into<String>,
        resource_ura: Option<String>,
    ) -> Self {
        Self {
            ok: true,
            name: name.into(),
            agent: agent.into(),
            resource_ura,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillPublishReceipt {
    pub ok: bool,
    pub owner_agent_id: String,
    pub skill_name: String,
    pub skill_dir: String,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_run_id: Option<String>,
}

impl SkillPublishReceipt {
    pub fn success(
        owner_agent_id: impl Into<String>,
        skill_name: impl Into<String>,
        skill_dir: impl Into<String>,
        content_hash: impl Into<String>,
        mission_run_id: Option<String>,
    ) -> Self {
        Self {
            ok: true,
            owner_agent_id: owner_agent_id.into(),
            skill_name: skill_name.into(),
            skill_dir: skill_dir.into(),
            content_hash: content_hash.into(),
            mission_run_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillUnpublishReceipt {
    pub ok: bool,
    pub owner_agent_id: String,
    pub skill_name: String,
    pub removed_dir: String,
    pub content_hash: String,
}

impl SkillUnpublishReceipt {
    pub fn success(
        owner_agent_id: impl Into<String>,
        skill_name: impl Into<String>,
        removed_dir: impl Into<String>,
        content_hash: impl Into<String>,
    ) -> Self {
        Self {
            ok: true,
            owner_agent_id: owner_agent_id.into(),
            skill_name: skill_name.into(),
            removed_dir: removed_dir.into(),
            content_hash: content_hash.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillTreeResponse {
    pub ok: bool,
    pub owner_agent_id: String,
    pub skill_name: String,
    pub root: String,
    pub files: Vec<Value>,
    pub resource_ura: String,
}

impl SkillTreeResponse {
    pub fn success(
        owner_agent_id: impl Into<String>,
        skill_name: impl Into<String>,
        root: impl Into<String>,
        files: Vec<Value>,
        resource_ura: impl Into<String>,
    ) -> Self {
        Self {
            ok: true,
            owner_agent_id: owner_agent_id.into(),
            skill_name: skill_name.into(),
            root: root.into(),
            files,
            resource_ura: resource_ura.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillReadFileResponse {
    pub ok: bool,
    pub owner_agent_id: String,
    pub skill_name: String,
    pub path: String,
    pub content: String,
    pub encoding: String,
    pub size_bytes: u64,
    pub resource_ura: String,
}

impl SkillReadFileResponse {
    pub fn success(
        owner_agent_id: impl Into<String>,
        skill_name: impl Into<String>,
        path: impl Into<String>,
        content: impl Into<String>,
        size_bytes: u64,
        resource_ura: impl Into<String>,
    ) -> Self {
        Self {
            ok: true,
            owner_agent_id: owner_agent_id.into(),
            skill_name: skill_name.into(),
            path: path.into(),
            content: content.into(),
            encoding: "utf-8".to_string(),
            size_bytes,
            resource_ura: resource_ura.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillWriteFileReceipt {
    pub ok: bool,
    pub owner_agent_id: String,
    pub skill_name: String,
    pub path: String,
    pub size_bytes: u64,
    pub content_hash: String,
    pub resource_ura: String,
}

impl SkillWriteFileReceipt {
    pub fn success(
        owner_agent_id: impl Into<String>,
        skill_name: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
        content_hash: impl Into<String>,
        resource_ura: impl Into<String>,
    ) -> Self {
        Self {
            ok: true,
            owner_agent_id: owner_agent_id.into(),
            skill_name: skill_name.into(),
            path: path.into(),
            size_bytes,
            content_hash: content_hash.into(),
            resource_ura: resource_ura.into(),
        }
    }
}

impl InstalledSkillProjection {
    pub fn from_record(record: InstallRecord, resource_ura: Option<String>) -> Self {
        Self {
            name: record.name,
            description: record.description,
            agent_id: record.agent_id,
            source: record.source,
            skill_tree_hash: record.skill_tree_hash,
            size_bytes: record.size_bytes,
            installed_at: record.installed_at,
            last_checked_at: record.last_checked_at,
            upgrade_available: record.upgrade_available,
            resource_ura,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> InstallRecord {
        InstallRecord {
            name: "alpha".to_string(),
            description: "Alpha skill".to_string(),
            agent_id: "alice".to_string(),
            source: SkillSource {
                kind: "github".to_string(),
                identifier: "a/b".to_string(),
                ref_: Some("main".to_string()),
                subpath: None,
            },
            skill_tree_hash: "sha256:wire".to_string(),
            size_bytes: 99,
            installed_at: "2026-04-23T00:00:00Z".to_string(),
            last_checked_at: None,
            upgrade_available: false,
        }
    }

    #[test]
    fn installed_skill_projection_owns_resource_ura_without_mutating_install_record_schema() {
        let projection = InstalledSkillProjection::from_record(
            record(),
            Some("easynet:///r/acme/resource/agent.u.alice/skill/alpha".to_string()),
        );
        let wire = serde_json::to_value(&projection).expect("projection serializes");

        assert_eq!(wire["content_hash"], "sha256:wire");
        assert!(wire.get("skill_tree_hash").is_none());
        assert_eq!(
            wire["resource_ura"],
            "easynet:///r/acme/resource/agent.u.alice/skill/alpha"
        );
    }

    #[test]
    fn installed_skill_projection_rejects_unknown_response_fields() {
        let wire = r#"{
            "name": "alpha",
            "description": "Alpha skill",
            "agent_id": "alice",
            "source": {
                "kind": "github",
                "identifier": "a/b",
                "ref": "main"
            },
            "content_hash": "sha256:wire",
            "size_bytes": 99,
            "installed_at": "2026-04-23T00:00:00Z",
            "upgrade_available": false,
            "legacy_resource_ref": "retired"
        }"#;
        let error = serde_json::from_str::<InstalledSkillProjection>(wire)
            .expect_err("unknown response fields must fail closed");
        assert!(
            error.to_string().contains("legacy_resource_ref"),
            "strict projection error should name the unknown field: {error}"
        );
    }

    #[test]
    fn skill_record_response_preserves_public_envelope_shape() {
        let response =
            SkillRecordResponse::ok(InstalledSkillProjection::from_record(record(), None));
        let wire = serde_json::to_value(&response).expect("response serializes");

        assert_eq!(wire["ok"], true);
        assert_eq!(wire["record"]["name"], "alpha");
        assert_eq!(wire["record"]["content_hash"], "sha256:wire");
    }

    #[test]
    fn skill_list_response_preserves_items_shape() {
        let response = SkillListResponse::from_items(vec![InstalledSkillProjection::from_record(
            record(),
            None,
        )]);
        let wire = serde_json::to_value(&response).expect("response serializes");

        assert_eq!(wire["items"][0]["name"], "alpha");
        assert_eq!(wire["items"][0]["content_hash"], "sha256:wire");
    }

    #[test]
    fn skill_remove_receipt_preserves_public_shape_and_rejects_unknown_fields() {
        let receipt = SkillRemoveReceipt::success(
            "alpha",
            "alice",
            Some("easynet:///r/acme/resource/agent.u.alice/skill/alpha".to_string()),
        );
        let wire = serde_json::to_value(&receipt).expect("receipt serializes");

        assert_eq!(wire["ok"], true);
        assert_eq!(wire["name"], "alpha");
        assert_eq!(wire["agent"], "alice");
        assert_eq!(
            wire["resource_ura"],
            "easynet:///r/acme/resource/agent.u.alice/skill/alpha"
        );

        let error = serde_json::from_value::<SkillRemoveReceipt>(serde_json::json!({
            "ok": true,
            "name": "alpha",
            "agent": "alice",
            "legacy_removed": true
        }))
        .expect_err("unknown receipt fields must fail closed");
        assert!(
            error.to_string().contains("legacy_removed"),
            "strict receipt error should name unknown field: {error}"
        );
    }

    #[test]
    fn skill_publish_and_unpublish_receipts_preserve_public_shapes() {
        let publish = SkillPublishReceipt::success(
            "claude",
            "alpha",
            "/tmp/alpha",
            "sha256:abc",
            Some("run-1".to_string()),
        );
        let publish_wire = serde_json::to_value(&publish).expect("publish receipt");
        assert_eq!(publish_wire["ok"], true);
        assert_eq!(publish_wire["owner_agent_id"], "claude");
        assert_eq!(publish_wire["skill_name"], "alpha");
        assert_eq!(publish_wire["skill_dir"], "/tmp/alpha");
        assert_eq!(publish_wire["content_hash"], "sha256:abc");
        assert_eq!(publish_wire["mission_run_id"], "run-1");

        let unpublish =
            SkillUnpublishReceipt::success("claude", "alpha", "/tmp/alpha", "sha256:abc");
        let unpublish_wire = serde_json::to_value(&unpublish).expect("unpublish receipt");
        assert_eq!(unpublish_wire["ok"], true);
        assert_eq!(unpublish_wire["removed_dir"], "/tmp/alpha");
        assert_eq!(unpublish_wire["content_hash"], "sha256:abc");
    }

    #[test]
    fn skill_file_operation_responses_preserve_public_shapes() {
        let resource = "easynet:///r/acme/resource/agent.u.alice/skill/alpha/SKILL.md";
        let tree = SkillTreeResponse::success(
            "alice",
            "alpha",
            "/tmp/alpha",
            vec![serde_json::json!({"path": "SKILL.md", "kind": "file"})],
            resource,
        );
        let tree_wire = serde_json::to_value(&tree).expect("tree response");
        assert_eq!(tree_wire["files"][0]["path"], "SKILL.md");
        assert_eq!(tree_wire["resource_ura"], resource);

        let read =
            SkillReadFileResponse::success("alice", "alpha", "SKILL.md", "body", 4, resource);
        let read_wire = serde_json::to_value(&read).expect("read response");
        assert_eq!(read_wire["encoding"], "utf-8");
        assert_eq!(read_wire["content"], "body");
        assert_eq!(read_wire["size_bytes"], 4);

        let write =
            SkillWriteFileReceipt::success("alice", "alpha", "SKILL.md", 4, "sha256:def", resource);
        let write_wire = serde_json::to_value(&write).expect("write receipt");
        assert_eq!(write_wire["content_hash"], "sha256:def");
        assert_eq!(write_wire["resource_ura"], resource);
    }
}

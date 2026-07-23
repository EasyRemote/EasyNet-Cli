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
}

// EasyNet CLI — Ability Catalogue Row
// ===================================
//
// File: src/cli/ability_catalog_row.rs
// Description: Presentation-layer projection for ability catalogue rows.
//
// Protocol Responsibility
// -----------------------
// Treat `ability_ura` as the only identity-bearing field. Human labels
// may use owner-local `name` / `public_name`. Non-canonical row fields
// fail at the DTO boundary so stale read-model rows cannot be rendered as
// canonical catalogue state.
//
// Implementation Approach
// -----------------------
// Parse through a presentation-local DTO with `deny_unknown_fields`, then
// through Axon's URA helpers. The CLI does not reconstruct or infer owners
// from dotted names.
//
// Usage Contract
// --------------
// Consumers may render `label()` for humans and `ability_ura()` for
// routing/correlation. Missing labels fall back to the URA itself;
// missing URAs remain empty and should be shown as `-`.
//
// Architectural Position
// ----------------------
// CLI facade only. Runtime catalogue generation lives under
// `daemon::ability::builtins::governance::meta` / owner projections.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AbilityCatalogueRow {
    label: String,
    ability_ura: Option<String>,
    owner_ura: Option<String>,
}

impl AbilityCatalogueRow {
    pub(crate) fn from_value(value: &Value) -> anyhow::Result<Self> {
        let row = AbilityCatalogueRowWire::from_value(value)?;
        let ability_ura = row.ability_ura;
        let owner_ura = row
            .owner_ura
            .or_else(|| ability_ura.as_deref().and_then(owner_ura_from_ability_ura));
        let label = row
            .public_name
            .or(row.name)
            .or_else(|| ability_ura.clone())
            .unwrap_or_else(|| "-".to_string());
        Ok(Self {
            label,
            ability_ura,
            owner_ura,
        })
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn ability_ura(&self) -> Option<&str> {
        self.ability_ura.as_deref()
    }

    pub(crate) fn owner_ura(&self) -> Option<&str> {
        self.owner_ura.as_deref()
    }
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AbilityCatalogueRowWire {
    #[serde(default, deserialize_with = "optional_trimmed_string")]
    name: Option<String>,
    #[serde(default, deserialize_with = "optional_trimmed_string")]
    public_name: Option<String>,
    #[serde(default, deserialize_with = "optional_trimmed_string")]
    ability_ura: Option<String>,
    #[serde(default, deserialize_with = "optional_trimmed_string")]
    owner_ura: Option<String>,
    #[serde(default)]
    version: Option<Value>,
    #[serde(default)]
    descriptor_version: Option<Value>,
    #[serde(default)]
    schema_hash: Option<Value>,
    #[serde(default)]
    descriptor_hash: Option<Value>,
    #[serde(default)]
    descriptor_ref: Option<Value>,
    #[serde(default)]
    call_mode: Option<Value>,
    #[serde(default)]
    admission_action: Option<Value>,
    #[serde(default)]
    receipt_semantics: Option<Value>,
    #[serde(default)]
    visibility: Option<Value>,
    #[serde(default)]
    scope_subjects: Option<Value>,
    #[serde(default)]
    scope_agents: Option<Value>,
    #[serde(default)]
    denied_agents: Option<Value>,
    #[serde(default)]
    description: Option<Value>,
    #[serde(default)]
    source: Option<Value>,
    #[serde(default)]
    schema_summary: Option<Value>,
    #[serde(default)]
    hints: Option<Value>,
    #[serde(default)]
    metadata: Option<Value>,
}

impl AbilityCatalogueRowWire {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        serde_json::from_value(value.clone())
            .map_err(|error| anyhow::anyhow!("ability catalogue row is not canonical: {error}"))
    }
}

fn optional_trimmed_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(|value| {
        value
            .map(|raw| raw.trim().to_string())
            .filter(|trimmed| !trimmed.is_empty())
    })
}

fn owner_ura_from_ability_ura(ability_ura: &str) -> Option<String> {
    let parsed = crate::core::ura::parse_ura(ability_ura).ok()?;
    if parsed.kind != crate::core::ura::URAKind::Ability {
        return None;
    }
    let ability = parsed.ability()?;
    match ability.owner {
        crate::core::ura::AbilityOwner::Authority => Some(crate::core::ura::hub_ura(&parsed.realm)),
        crate::core::ura::AbilityOwner::Agent { user_id, agent_id } => Some(
            crate::core::ura::agent_ura(&parsed.realm, &user_id, &agent_id),
        ),
        crate::core::ura::AbilityOwner::Device { device_id } => {
            Some(crate::core::ura::device_ura(&parsed.realm, &device_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn projection_uses_ability_ura_for_identity_and_owner() {
        let row = AbilityCatalogueRow::from_value(&json!({
            "name": "chat",
            "ability_ura": "easynet:///r/acme/ability/alice.bot.chat"
        }))
        .expect("canonical row should project");

        assert_eq!(row.label(), "chat");
        assert_eq!(
            row.ability_ura(),
            Some("easynet:///r/acme/ability/alice.bot.chat")
        );
        assert_eq!(row.owner_ura(), Some("easynet:///r/acme/agent/alice.bot"));
    }

    #[test]
    fn projection_derives_label_and_owner_from_ability_ura() {
        let row = AbilityCatalogueRow::from_value(&json!({
            "ability_ura": "easynet:///r/acme/ability/device.dev-1.fs.read"
        }))
        .expect("canonical row should project");

        assert_eq!(
            row.label(),
            "easynet:///r/acme/ability/device.dev-1.fs.read"
        );
        assert_eq!(row.owner_ura(), Some("easynet:///r/acme/device/dev-1"));
    }

    #[test]
    fn projection_rejects_non_canonical_catalogue_alias_fields() {
        let error = AbilityCatalogueRow::from_value(&json!({
            "ability_name": "legacy.name",
            "tool_name": "legacy.tool",
            "ability_ura": "easynet:///r/acme/ability/device.dev-1.fs.read"
        }))
        .expect_err("non-canonical aliases must fail closed");

        let message = error.to_string();
        assert!(message.contains("ability_name"), "{message}");
        assert!(
            message.contains("unknown field") || message.contains("tool_name"),
            "{message}"
        );
    }
}

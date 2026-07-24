// EasyNet CLI — Ability Catalogue Row
// ===================================
//
// File: src/cli/ability_catalog_row.rs
// Description: Presentation-layer projection for ability catalogue rows.
//
// Protocol Responsibility
// -----------------------
// Treat `ability_ura` as the only identity-bearing field. Human labels
// may use owner-local `name` / `public_name`. Retired alias fields such
// as `ability_name` and `tool_name` are rejected so stale read-model rows
// cannot be rendered as canonical catalogue state.
//
// Implementation Approach
// -----------------------
// Parse only through Axon's URA helpers. The CLI does not reconstruct
// or infer owners from dotted names.
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

use serde_json::Value;

const RETIRED_CATALOGUE_FIELDS: &[&str] = &["ability_name", "tool_name"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AbilityCatalogueRow {
    label: String,
    ability_ura: Option<String>,
    owner_ura: Option<String>,
}

impl AbilityCatalogueRow {
    pub(crate) fn from_value(value: &Value) -> anyhow::Result<Self> {
        reject_retired_catalogue_fields(value)?;

        let ability_ura = string_field(value, "ability_ura");
        let owner_ura = string_field(value, "owner_ura")
            .or_else(|| ability_ura.as_deref().and_then(owner_ura_from_ability_ura));
        let label = string_field(value, "public_name")
            .or_else(|| string_field(value, "name"))
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

fn reject_retired_catalogue_fields(value: &Value) -> anyhow::Result<()> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    let retired = RETIRED_CATALOGUE_FIELDS
        .iter()
        .copied()
        .filter(|field| object.contains_key(*field))
        .collect::<Vec<_>>();
    if retired.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "ability catalogue row contains retired field(s): {}",
        retired.join(", ")
    )
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
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
    fn projection_rejects_retired_ability_name_and_tool_name_fields() {
        let error = AbilityCatalogueRow::from_value(&json!({
            "ability_name": "legacy.name",
            "tool_name": "legacy.tool",
            "ability_ura": "easynet:///r/acme/ability/device.dev-1.fs.read"
        }))
        .expect_err("retired aliases must fail closed");

        let message = error.to_string();
        assert!(message.contains("ability_name"), "{message}");
        assert!(message.contains("tool_name"), "{message}");
    }
}

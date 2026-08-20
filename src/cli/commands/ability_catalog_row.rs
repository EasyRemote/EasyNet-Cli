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
// Parse through the daemon-owned canonical catalogue row contract. The CLI
// does not reconstruct or infer owners from dotted names.
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

use crate::daemon::ability::AbilityCatalogRow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AbilityCatalogueRow {
    label: String,
    ability_ura: Option<String>,
    owner_ura: Option<String>,
}

impl AbilityCatalogueRow {
    pub(crate) fn from_value(value: &Value) -> anyhow::Result<Self> {
        let row = AbilityCatalogRow::parse(value, 0, "CLI catalogue projection")
            .map_err(anyhow::Error::msg)?;
        let descriptor = row.descriptor();
        let ability_ura = descriptor.canonical_ability_ura();
        let owner_ura = Some(descriptor.owner_ura.clone());
        let label = descriptor.public_name();
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(name: &str, owner_ura: &str) -> Value {
        AbilityCatalogRow::from_descriptor(
            crate::daemon::ability::AbilityDescriptor::new(
                name,
                owner_ura,
                crate::daemon::ability::descriptors::Visibility::Public,
                crate::daemon::ability::descriptors::AdmissionAction::Read,
            )
            .expect("test descriptor"),
        )
        .expect("test catalog row")
        .into_value()
    }

    #[test]
    fn projection_uses_ability_ura_for_identity_and_owner() {
        let row =
            AbilityCatalogueRow::from_value(&row("chat", "easynet:///r/acme/agent/alice.bot"))
                .expect("canonical row should project");

        assert_eq!(row.label(), "chat");
        assert_eq!(
            row.ability_ura(),
            Some("easynet:///r/acme/ability/alice.bot.chat")
        );
        assert_eq!(row.owner_ura(), Some("easynet:///r/acme/agent/alice.bot"));
    }

    #[test]
    fn projection_uses_canonical_system_agent_owner() {
        let owner = "easynet:///r/acme/agent/device.dev-1.locomotion";
        let row = AbilityCatalogueRow::from_value(&row("fs.read", owner))
            .expect("canonical row should project");

        assert_eq!(row.label(), "fs.read");
        assert_eq!(row.owner_ura(), Some(owner));
    }

    #[test]
    fn projection_rejects_non_canonical_catalogue_alias_fields() {
        let mut value = row("fs.read", "easynet:///r/acme/agent/device.dev-1.locomotion");
        value["ability_name"] = json!("legacy.name");
        value["tool_name"] = json!("legacy.tool");
        let error = AbilityCatalogueRow::from_value(&value)
            .expect_err("non-canonical aliases must fail closed");

        let message = error.to_string();
        assert!(
            message.contains("ability_name") || message.contains("tool_name"),
            "{message}"
        );
    }
}

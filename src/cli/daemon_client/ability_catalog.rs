// EasyNet CLI — Ability catalogue client
// ======================================
//
// Centralizes the CLI facade path for reading `meta.list_abilities`.
// `ability list`, `ability show`, and future SDK-facing CLI helpers should
// route through this object instead of hand-writing local/remote catalogue
// dispatch. The invariant is deliberately narrow: one catalogue request per
// target. Cross-device aggregate fan-out belongs in a named daemon ability, not
// in a facade loop.

use serde_json::Value;

use crate::cli::daemon_client::remote_system_ability::invoke_remote_device_catalogue_read;
use crate::daemon::ability::{AbilityCatalogQuery, AbilityCatalogRow};
use crate::support::platform::local_invoke::LocalRuntimeCatalogueReadIssuer;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AbilityCatalogueQuery {
    inner: AbilityCatalogQuery,
}

impl AbilityCatalogueQuery {
    pub(crate) fn new(owner_ura: Option<String>, ability_ura: Option<String>) -> Self {
        Self {
            inner: AbilityCatalogQuery::new(owner_ura, ability_ura),
        }
    }

    pub(crate) fn owner_ura(&self) -> Option<&str> {
        self.inner.owner_ura()
    }

    pub(crate) fn ability_ura(&self) -> Option<&str> {
        self.inner.ability_ura()
    }

    pub(crate) fn to_request(&self) -> Value {
        self.inner.to_request_json()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AbilityCatalogueClient {
    query: AbilityCatalogueQuery,
}

impl AbilityCatalogueClient {
    pub(crate) fn new(query: AbilityCatalogueQuery) -> Self {
        Self { query }
    }

    pub(crate) fn fetch_local_value(&self) -> anyhow::Result<Value> {
        LocalRuntimeCatalogueReadIssuer::list_abilities(self.query.to_request())
    }

    pub(crate) fn fetch_local_abilities(&self) -> anyhow::Result<Vec<Value>> {
        let value = self.fetch_local_value()?;
        Self::abilities_from_value(&value)
    }

    pub(crate) fn fetch_remote_value(
        &self,
        node: &str,
        action_label: &str,
    ) -> anyhow::Result<Value> {
        invoke_remote_catalogue(node, self.query.to_request(), action_label)
    }

    pub(crate) fn fetch_remote_abilities(
        &self,
        node: &str,
        action_label: &str,
    ) -> anyhow::Result<Vec<Value>> {
        let value = self.fetch_remote_value(node, action_label)?;
        Self::abilities_from_value(&value)
    }

    pub(crate) fn abilities_from_value(value: &Value) -> anyhow::Result<Vec<Value>> {
        let entries = value
            .get("abilities")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow::anyhow!("meta.list_abilities response missing abilities array")
            })?;
        entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                AbilityCatalogRow::parse(entry, index, "CLI meta.list_abilities")
                    .map(AbilityCatalogRow::into_value)
                    .map_err(anyhow::Error::msg)
            })
            .collect()
    }
}

fn invoke_remote_catalogue(
    node: &str,
    request: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    invoke_remote_device_catalogue_read(node, request, action_label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_row(name: &str, owner_ura: &str) -> Value {
        AbilityCatalogRow::from_descriptor(
            crate::daemon::ability::AbilityDescriptor::new(
                name,
                owner_ura,
                crate::daemon::ability::descriptors::Visibility::Public,
                crate::daemon::ability::descriptors::AdmissionAction::Stream,
            )
            .expect("test descriptor"),
        )
        .expect("test catalog row")
        .into_value()
    }

    #[test]
    fn abilities_from_value_requires_descriptor_bound_ref() {
        let mut row = catalog_row("er.add", "easynet:///r/acme/device/dev");
        row.as_object_mut()
            .expect("catalog row object")
            .remove("descriptor_ref");
        let value = serde_json::json!({ "abilities": [row] });

        let err = AbilityCatalogueClient::abilities_from_value(&value)
            .expect_err("CLI catalogue must not synthesize descriptor_ref")
            .to_string();

        assert!(
            err.contains("missing required field \"descriptor_ref\""),
            "got {err}"
        );
    }

    #[test]
    fn abilities_from_value_preserves_daemon_descriptor_ref() {
        let row = catalog_row("er.add", "easynet:///r/acme/device/dev");
        let descriptor_ref = row["descriptor_ref"].as_str().unwrap().to_string();
        let value = serde_json::json!({ "abilities": [row] });

        let abilities = AbilityCatalogueClient::abilities_from_value(&value)
            .expect("descriptor-bound catalogue row");

        assert_eq!(abilities[0]["descriptor_ref"], descriptor_ref);
    }

    #[test]
    fn abilities_from_value_rejects_name_derived_owner_repair() {
        let mut row = catalog_row("er.add", "easynet:///r/acme/device/dev");
        row["owner_ura"] = Value::String("easynet:///r/acme/device/other".to_string());
        let value = serde_json::json!({ "abilities": [row] });

        let err = AbilityCatalogueClient::abilities_from_value(&value)
            .expect_err("owner must be catalogue-bound, not derived by renderer")
            .to_string();

        assert!(
            err.contains("wire ability_ura")
                && err.contains("does not match canonical ability_ura"),
            "got {err}"
        );
    }

    #[test]
    fn abilities_from_value_accepts_device_sponsored_agent_owner() {
        let row = catalog_row("search", "easynet:///r/acme/agent/device.dev-1.mcp-default");
        let value = serde_json::json!({ "abilities": [row] });

        let abilities = AbilityCatalogueClient::abilities_from_value(&value)
            .expect("device-sponsored Agent catalogue row must stay canonical");

        assert_eq!(
            abilities[0]["owner_ura"],
            value["abilities"][0]["owner_ura"]
        );
        assert_eq!(
            abilities[0]["ability_ura"],
            value["abilities"][0]["ability_ura"]
        );
    }

    #[test]
    fn abilities_from_value_rejects_missing_abilities_array() {
        let err = AbilityCatalogueClient::abilities_from_value(&serde_json::json!({}))
            .expect_err("missing array must fail closed")
            .to_string();
        assert!(err.contains("missing abilities array"), "got {err}");
    }
}

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

use crate::cli::daemon_client::remote_system_ability::invoke_remote_device_system_ability;
use crate::core::ura::{parse_ura, AbilitySelector, URAKind};
use crate::support::platform::local_invoke::invoke_local_ability;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AbilityCatalogueQuery {
    agent_ura: Option<String>,
    subject_ura: Option<String>,
}

impl AbilityCatalogueQuery {
    pub(crate) fn new(agent_ura: Option<String>, subject_ura: Option<String>) -> Self {
        Self {
            agent_ura,
            subject_ura,
        }
    }

    pub(crate) fn agent_ura(&self) -> Option<&str> {
        self.agent_ura.as_deref()
    }

    pub(crate) fn subject_ura(&self) -> Option<&str> {
        self.subject_ura.as_deref()
    }

    pub(crate) fn to_request(&self) -> Value {
        let mut body = serde_json::Map::new();
        if let Some(agent_ura) = self.agent_ura.as_ref() {
            body.insert("agent_ura".to_string(), Value::String(agent_ura.clone()));
        }
        if let Some(subject_ura) = self.subject_ura.as_ref() {
            body.insert(
                "subject_ura".to_string(),
                Value::String(subject_ura.clone()),
            );
        }
        Value::Object(body)
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
        invoke_local_ability("meta.list_abilities", self.query.to_request())
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
            .map(|(index, entry)| schema_bound_catalogue_entry(entry, index))
            .collect()
    }
}

fn schema_bound_catalogue_entry(entry: &Value, index: usize) -> anyhow::Result<Value> {
    let object = entry
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("meta.list_abilities row #{index} is not an object"))?;
    let ability_ura = required_catalogue_string(object, index, "ability_ura")?;
    let selector = AbilitySelector::parse(ability_ura).map_err(|error| {
        anyhow::anyhow!("meta.list_abilities row #{index} has invalid ability_ura: {error}")
    })?;
    let owner_ura = required_catalogue_string(object, index, "owner_ura")?;
    let owner = parse_ura(owner_ura).map_err(|error| {
        anyhow::anyhow!("meta.list_abilities row #{index} has invalid owner_ura: {error}")
    })?;
    if !matches!(
        owner.kind,
        URAKind::Agent | URAKind::Authority | URAKind::Device
    ) {
        anyhow::bail!(
            "meta.list_abilities row #{index} owner_ura must be an Agent, Device, or Authority URA"
        );
    }
    if owner_ura != selector.owner_ura() {
        anyhow::bail!(
            "meta.list_abilities row #{index} owner_ura {owner_ura:?} does not match \
             ability_ura owner {:?}",
            selector.owner_ura()
        );
    }
    let name = required_catalogue_string(object, index, "name")?;
    if name != selector.public_name() {
        anyhow::bail!(
            "meta.list_abilities row #{index} name {name:?} does not match ability_ura public \
             name {:?}",
            selector.public_name()
        );
    }
    let version = required_catalogue_string(object, index, "version")?;
    let descriptor_ref = object
        .get("descriptor_ref")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("meta.list_abilities row #{index} missing canonical descriptor_ref")
        })?;
    axon_sdk::invocation::canonical_ability_descriptor_ref(descriptor_ref).map_err(|error| {
        anyhow::anyhow!("meta.list_abilities row #{index} has invalid descriptor_ref: {error}")
    })?;
    let descriptor_ability_ura =
        axon_sdk::invocation::ability_ura_from_descriptor_ref(descriptor_ref).map_err(|error| {
            anyhow::anyhow!(
                "meta.list_abilities row #{index} descriptor_ref has invalid ability_ura: {error}"
            )
        })?;
    if descriptor_ability_ura != ability_ura {
        anyhow::bail!(
            "meta.list_abilities row #{index} descriptor_ref ability {descriptor_ability_ura:?} \
             does not match ability_ura {ability_ura:?}"
        );
    }
    if !descriptor_ref.starts_with(&format!("{ability_ura}@{version}#")) {
        anyhow::bail!(
            "meta.list_abilities row #{index} descriptor_ref does not bind version {version:?}"
        );
    }
    Ok(entry.clone())
}

fn required_catalogue_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    index: usize,
    field: &str,
) -> anyhow::Result<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("meta.list_abilities row #{index} missing {field}"))
}

fn invoke_remote_catalogue(
    node: &str,
    request: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    invoke_remote_device_system_ability(node, "meta.list_abilities", request, action_label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abilities_from_value_requires_descriptor_bound_ref() {
        let value = serde_json::json!({
            "abilities": [{
                "ability_ura": "easynet:///r/acme/ability/device.dev.er.add",
                "owner_ura": "easynet:///r/acme/device/dev",
                "name": "er.add",
                "version": "1.0.0",
                "descriptor_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "admission_action": "stream"
            }]
        });

        let err = AbilityCatalogueClient::abilities_from_value(&value)
            .expect_err("CLI catalogue must not synthesize descriptor_ref")
            .to_string();

        assert!(
            err.contains("missing canonical descriptor_ref"),
            "got {err}"
        );
    }

    #[test]
    fn abilities_from_value_preserves_daemon_descriptor_ref() {
        let descriptor_ref = "easynet:///r/acme/ability/device.dev.er.add@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!stream";
        let value = serde_json::json!({
            "abilities": [{
                "ability_ura": "easynet:///r/acme/ability/device.dev.er.add",
                "owner_ura": "easynet:///r/acme/device/dev",
                "name": "er.add",
                "version": "1.0.0",
                "descriptor_ref": descriptor_ref
            }]
        });

        let abilities = AbilityCatalogueClient::abilities_from_value(&value)
            .expect("descriptor-bound catalogue row");

        assert_eq!(abilities[0]["descriptor_ref"], descriptor_ref);
    }

    #[test]
    fn abilities_from_value_rejects_name_derived_owner_repair() {
        let descriptor_ref = "easynet:///r/acme/ability/device.dev.er.add@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!stream";
        let value = serde_json::json!({
            "abilities": [{
                "ability_ura": "easynet:///r/acme/ability/device.dev.er.add",
                "owner_ura": "easynet:///r/acme/device/other",
                "name": "er.add",
                "version": "1.0.0",
                "descriptor_ref": descriptor_ref
            }]
        });

        let err = AbilityCatalogueClient::abilities_from_value(&value)
            .expect_err("owner must be catalogue-bound, not derived by renderer")
            .to_string();

        assert!(
            err.contains("owner_ura") && err.contains("does not match"),
            "got {err}"
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

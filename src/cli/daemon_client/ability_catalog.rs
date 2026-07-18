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
        Ok(Self::abilities_from_value(&value))
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
        Ok(Self::abilities_from_value(&value))
    }

    pub(crate) fn abilities_from_value(value: &Value) -> Vec<Value> {
        value
            .get("abilities")
            .and_then(Value::as_array)
            .cloned()
            .map(|entries| entries.into_iter().map(enrich_descriptor_ref).collect())
            .unwrap_or_default()
    }
}

fn enrich_descriptor_ref(mut entry: Value) -> Value {
    let Some(object) = entry.as_object_mut() else {
        return entry;
    };
    if object.contains_key("descriptor_ref") {
        return entry;
    }
    let Some(ability_ura) = object.get("ability_ura").and_then(Value::as_str) else {
        return entry;
    };
    let Some(version) = object
        .get("version")
        .or_else(|| object.get("ability_version"))
        .and_then(Value::as_str)
    else {
        return entry;
    };
    let Some(descriptor_hash) = object.get("descriptor_hash").and_then(Value::as_str) else {
        return entry;
    };
    let Some(admission_action) = object.get("admission_action").and_then(Value::as_str) else {
        return entry;
    };
    let Some(hash_hex) = descriptor_hash.trim().strip_prefix("sha256:") else {
        return entry;
    };
    let candidate = format!(
        "{}@{}#{}!{}",
        ability_ura.trim(),
        version.trim(),
        hash_hex.trim(),
        admission_action.trim()
    );
    if axon_sdk::invocation::canonical_ability_descriptor_ref(&candidate).is_ok() {
        object.insert("descriptor_ref".to_string(), Value::String(candidate));
    }
    entry
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
    fn abilities_from_value_adds_descriptor_bound_ref() {
        let value = serde_json::json!({
            "abilities": [{
                "ability_ura": "easynet:///r/acme/ability/device.dev.er.add",
                "version": "1.0.0",
                "descriptor_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "admission_action": "stream"
            }]
        });

        let abilities = AbilityCatalogueClient::abilities_from_value(&value);

        assert_eq!(
            abilities[0].get("descriptor_ref").and_then(Value::as_str),
            Some("easynet:///r/acme/ability/device.dev.er.add@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!stream")
        );
    }
}

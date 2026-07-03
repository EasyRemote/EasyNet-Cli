// EasyNet CLI — Ability catalogue client
// ======================================
//
// Centralizes the CLI facade path for reading `meta.list_abilities`.
// `ability list`, `ability show`, and future SDK-facing CLI helpers should
// route through this object instead of hand-writing local/remote catalogue
// dispatch. The invariant is deliberately narrow: one catalogue request per
// target. Cross-device aggregate fan-out belongs in a named daemon ability, not
// in a facade loop.

#[cfg(feature = "axon-pb")]
use anyhow::Context;
use serde_json::Value;

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
            .unwrap_or_default()
    }
}

#[cfg(feature = "axon-pb")]
fn invoke_remote_catalogue(
    node: &str,
    request: Value,
    _action_label: &str,
) -> anyhow::Result<Value> {
    let target_ura = crate::support::platform::remote_device::resolve_target_device_ura(node)?;
    let caller_ura = crate::support::platform::remote_device::caller_device_ura_from_credentials();
    let target_call = crate::daemon::invocation::routing::federation_invoke::RemoteAbilityInvocationTarget::for_target_owned_selector(
        &target_ura,
        "meta.list_abilities",
    )?;
    crate::daemon::invocation::routing::federation_invoke::invoke_via_federation_forward_target(
        &target_call,
        request,
        caller_ura.as_deref(),
    )
    .with_context(|| format!("forward meta.list_abilities to target={target_ura}"))
}

#[cfg(not(feature = "axon-pb"))]
fn invoke_remote_catalogue(
    _node: &str,
    _request: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    Err(crate::support::platform::local_invoke::federation_not_wired_error(action_label))
}

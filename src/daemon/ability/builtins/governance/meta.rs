// EasyNet CLI — meta.{describe, list_abilities} ability handlers
// =================================================================
//
// File: src/daemon/ability/builtins/governance/meta.rs
//
// Per-Agent self-introspection. Per RFC §18, both abilities are
// PUBLIC (callable by anyone), with `meta.list_abilities`'s result
// row-filtered against AbilityDescriptor.visibility — that filter
// belongs at the admission/dispatch layer, not in the handler, so
// the handler returns the full local catalog and lets the gate
// trim per visibility rule (§1.6).
//
// What lives here
// ---------------
//   * meta.describe — `{ ura, identity_summary,
//                              abilities_summary, metadata }` for the
//                            host device-profile.
//                            identity_summary surfaces the canonical URA
//                            and signing-authority hint; abilities_summary
//                            is the count + namespace breakdown so a caller
//                            can decide whether to follow up with a full
//                            meta.list_abilities.
//   * meta.list_abilities — `{ abilities: AbilityDescriptor[] }`.
//                                  The same descriptor catalog
//                                  mcp.bridge.list_tools projects to MCP,
//                                  but in the native ontology shape
//                                  (no MCP wrapper). This is the canonical
//                                  Invoke surface for ability discovery;
//                                  the MCP ability is the edge-protocol
//                                  projection of the same data.
//
// Why two abilities, not one
// --------------------------
// describe is cheap (a constant-shape summary blob) and is what a
// federation peer hits to confirm "is this the device I think it
// is?" — pulling the full ability list for that question would burn
// bandwidth on every cache check. list_abilities is the catalog
// fetch. Splitting them lets a caller pay only for what it needs,
// the same way the MCP spec splits resource description from full
// resource fetch.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::daemon::ability::catalog as ability_catalog;
use crate::daemon::ability::descriptors::AbilityDescriptor;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::ability::{insert_catalog_descriptor, AbilityCatalogRow, CatalogDescriptorKey};
use crate::daemon::federation::read_model::authority_published_abilities::AuthorityPublishedAbilityStore;
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;
use serde_json::{json, Value};

pub const ABILITY_DESCRIBE: &str = crate::daemon::ability::names::governance::META_DESCRIBE;
pub const ABILITY_LIST_ABILITIES: &str =
    crate::daemon::ability::names::governance::META_LIST_ABILITIES;

/// Register both meta abilities on the registry.
///
/// `descriptors_provider` is a test fixture seam. Daemon production requires
/// the committed control-plane handle and fails closed while it is unpublished.
///
/// `registry_handle` is a `OnceLock` populated by the build site
/// AFTER `Arc::new(reg)`. The list_abilities handler reads through
/// it to enumerate every currently committed static and hot-registered
/// control-plane record. Each row already owns its normalized schema,
/// authority, transport, receipt semantics, and access policy.
pub fn register<F>(
    reg: &mut AxonAbilityCatalog,
    local_runtime_owners: Vec<OwnerKind>,
    descriptors_provider: F,
    registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    authority_published_abilities: Arc<AuthorityPublishedAbilityStore>,
) where
    F: Fn() -> Vec<AbilityDescriptor> + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync> =
        Arc::new(descriptors_provider);
    let p_for_describe = Arc::clone(&provider);
    let handle_for_describe = Arc::clone(&registry_handle);
    let authority_published_abilities_for_describe = Arc::clone(&authority_published_abilities);
    let describe_handler: crate::daemon::ability::dispatch::LocalRpcHandlerWithEnvelope =
        Arc::new(move |envelope, _args: Value| {
            describe_handler(
                &p_for_describe,
                &handle_for_describe,
                &authority_published_abilities_for_describe,
                envelope.callee(),
                envelope.subject(),
            )
        });
    let p_for_list = Arc::clone(&provider);
    let handle_for_list = Arc::clone(&registry_handle);
    let authority_published_abilities_for_list = Arc::clone(&authority_published_abilities);
    let list_handler: crate::daemon::ability::dispatch::LocalRpcHandlerWithEnvelope =
        Arc::new(move |envelope, args: Value| {
            list_abilities_handler(
                &p_for_list,
                &handle_for_list,
                args,
                &authority_published_abilities_for_list,
                envelope.callee(),
            )
        });
    for owner in introspection_registration_owners(local_runtime_owners) {
        reg.register_rpc_with_envelope_and_owner(
            ABILITY_DESCRIBE,
            owner.clone(),
            Arc::clone(&describe_handler),
        );
        reg.register_rpc_with_envelope_and_owner(
            ABILITY_LIST_ABILITIES,
            owner,
            Arc::clone(&list_handler),
        );
    }
}

fn introspection_registration_owners(local_runtime_owners: Vec<OwnerKind>) -> Vec<OwnerKind> {
    local_runtime_owners
        .into_iter()
        .map(|owner| match owner {
            OwnerKind::DeviceProfileProjection => OwnerKind::runtime_introspection_system(),
            other => other,
        })
        .collect()
}

fn describe_handler(
    descriptors_provider: &Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync>,
    registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    authority_published_abilities: &AuthorityPublishedAbilityStore,
    catalog_callee_ura: &str,
    invocation_subject_ura: &str,
) -> anyhow::Result<Value> {
    // The callee is the behavioral actor being described. The subject remains
    // the resource read for admission/audit and must never be reinterpreted as
    // actor identity or execution placement.
    let catalog_callee = crate::core::ura::parse_ura(catalog_callee_ura).map_err(|error| {
        anyhow::anyhow!(
            "meta.describe: invocation callee `{catalog_callee_ura}` is not a canonical URA: \
             {error}"
        )
    })?;

    // `describe` is the lightweight summary of the same subject-scoped
    // catalogue returned by `list_abilities`. Reusing the canonical
    // projection prevents Hub/Both mode from reporting the Device profile's
    // full static template set under a Hub identity.
    let catalog = list_abilities_handler(
        descriptors_provider,
        registry_handle,
        json!({}),
        authority_published_abilities,
        catalog_callee_ura,
    )?;
    let abilities = catalog
        .get("abilities")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow::anyhow!("meta.describe: canonical catalogue returned no abilities")
        })?;

    // abilities_summary = count + per-namespace count. The breakdown
    // is what makes the response useful to a caller deciding whether
    // to fetch the full catalogue: "12 abilities, 4 in fs.* and
    // 3 in consent.*" tells you what the device actually does.
    //
    // M0 note: the `split_once('.')` here is intentional and is NOT
    // a routing-by-name-prefix sniff. `by_namespace` is a UI hint
    // grouping verbs by their first dotted segment (i.e. by the
    // *verb namespace*, not by ownership). After the system-namespace
    // partitioning lands (Stage 4 of the deprecate-self-alias open
    // question) every name's first segment will be `device` / `hub`
    // / `<agent>` / `<user>`, at which point the breakdown becomes
    // owner-shaped naturally, but the call here continues to split
    // on the textual namespace because that's what callers want.
    let mut by_namespace: BTreeMap<String, usize> = BTreeMap::new();
    for ability in abilities {
        let Some(name) = ability.get("name").and_then(Value::as_str) else {
            continue;
        };
        let ns = name
            .split_once('.')
            .map(|(ns, _)| ns.to_string())
            .unwrap_or_else(|| "(no-namespace)".to_string());
        *by_namespace.entry(ns).or_insert(0) += 1;
    }

    let hosted_agent_count = describe_hosted_agent_count(&catalog_callee)?;

    Ok(json!({
        "ura": catalog_callee_ura,
        "subject_ura": invocation_subject_ura,
        "identity_summary": {
            "signing_authority": "self_signed",
        },
        "abilities_summary": {
            "total": abilities.len(),
            "by_namespace": by_namespace,
        },
        "metadata": {
            "hosted_agent_count": hosted_agent_count,
        },
    }))
}

fn describe_hosted_agent_count(
    catalog_callee: &crate::core::ura::ParsedURA,
) -> anyhow::Result<usize> {
    let is_device_runtime = catalog_callee.kind == crate::core::ura::URAKind::Device
        || catalog_callee
            .device_agent_ids()
            .is_some_and(|(_, system_agent_id)| {
                system_agent_id
                    == crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID
            });
    if !is_device_runtime {
        return Ok(0);
    }
    AgentAggregateRepository::load_hosted_identity_status()
        .map(|status| status.hosted_agent_count())
        .map_err(|error| {
            anyhow::anyhow!("meta.describe: load hosted-Agent identity status: {error:#}")
        })
}

fn list_abilities_handler(
    _descriptors_provider: &Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync>,
    registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    args: Value,
    authority_published_abilities: &AuthorityPublishedAbilityStore,
    catalog_callee_ura: &str,
) -> anyhow::Result<Value> {
    let scope = AbilityListScope::from_args(&args)?;
    let catalog_execution_host_ura = catalog_execution_host_ura(catalog_callee_ura)?;
    let live_registry = registry_handle.get();
    // The committed control plane is the sole production catalogue. Import
    // manifests and profile templates have already been normalized into its
    // governed descriptor, so discovery only filters and serializes those
    // aggregates.
    let mut catalog: BTreeMap<CatalogDescriptorKey, AbilityDescriptor> = BTreeMap::new();
    let mut runtime_bindings = BTreeMap::new();
    if let Some(registry) = live_registry {
        for row in registry.authority_ability_catalog_snapshot() {
            if row.execution_host_ura != catalog_execution_host_ura {
                continue;
            }
            if !ability_catalog::is_publishable_catalog_name(&row.name) {
                continue;
            }
            let descriptor = row.descriptor;
            if !scope.matches_descriptor(&descriptor) {
                continue;
            }
            let key =
                CatalogDescriptorKey::from_descriptor(&descriptor).map_err(anyhow::Error::msg)?;
            insert_catalog_descriptor(&mut catalog, descriptor, "local committed catalog")
                .map_err(anyhow::Error::msg)?;
            runtime_bindings.insert(key, row.runtime_binding);
        }
    } else {
        #[cfg(not(test))]
        anyhow::bail!("meta.list_abilities: committed descriptor control plane is not published");
        #[cfg(test)]
        {
            // Unit fixtures may publish canonical descriptors directly. This
            // branch is absent from production binaries.
            for descriptor in _descriptors_provider() {
                if !descriptor_owner_is_canonical(&descriptor)
                    || !ability_catalog::is_publishable_catalog_name(&descriptor.name)
                    || !scope.matches_descriptor(&descriptor)
                {
                    continue;
                }
                insert_catalog_descriptor(&mut catalog, descriptor, "test catalog fixture")
                    .map_err(anyhow::Error::msg)?;
            }
        }
    }

    // Realm rows enter the same canonical key/conflict gate as local rows.
    // Appending serialized JSON here would bypass duplicate/hash detection and
    // could expose two incompatible descriptor refs for one identity.
    let authority_catalog_revision = if scope.include_realm {
        let published = authority_published_abilities.snapshot_with_revision();
        let revision = published.revision;
        for descriptor in published.descriptors {
            if scope.matches_descriptor(&descriptor) {
                insert_catalog_descriptor(&mut catalog, descriptor, "Authority-published catalog")
                    .map_err(anyhow::Error::msg)?;
            }
        }
        Some(revision)
    } else {
        None
    };

    let catalog_revision = catalog_snapshot_revision(&catalog)?;

    // Runtime binding and advisory health are row annotations, not governed
    // descriptor metadata. Mutating AbilityDescriptor.metadata here would
    // change the descriptor hash and fabricate a new descriptor_ref when
    // execution or health state changes.
    let mut merged: Vec<Value> = catalog
        .into_iter()
        .map(|(key, descriptor)| {
            let health = descriptor
                .canonical_ability_ura()
                .and_then(|ura| crate::daemon::ability::health::snapshot(&ura));
            let mut row = public_catalog_descriptor_row(&descriptor)?;
            if let Value::Object(object) = &mut row {
                if let Some(runtime_binding) = runtime_bindings.remove(&key) {
                    object.insert(
                        "runtime_binding".to_string(),
                        serde_json::to_value(runtime_binding)?,
                    );
                }
                if let Some(health) = health {
                    let mut annotation = serde_json::Map::new();
                    annotation.insert(
                        "status".to_string(),
                        Value::String(health.status.as_wire_str().to_string()),
                    );
                    annotation.insert(
                        "checked_unix_ms".to_string(),
                        Value::Number(health.checked_unix_ms.into()),
                    );
                    if !health.detail.is_empty() {
                        annotation.insert("detail".to_string(), Value::String(health.detail));
                    }
                    object.insert("health".to_string(), Value::Object(annotation));
                }
            }
            Ok::<_, anyhow::Error>(row)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    scope.apply(&mut merged);
    Ok(json!({
        "abilities": merged,
        "catalog_revision": catalog_revision,
        "authority_catalog_revision": authority_catalog_revision,
    }))
}

fn public_catalog_descriptor_row(descriptor: &AbilityDescriptor) -> anyhow::Result<Value> {
    AbilityCatalogRow::from_descriptor(descriptor.clone())
        .map(AbilityCatalogRow::into_value)
        .map_err(anyhow::Error::msg)
}

fn catalog_snapshot_revision(
    catalog: &BTreeMap<CatalogDescriptorKey, AbilityDescriptor>,
) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};

    let rows = catalog
        .values()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let canonical = crate::daemon::ability::descriptors::canonical_json_bytes(&Value::Array(rows));
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(canonical))))
}

fn catalog_execution_host_ura(catalog_callee_ura: &str) -> anyhow::Result<String> {
    let callee = crate::core::ura::parse_ura(catalog_callee_ura).map_err(|error| {
        anyhow::anyhow!(
            "meta.list_abilities: catalogue callee `{catalog_callee_ura}` is invalid: {error}"
        )
    })?;
    if let Some((device_id, system_agent_id)) = callee.device_agent_ids() {
        if system_agent_id
            == crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID
        {
            return Ok(crate::core::ura::device_ura(&callee.realm, device_id));
        }
    }
    Ok(catalog_callee_ura.to_string())
}

struct AbilityListScope {
    include_realm: bool,
    owner_ura: Option<String>,
    ability_ura: Option<String>,
    descriptor_version: Option<String>,
}

impl AbilityListScope {
    fn from_args(args: &Value) -> anyhow::Result<Self> {
        let object = args
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("meta.list_abilities: args must be a JSON object"))?;
        for key in object.keys() {
            match key.as_str() {
                "scope" | "owner_ura" | "ability_ura" | "descriptor_version" => {}
                other => {
                    anyhow::bail!("meta.list_abilities: unsupported field `{other}`")
                }
            }
        }

        let include_realm = match string_arg(object, "scope").as_deref() {
            None | Some("local") => false,
            Some("realm") => true,
            Some(other) => {
                anyhow::bail!(
                    "meta.list_abilities: unsupported scope {other:?}; expected `local` or `realm`"
                )
            }
        };
        let owner_ura = string_arg(object, "owner_ura")
            .map(|ura| parse_owner_scope("owner_ura", &ura).map(|_| ura))
            .transpose()?;
        let ability_ura = string_arg(object, "ability_ura")
            .map(|ura| parse_ability_scope(&ura))
            .transpose()?;
        let descriptor_version = string_arg(object, "descriptor_version")
            .map(|version| {
                crate::daemon::ability::AbilityDescriptorVersion::new(version.clone())
                    .map(|_| version)
                    .map_err(|error| {
                        anyhow::anyhow!("meta.list_abilities: invalid descriptor_version: {error}")
                    })
            })
            .transpose()?;
        validate_ability_scope_owner(owner_ura.as_deref(), ability_ura.as_deref())?;

        Ok(Self {
            include_realm,
            owner_ura,
            ability_ura,
            descriptor_version,
        })
    }

    fn apply(&self, abilities: &mut Vec<Value>) {
        if let Some(owner_ura) = self.owner_ura.as_deref() {
            abilities.retain(|entry| {
                entry
                    .get("owner_ura")
                    .and_then(Value::as_str)
                    .map(|candidate| candidate == owner_ura)
                    .unwrap_or(false)
            });
        }
        if let Some(ability_ura) = self.ability_ura.as_deref() {
            abilities.retain(|entry| {
                entry
                    .get("ability_ura")
                    .and_then(Value::as_str)
                    .map(|candidate| candidate == ability_ura)
                    .unwrap_or(false)
            });
        }
        if let Some(version) = self.descriptor_version.as_deref() {
            abilities.retain(|entry| {
                entry
                    .get("version")
                    .and_then(Value::as_str)
                    .map(|candidate| candidate == version)
                    .unwrap_or(false)
            });
        }
    }

    fn matches_descriptor(&self, descriptor: &AbilityDescriptor) -> bool {
        if let Some(owner_ura) = self.owner_ura.as_deref() {
            if descriptor.owner_ura != owner_ura {
                return false;
            }
        }
        if let Some(ability_ura) = self.ability_ura.as_deref() {
            if !descriptor
                .canonical_ability_ura()
                .as_deref()
                .map(|candidate| candidate == ability_ura)
                .unwrap_or(false)
            {
                return false;
            }
        }
        if self
            .descriptor_version
            .as_deref()
            .is_some_and(|version| descriptor.version != version)
        {
            return false;
        }
        true
    }
}

fn string_arg(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_ability_scope(ura: &str) -> anyhow::Result<String> {
    let parsed = crate::core::ura::parse_ura(ura)
        .map_err(|e| anyhow::anyhow!("meta.list_abilities: invalid ability_ura {ura:?}: {e}"))?;
    match parsed.kind {
        crate::core::ura::URAKind::Ability => Ok(ura.to_string()),
        other => anyhow::bail!(
            "meta.list_abilities: ability_ura must be an Ability URA, got {:?}",
            other
        ),
    }
}

fn parse_owner_scope(field: &str, ura: &str) -> anyhow::Result<()> {
    let parsed = crate::core::ura::parse_ura(ura)
        .map_err(|e| anyhow::anyhow!("meta.list_abilities: invalid {field} {ura:?}: {e}"))?;
    match parsed.kind {
        crate::core::ura::URAKind::Agent
        | crate::core::ura::URAKind::Device
        | crate::core::ura::URAKind::Authority
        | crate::core::ura::URAKind::Service
        | crate::core::ura::URAKind::User => Ok(()),
        other => anyhow::bail!(
            "meta.list_abilities: {field} must be an owner URA, got {:?}",
            other
        ),
    }
}

fn validate_ability_scope_owner(
    owner_ura: Option<&str>,
    ability_ura: Option<&str>,
) -> anyhow::Result<()> {
    let (Some(owner_ura), Some(ability_ura)) = (owner_ura, ability_ura) else {
        return Ok(());
    };
    let matches_owner =
        crate::core::ura::public_ability_name_from_ability_ura(owner_ura, ability_ura).is_some();
    if !matches_owner {
        anyhow::bail!("meta.list_abilities: owner_ura and ability_ura owner must match");
    }
    Ok(())
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn describe_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

pub fn describe_description() -> &'static str {
    "Return this Agent's identity + ability summary. Lightweight \
     companion to meta.list_abilities — answers \"who are you and \
     roughly what do you do\" in one call so a peer doesn't have \
     to fetch the full descriptor catalogue for a cache check."
}

pub fn list_abilities_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "scope": {
                "type": "string",
                "enum": ["local", "realm"],
                "description":
                    "`local` (default) returns device-owned abilities only. \
                     `realm` adds realm Authority-published abilities the realm Authority \
                     broadcast at join + heartbeat (RFC-001 v4.1.7)."
            },
            "owner_ura": {
                "type": "string",
                "description": "Canonical owner URA. Filters the catalogue to abilities published by that owner."
            },
            "ability_ura": {
                "type": "string",
                "description": "Canonical Ability URA. Filters the catalogue to one exact ability descriptor set."
            },
            "descriptor_version": {
                "type": "string",
                "description": "Exact governed descriptor version. Use with ability_ura when selecting one version."
            }
        },
        "additionalProperties": false,
    })
}

pub fn list_abilities_description() -> &'static str {
    "Return the full local AbilityDescriptor catalogue. Canonical \
     Invoke surface for ability discovery; the MCP-shaped projection \
     lives at mcp.bridge.list_tools for external MCP clients."
}

#[cfg(test)]
fn descriptor_owner_is_canonical(descriptor: &AbilityDescriptor) -> bool {
    crate::core::ura::parse_ura(&descriptor.owner_ura).is_ok()
        && descriptor
            .canonical_ability_ura()
            .is_some_and(|ability_ura| crate::core::ura::parse_ura(&ability_ura).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::descriptors::{
        AbilityDescriptor, AdmissionAction, ScopeRule, Visibility,
    };

    fn d(name: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(
            name,
            "easynet:///r/test/agent/device.01DEV.runtime-introspection",
            Visibility::Public,
            AdmissionAction::Invoke,
        )
        .expect("test descriptor")
    }

    fn d_for_owner(name: &str, owner_ura: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(name, owner_ura, Visibility::Scoped, AdmissionAction::Invoke)
            .expect("test descriptor")
    }

    #[test]
    fn canonical_catalog_preserves_distinct_versions_and_call_modes() {
        let mut catalog = BTreeMap::new();
        insert_catalog_descriptor(&mut catalog, d("fs.read"), "test").expect("RPC v1 inserts");
        insert_catalog_descriptor(
            &mut catalog,
            d("fs.read").with_call_mode(crate::daemon::ability::CallMode::Stream),
            "test",
        )
        .expect("Stream v1 inserts");
        insert_catalog_descriptor(
            &mut catalog,
            d("fs.read").with_version("2.0.0").expect("valid version"),
            "test",
        )
        .expect("RPC v2 inserts");

        assert_eq!(catalog.len(), 3);
    }

    #[test]
    fn canonical_catalog_rejects_conflicting_same_identity_version_and_mode() {
        let mut catalog = BTreeMap::new();
        insert_catalog_descriptor(
            &mut catalog,
            d("fs.read").with_input_schema(json!({"type": "object"})),
            "test",
        )
        .expect("first descriptor inserts");

        let error = insert_catalog_descriptor(
            &mut catalog,
            d("fs.read").with_input_schema(json!({"type": "string"})),
            "test",
        )
        .expect_err("same identity/version/mode with different schema must fail closed");
        assert!(error
            .to_string()
            .contains("contains conflicting descriptors"));
    }

    fn seed_test_credentials(realm: &str, node_id: &str, username: &str) {
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: node_id.to_string(),
                credential_token: "test-token".to_string(),
                hub_endpoint: "axon://hub.test:50051".to_string(),
                realm: realm.to_string(),
                username: Some(username.to_string()),
                user_id: Some(format!("user-{username}")),
                ..Default::default()
            },
        )
        .expect("seed credentials");
    }

    /// Empty OnceLock used by tests that don't care about the
    /// live-registry merge — they only exercise the static
    /// descriptor path. The list_abilities handler tolerates an
    /// unset OnceLock (returns the static catalogue alone), so
    /// passing an empty one is the cheapest fixture.
    fn empty_registry_handle() -> Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>> {
        Arc::new(std::sync::OnceLock::new())
    }

    fn register<F>(
        reg: &mut AxonAbilityCatalog,
        descriptors_provider: F,
        registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    ) where
        F: Fn() -> Vec<AbilityDescriptor> + Send + Sync + 'static,
    {
        super::register(
            reg,
            vec![OwnerKind::DeviceProfileProjection],
            descriptors_provider,
            registry_handle,
            AuthorityPublishedAbilityStore::new(),
        );
    }

    fn invoke_list(
        reg: &AxonAbilityCatalog,
        callee_ura: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        let handler = reg
            .resolve_rpc_with_env(ABILITY_LIST_ABILITIES)
            .expect("meta.list_abilities envelope handler");
        let envelope = crate::daemon::ability::dispatch::EnvelopeContext::for_test_targeted_ability(
            "easynet:///r/test/user/test-caller",
            callee_ura,
            ABILITY_LIST_ABILITIES,
            callee_ura,
        );
        handler(envelope, args)
    }

    fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
        match payload.downcast::<String>() {
            Ok(message) => *message,
            Err(payload) => match payload.downcast::<&'static str>() {
                Ok(message) => (*message).to_string(),
                Err(_) => "<non-string panic>".to_string(),
            },
        }
    }

    fn invoke_describe(reg: &AxonAbilityCatalog, callee_ura: &str) -> anyhow::Result<Value> {
        let handler = reg
            .resolve_rpc_with_env(ABILITY_DESCRIBE)
            .expect("meta.describe envelope handler");
        let envelope = crate::daemon::ability::dispatch::EnvelopeContext::for_test_targeted_ability(
            "easynet:///r/test/user/test-caller",
            callee_ura,
            ABILITY_DESCRIBE,
            callee_ura,
        );
        handler(envelope, json!({}))
    }

    fn invoke_list_targeted(
        reg: &AxonAbilityCatalog,
        callee_ura: &str,
        args: Value,
    ) -> anyhow::Result<Value> {
        reg.execute_rpc(explicit_meta_target(
            ABILITY_LIST_ABILITIES,
            callee_ura,
            args,
        )?)
    }

    fn invoke_describe_targeted(
        reg: &AxonAbilityCatalog,
        callee_ura: &str,
    ) -> anyhow::Result<Value> {
        reg.execute_rpc(explicit_meta_target(
            ABILITY_DESCRIBE,
            callee_ura,
            json!({}),
        )?)
    }

    fn explicit_meta_target(
        ability: &str,
        subject_owner_ura: &str,
        args: Value,
    ) -> anyhow::Result<crate::daemon::invocation::routing::target::InvocationTarget> {
        let ability_owner_ura = meta_read_ability_owner_for_subject(subject_owner_ura);
        let ability_ura = crate::core::ura::owner_ability_ura(&ability_owner_ura, ability)
            .expect("canonical meta ability URA for explicit target");
        crate::daemon::invocation::routing::target::PublicInvocationTargetIssuer::local_explicit_tuple(
            ability_ura,
            args,
            crate::daemon::invocation::routing::target::CallMode::Rpc,
            subject_owner_ura.to_string(),
            axon_sdk::invocation::CausalContext::None,
        )
    }

    fn meta_read_ability_owner_for_subject(subject_owner_ura: &str) -> String {
        let parsed = crate::core::ura::parse_ura(subject_owner_ura)
            .expect("explicit meta target subject must be canonical");
        match parsed.kind {
            crate::core::ura::URAKind::Device => {
                let device_id = parsed
                    .device_id()
                    .expect("Device subject must carry device id");
                crate::core::ura::device_agent_ura(
                    &parsed.realm,
                    device_id,
                    crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID,
                )
            }
            _ => subject_owner_ura.to_string(),
        }
    }

    fn canonical_meta_fixtures() -> Vec<AbilityDescriptor> {
        const FIXTURE_OWNER: &str =
            "easynet:///r/test/agent/device.meta-fixture.runtime-introspection";
        [ABILITY_DESCRIBE, ABILITY_LIST_ABILITIES]
            .into_iter()
            .map(|name| {
                AbilityDescriptor::new(
                    name,
                    FIXTURE_OWNER,
                    Visibility::Scoped,
                    AdmissionAction::Invoke,
                )
                .expect("canonical descriptor fixture")
                .with_scope_subjects(ScopeRule::OnlyMatching(vec![FIXTURE_OWNER.to_string()]))
                .with_scope_agents(ScopeRule::OnlyMatching(vec![FIXTURE_OWNER.to_string()]))
            })
            .collect()
    }

    fn authority_bound_meta_registry(
        authority_context: crate::daemon::ability::dispatch::AbilityAuthorityContext,
        owners: Vec<OwnerKind>,
    ) -> Arc<AxonAbilityCatalog> {
        let handle = Arc::new(std::sync::OnceLock::new());
        let mut registry = runtime_metadata_test_catalog(authority_context);
        super::register(
            &mut registry,
            owners,
            canonical_meta_fixtures,
            Arc::clone(&handle),
            AuthorityPublishedAbilityStore::new(),
        );
        let registry = Arc::new(registry);
        handle
            .set(Arc::clone(&registry))
            .expect("publish authority-bound meta registry");
        registry
    }

    fn metadata_test_catalog() -> AxonAbilityCatalog {
        metadata_test_catalog_for_device("easynet:///r/test/device/01DEV")
    }

    fn metadata_test_catalog_for_device(device_ura: &str) -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(device_ura)
    }

    fn runtime_metadata_test_catalog(
        authority_context: crate::daemon::ability::dispatch::AbilityAuthorityContext,
    ) -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            authority_context,
        )
    }

    fn registry_with_hosted_agent_authorities(
        device_ura: &str,
        hosted_agent_uras: impl IntoIterator<Item = &'static str>,
    ) -> AxonAbilityCatalog {
        let authority_context =
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_combined_authority_roots_with_hosted_agents(
                device_ura,
                hosted_agent_uras.into_iter().map(str::to_string),
            )
            .expect("explicit hosted-Agent test authorities must be canonical");
        runtime_metadata_test_catalog(authority_context)
    }

    #[test]
    fn registration_makes_both_abilities_dispatchable() {
        let mut reg = metadata_test_catalog();
        register(&mut reg, Vec::new, empty_registry_handle());
        assert!(reg.has_rpc(ABILITY_DESCRIBE));
        assert!(reg.resolve_rpc_with_env(ABILITY_DESCRIBE).is_some());
        assert!(reg.has_rpc(ABILITY_LIST_ABILITIES));
        assert!(reg.resolve_rpc_with_env(ABILITY_LIST_ABILITIES).is_some());
        // The legacy `device.easynet.discover` alias was removed
        // in RFC-001 v4.1.7 M2. The canonical name is the only
        // surface; assert the legacy literal is NOT registered so
        // future regressions that re-introduce the alias trip here.
        assert!(reg.get_rpc("device.easynet.discover").is_none());
    }

    #[test]
    fn hub_meta_surfaces_project_exact_hub_authority_without_self_templates() {
        use crate::daemon::ability::dispatch::AbilityAuthorityContext;

        let hub_ura = crate::core::ura::hub_ura("hub-view");
        let registry = authority_bound_meta_registry(
            AbilityAuthorityContext::for_realm_authority_root(&hub_ura)
                .expect("fixed realm authority context"),
            vec![OwnerKind::RealmAuthority],
        );

        let response = invoke_list_targeted(&registry, &hub_ura, json!({})).unwrap();
        let abilities = response["abilities"].as_array().unwrap();
        assert_eq!(abilities.len(), 2, "Hub view must contain only Hub rows");
        assert!(abilities.iter().all(|row| row["owner_ura"] == hub_ura));
        assert!(abilities.iter().all(|row| row["owner_ura"] != "self"));

        let list = abilities
            .iter()
            .find(|row| row["name"] == ABILITY_LIST_ABILITIES)
            .expect("Hub meta.list_abilities descriptor");
        assert_eq!(
            list["ability_ura"],
            crate::core::ura::hub_ability_ura("hub-view", ABILITY_LIST_ABILITIES)
        );
        assert_eq!(list["visibility"], json!("SCOPED"));
        assert_eq!(list["scope_subjects"]["kind"], json!("any"));
        assert_eq!(list["scope_agents"]["kind"], json!("any"));

        let describe = invoke_describe_targeted(&registry, &hub_ura).unwrap();
        assert_eq!(describe["ura"], hub_ura);
        assert_eq!(
            describe["identity_summary"]["signing_authority"],
            "self_signed"
        );
        assert_eq!(describe["abilities_summary"]["total"], 2);
        assert_eq!(describe["metadata"]["hosted_agent_count"], 0);
    }

    #[test]
    fn hub_live_registry_never_calls_device_descriptor_provider() {
        use crate::daemon::ability::dispatch::AbilityAuthorityContext;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let hub_ura = crate::core::ura::hub_ura("hub-no-provider");
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let calls_for_provider = Arc::clone(&provider_calls);
        let handle = Arc::new(std::sync::OnceLock::new());
        let mut registry = runtime_metadata_test_catalog(
            AbilityAuthorityContext::for_realm_authority_root(&hub_ura)
                .expect("realm authority context"),
        );
        super::register(
            &mut registry,
            vec![OwnerKind::RealmAuthority],
            move || -> Vec<AbilityDescriptor> {
                calls_for_provider.fetch_add(1, Ordering::SeqCst);
                panic!("Hub live catalogue must not call Device descriptor provider")
            },
            Arc::clone(&handle),
            AuthorityPublishedAbilityStore::new(),
        );
        let registry = Arc::new(registry);
        handle
            .set(Arc::clone(&registry))
            .expect("publish Hub registry");

        let response = invoke_list_targeted(&registry, &hub_ura, json!({})).expect("Hub list");
        let rows = response["abilities"].as_array().expect("ability rows");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row["owner_ura"] == hub_ura));
        assert!(rows
            .iter()
            .all(|row| row["source"] == "daemon:control-plane"));
        assert!(rows.iter().all(|row| {
            row["ability_ura"].as_str().is_some_and(|ability_ura| {
                crate::core::ura::AbilitySelector::parse(ability_ura)
                    .is_ok_and(|selector| selector.owner_ura() == hub_ura)
            })
        }));

        let describe = invoke_describe_targeted(&registry, &hub_ura).expect("Hub describe");
        assert_eq!(describe["abilities_summary"]["total"], 2);
        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn device_describe_rejects_corrupt_hosted_agent_projection_before_zero_fallback() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let path = crate::daemon::persistence::local_agents::path();
        std::fs::create_dir_all(path.parent().expect("local-agents parent"))
            .expect("create local-agents parent");
        std::fs::write(&path, b"{not-json").expect("write malformed local-agents projection");

        let device_ura = crate::core::ura::device_ura("describe-corrupt", "dev-1");
        let registry = authority_bound_meta_registry(
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
                &device_ura,
            )
            .expect("device authority context"),
            vec![OwnerKind::DeviceProfileProjection],
        );

        let error = invoke_describe_targeted(&registry, &device_ura)
            .expect_err("device describe must fail closed on corrupt hosted-Agent projection");
        let message = format!("{error:#}");
        assert!(
            message.contains("meta.describe: load hosted-Agent identity status")
                && message.contains("local-agents.json"),
            "wrong corrupt projection error: {message}"
        );
    }

    #[test]
    fn combined_runtime_projects_disjoint_device_and_hub_views_from_callee() {
        use crate::daemon::ability::dispatch::AbilityAuthorityContext;

        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let device_ura = crate::core::ura::device_ura("both-view", "dev-1");
        let hub_ura = crate::core::ura::hub_ura("both-view");
        let registry = authority_bound_meta_registry(
            AbilityAuthorityContext::for_combined_authority_roots(&device_ura)
                .expect("combined authority context"),
            vec![
                OwnerKind::DeviceProfileProjection,
                OwnerKind::RealmAuthority,
            ],
        );

        let device_response = invoke_list_targeted(&registry, &device_ura, json!({})).unwrap();
        let hub_response = invoke_list_targeted(&registry, &hub_ura, json!({})).unwrap();
        let device_rows = device_response["abilities"].as_array().unwrap();
        let hub_rows = hub_response["abilities"].as_array().unwrap();
        assert_eq!(device_rows.len(), 2);
        assert_eq!(hub_rows.len(), 2);
        let runtime_introspection_ura = crate::core::ura::device_agent_ura(
            "both-view",
            "dev-1",
            crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID,
        );
        assert!(
            device_rows
                .iter()
                .all(|row| row["owner_ura"] == runtime_introspection_ura
                    && row["owner_ura"] != "self")
        );
        assert!(hub_rows
            .iter()
            .all(|row| row["owner_ura"] == hub_ura && row["owner_ura"] != "self"));

        let device_list = device_rows
            .iter()
            .find(|row| row["name"] == ABILITY_LIST_ABILITIES)
            .expect("Device list descriptor");
        let hub_list = hub_rows
            .iter()
            .find(|row| row["name"] == ABILITY_LIST_ABILITIES)
            .expect("Hub list descriptor");
        assert_eq!(
            device_list["ability_ura"],
            crate::core::ura::owner_ability_ura(&runtime_introspection_ura, ABILITY_LIST_ABILITIES)
                .expect("runtime-introspection list ability URA")
        );
        assert_eq!(
            hub_list["ability_ura"],
            crate::core::ura::hub_ability_ura("both-view", ABILITY_LIST_ABILITIES)
        );
        assert_ne!(device_list["ability_ura"], hub_list["ability_ura"]);

        let device_describe = invoke_describe_targeted(&registry, &device_ura).unwrap();
        let hub_describe = invoke_describe_targeted(&registry, &hub_ura).unwrap();
        assert_eq!(device_describe["ura"], runtime_introspection_ura);
        assert_eq!(device_describe["subject_ura"], device_ura);
        assert_eq!(hub_describe["ura"], hub_ura);
        assert_eq!(
            device_describe["abilities_summary"]["total"], 2,
            "Device describe must summarize its callee-scoped catalogue: {device_describe}"
        );
        assert_eq!(hub_describe["abilities_summary"]["total"], 2);
    }

    #[test]
    fn list_abilities_projects_static_descriptors_to_public_catalog_names() {
        let mut reg = metadata_test_catalog();
        register(
            &mut reg,
            || vec![d("observe.health"), d("agent.list")],
            empty_registry_handle(),
        );
        let resp = invoke_list(&reg, "easynet:///r/test/device/01DEV", json!({})).unwrap();
        let abilities = resp["abilities"].as_array().unwrap();
        assert_eq!(abilities.len(), 2);
        // The internal static descriptors are built from registry
        // keys (`observe.health`, `agent.list`), while the
        // product-facing catalogue exposes owner-local public names
        // (`observe.health`, `agent.list`). The canonical `ability_ura`
        // carries owner identity.
        let names: Vec<&str> = abilities
            .iter()
            .filter_map(|a| a["name"].as_str())
            .collect();
        assert!(names.contains(&"observe.health"));
        assert!(names.contains(&"agent.list"));
        assert!(
            abilities.iter().all(|a| a["ability_ura"]
                .as_str()
                .unwrap_or_default()
                .contains("/ability/")),
            "every public row must carry canonical ability_ura: {abilities:?}"
        );
        assert!(
            abilities.iter().all(|a| a["descriptor_ref"]
                .as_str()
                .is_some_and(|descriptor_ref| descriptor_ref
                    .starts_with(a["ability_ura"].as_str().unwrap_or_default()))),
            "every public row must carry canonical descriptor_ref: {abilities:?}"
        );
    }

    #[test]
    fn list_abilities_accepts_service_owner_scope_from_committed_catalog() {
        use std::sync::OnceLock;

        let device_ura = crate::core::ura::device_ura("pages-scope", "dev-1");
        let owner_user_id = "user-alice";
        let pages_service_ura =
            crate::core::ura::service_ura("pages-scope", owner_user_id, "pages");
        let live_registry = metadata_test_catalog_for_device(&device_ura);
        live_registry
            .hot_register_rpc_with_spec(
                "project_list",
                OwnerKind::pages_service(owner_user_id),
                crate::daemon::ability::manifest::AbilityManifest::new(
                    "project_list",
                    "List published Pages projects.",
                    json!({
                        "type": "object",
                        "additionalProperties": false
                    }),
                )
                .and_then(|manifest| manifest.with_admission_action("read"))
                .expect("valid Pages project_list manifest"),
                Arc::new(|_args| Ok(json!({ "projects": [] }))),
            )
            .expect("register Service-owned Pages project_list");
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        handle
            .set(Arc::new(live_registry))
            .expect("publish live registry");

        let mut reg = metadata_test_catalog_for_device(&device_ura);
        register(&mut reg, Vec::new, handle);
        let resp = invoke_list(
            &reg,
            &device_ura,
            json!({
                "owner_ura": pages_service_ura
            }),
        )
        .expect("Service owner_ura must be a valid meta.list_abilities scope");
        let abilities = resp["abilities"].as_array().expect("ability rows");
        assert_eq!(
            abilities.len(),
            1,
            "Service owner scope must return only Pages Service rows: {resp}"
        );
        assert_eq!(abilities[0]["owner_ura"], pages_service_ura);
        assert_eq!(abilities[0]["name"], "project_list");
        assert_eq!(
            abilities[0]["ability_ura"],
            crate::core::ura::owner_ability_ura(&pages_service_ura, "project_list")
                .expect("canonical Pages Service project_list ability URA")
        );
    }

    #[test]
    fn list_abilities_projects_health_as_non_governed_row_annotation() {
        use crate::daemon::ability::health::{
            self as ability_health, AbilityHealthRecord, HealthStatus,
        };

        // Seed the process-wide health store under THIS test's unique
        // owner so parallel tests cannot collide (same residue
        // discipline as the hub-store test below).
        let owner = "easynet:///r/test/agent/dev.healthmeta";
        let monitored = d_for_owner("svc_probe", owner);
        let unmonitored = d_for_owner("svc_plain", owner);
        let ability_ura = monitored
            .canonical_ability_ura()
            .expect("canonical ability ura");
        let descriptor_ref = monitored
            .descriptor_ref()
            .expect("canonical descriptor ref before health projection");
        ability_health::seed_for_tests(
            &ability_ura,
            AbilityHealthRecord {
                status: HealthStatus::Unhealthy,
                detail: "exit 7: connection refused".to_string(),
                checked_unix_ms: 1_234,
                consecutive_failures: 3,
                last_boot_unix_ms: None,
                next_probe_unix_ms: i64::MAX,
            },
        );

        let mut reg = metadata_test_catalog();
        let provider_rows = vec![monitored, unmonitored];
        register(
            &mut reg,
            move || provider_rows.clone(),
            empty_registry_handle(),
        );
        let resp = invoke_list(&reg, owner, json!({})).unwrap();
        let abilities = resp["abilities"].as_array().unwrap();

        let seeded = abilities
            .iter()
            .find(|a| a["ability_ura"].as_str() == Some(ability_ura.as_str()))
            .expect("seeded ability row present");
        assert_eq!(seeded["health"]["status"].as_str(), Some("unhealthy"));
        assert_eq!(seeded["health"]["checked_unix_ms"].as_u64(), Some(1_234));
        assert_eq!(
            seeded["health"]["detail"].as_str(),
            Some("exit 7: connection refused")
        );
        assert_eq!(
            seeded["descriptor_ref"].as_str(),
            Some(descriptor_ref.as_str()),
            "ephemeral health must not mutate governed descriptor identity"
        );

        // A descriptor with no record must NOT grow health keys —
        // absence means "not monitored", never a fabricated state.
        let plain = abilities
            .iter()
            .find(|a| a["name"].as_str() == Some("svc_plain"))
            .expect("plain ability row present");
        assert!(plain.get("health").is_none());
    }

    #[test]
    fn list_abilities_realm_scope_includes_authority_published_entries() {
        // RFC-001 v4.1.7 realm Authority broadcast contract: when the caller
        // passes `scope = "realm"`, the merged catalogue includes entries
        // cached from federation joins and heartbeats. The default-local path
        // stays disjoint — pin both axes.
        use crate::daemon::federation::client::ability_contract::AuthorityAbilityEntry;
        let authority_published_abilities = AuthorityPublishedAbilityStore::new();

        let mut reg = metadata_test_catalog();
        super::register(
            &mut reg,
            vec![OwnerKind::DeviceProfileProjection],
            || vec![d("observe.health")],
            empty_registry_handle(),
            Arc::clone(&authority_published_abilities),
        );
        authority_published_abilities
            .apply_diff(
                crate::daemon::federation::client::ability_contract::AuthorityAbilitiesDiff {
                    revision: 99,
                    added: vec![AuthorityAbilityEntry {
                        name: "test.scope".to_string(),
                        descriptor: serde_json::to_value(
                            AbilityDescriptor::new(
                                "test.scope",
                                crate::core::ura::hub_ura("test"),
                                Visibility::Public,
                                AdmissionAction::Read,
                            )
                            .expect("canonical realm Authority descriptor")
                            .with_description("smoke entry"),
                        )
                        .expect("realm Authority descriptor json"),
                    }],
                    removed: vec![],
                },
            )
            .expect("canonical realm Authority ability diff");

        // Default scope: realm Authority entry must NOT appear.
        let local_resp = invoke_list(&reg, "easynet:///r/test/device/01DEV", json!({})).unwrap();
        let local_names: Vec<String> = local_resp["abilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a["name"].as_str().map(String::from))
            .collect();
        assert!(local_names.contains(&"observe.health".to_string()));
        assert!(
            !local_names.contains(&"test.scope".to_string()),
            "default scope must not leak realm Authority broadcast entries"
        );

        // Realm scope: the exact Authority-published descriptor must appear.
        let realm_resp = invoke_list(
            &reg,
            "easynet:///r/test/device/01DEV",
            json!({"scope": "realm"}),
        )
        .unwrap();
        let abilities = realm_resp["abilities"].as_array().unwrap();
        let authority_entry = abilities
            .iter()
            .find(|a| a["name"] == "test.scope")
            .expect("test.scope must be in realm-scope output");
        assert_eq!(authority_entry["source"], "");
        assert_eq!(realm_resp["authority_catalog_revision"], 99);
        assert!(realm_resp["catalog_revision"]
            .as_str()
            .is_some_and(|revision| revision.starts_with("sha256:")));
        assert!(
            authority_entry["descriptor_ref"]
                .as_str()
                .is_some_and(|descriptor_ref| descriptor_ref.starts_with(&format!(
                    "{}@",
                    crate::core::ura::authority_ability_ura("test", "test.scope")
                ))),
            "realm Authority-published row must stay canonical: {authority_entry}"
        );
    }

    #[test]
    fn list_abilities_filters_by_owner_ura_and_ability_ura() {
        let alice = "easynet:///r/test-realm/agent/user-1.alice";
        let bob = "easynet:///r/test-realm/agent/user-1.bob";
        let mut reg = metadata_test_catalog();
        register(
            &mut reg,
            move || {
                vec![
                    d_for_owner("chat", alice),
                    d_for_owner("summarise", alice),
                    d_for_owner("chat", bob),
                ]
            },
            empty_registry_handle(),
        );
        let by_owner = invoke_list(
            &reg,
            "easynet:///r/test-realm/device/test-device",
            json!({ "owner_ura": alice }),
        )
        .unwrap();
        let abilities = by_owner["abilities"].as_array().unwrap();
        assert_eq!(
            abilities.len(),
            2,
            "owner_ura must scope to the selected owner: {by_owner}"
        );
        assert!(abilities.iter().all(|a| a["owner_ura"] == alice));

        let ability_ura = crate::core::ura::owner_ability_ura(alice, "chat").unwrap();
        let by_ability = invoke_list(
            &reg,
            "easynet:///r/test-realm/device/test-device",
            json!({ "ability_ura": ability_ura }),
        )
        .unwrap();
        let abilities = by_ability["abilities"].as_array().unwrap();
        assert_eq!(
            abilities.len(),
            1,
            "full Ability URA must scope to one ability: {by_ability}"
        );
        assert_eq!(abilities[0]["name"], "chat");
        assert_eq!(abilities[0]["owner_ura"], alice);

        let err = invoke_list(
            &reg,
            "easynet:///r/test-realm/device/test-device",
            json!({
                "owner_ura": bob,
                "ability_ura": crate::core::ura::owner_ability_ura(alice, "chat").unwrap(),
            }),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("must match"), "got {err}");
    }

    #[test]
    fn list_abilities_rejects_unknown_query_fields() {
        let mut reg = metadata_test_catalog();
        register(&mut reg, Vec::new, empty_registry_handle());
        let err = invoke_list(
            &reg,
            "easynet:///r/test/device/01DEV",
            json!({ "agent_id": "legacy" }),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("unsupported field"), "got {err}");
    }

    #[test]
    fn list_abilities_rejects_retired_agent_and_subject_scope_fields() {
        let mut reg = metadata_test_catalog();
        register(&mut reg, Vec::new, empty_registry_handle());

        for legacy in [
            json!({ "agent_ura": "easynet:///r/test/device/01DEV" }),
            json!({ "subject_ura": "easynet:///r/test/ability/device.01DEV.meta.list_abilities" }),
        ] {
            let err = invoke_list(&reg, "easynet:///r/test/device/01DEV", legacy)
                .unwrap_err()
                .to_string();
            assert!(err.contains("unsupported field"), "got {err}");
        }
    }

    #[test]
    fn live_registry_surfaces_canonical_descriptor_normalized_from_manifest() {
        //
        // Authority identity is injected explicitly. The catalogue must not
        // reconstruct Agent owners from credentials or process-global HOME.
        use crate::daemon::ability::dispatch::{AbilityAuthorityContext, OwnerKind};
        use std::sync::OnceLock;

        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let device_ura = "easynet:///r/alice-realm/device/test-node";
        let alice_ura = crate::core::ura::agent_ura("alice-realm", "user-alice", "alice");
        let bob_ura = crate::core::ura::agent_ura("alice-realm", "user-bob", "bob");

        let mut reg = metadata_test_catalog();
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());

        // Registration imports the manifest into the governed descriptor;
        // meta.list_abilities reads that committed descriptor directly.
        let mut live_reg = runtime_metadata_test_catalog(
            AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
                device_ura,
                vec![alice_ura.clone(), bob_ura.clone()],
            )
            .expect("fixed Device context with hosted Agents"),
        );
        live_reg.register_stream_with_spec(
            "alice.chat",
            OwnerKind::Agent("alice".to_string()),
            crate::daemon::ability::manifest::default_chat_manifest(),
            Arc::new(|_args| {
                Ok(crate::daemon::ability::dispatch::StreamSource::Snapshot(
                    Vec::new(),
                ))
            }),
        );
        live_reg.register_stream_with_spec(
            "bob.chat",
            OwnerKind::Agent("bob".to_string()),
            crate::daemon::ability::manifest::default_chat_manifest(),
            Arc::new(|_args| {
                Ok(crate::daemon::ability::dispatch::StreamSource::Snapshot(
                    Vec::new(),
                ))
            }),
        );
        live_reg.register_stream_with_spec(
            "alice.subscribe",
            OwnerKind::Agent("alice".to_string()),
            crate::daemon::ability::manifest::AbilityManifest::new(
                "subscribe",
                "Subscribe to test Agent events.",
                json!({"type": "object"}),
            )
            .and_then(|manifest| manifest.with_admission_action("stream"))
            .expect("test stream manifest carries admission action"),
            Arc::new(|_args| {
                Ok(crate::daemon::ability::dispatch::StreamSource::Snapshot(
                    Vec::new(),
                ))
            }),
        );
        live_reg
            .hot_register_stream_with_spec(
                "alice.mcp_search",
                OwnerKind::Agent("alice".to_string()),
                crate::daemon::ability::manifest::AbilityManifest::new(
                    "mcp_search",
                    "Search reflected MCP content",
                    json!({
                        "type": "object",
                        "properties": {
                            "query": { "type": "string" }
                        },
                        "required": ["query"]
                    }),
                )
                .and_then(|manifest| manifest.with_admission_action("stream"))
                .expect("valid MCP manifest"),
                Arc::new(|_args| {
                    Ok(crate::daemon::ability::dispatch::StreamSource::Snapshot(
                        Vec::new(),
                    ))
                }),
            )
            .expect("dynamic stream manifest registers");
        let authority_rows = live_reg.authority_ability_catalog_snapshot();
        assert!(
            authority_rows
                .iter()
                .any(|row| row.name == "alice.chat" && row.descriptor.owner_ura == alice_ura),
            "fixed Device context must project Agent authority rows: {authority_rows:?}"
        );
        handle.set(Arc::new(live_reg)).expect("set OnceLock");

        register(&mut reg, Vec::new, handle);
        let resp = invoke_list(&reg, device_ura, json!({})).unwrap();
        let abilities = resp["abilities"].as_array().unwrap();

        let chat = abilities
            .iter()
            .find(|a| a["name"] == "chat" && a["owner_ura"] == alice_ura)
            .unwrap_or_else(|| {
                panic!("device-sponsored Agent chat must retain its Agent namespace: {abilities:?}")
            });
        let chat_owners: std::collections::BTreeSet<&str> = abilities
            .iter()
            .filter(|a| a["name"].as_str().is_some_and(|name| name == "chat"))
            .filter_map(|a| a["owner_ura"].as_str())
            .collect();
        assert!(
            chat_owners.contains(alice_ura.as_str()) && chat_owners.contains(bob_ura.as_str()),
            "agent-scoped public names must preserve one descriptor per owner, got: {chat_owners:?}"
        );
        let desc = chat["description"].as_str().unwrap_or_default();
        assert_eq!(
            desc,
            crate::daemon::ability::manifest::default_chat_manifest().description()
        );
        // Input schema must be a proper JSON Schema object with at
        // least the `prompt` property the chat manifest declares.
        let input_schema = &chat["schema_summary"]["input"];
        assert_eq!(
            input_schema["type"], "object",
            "input must be a JSON object schema"
        );
        assert!(
            input_schema["properties"]["prompt"].is_object(),
            "chat manifest declares `prompt` as a property; canonical descriptor must surface it. \
             Got: {input_schema}"
        );
        assert_eq!(
            chat["hints"]["streaming_only"],
            json!(true),
            "transport hints must project the canonical stream call mode"
        );
        assert_eq!(chat["call_mode"], json!("stream"));

        let subscribe = abilities
            .iter()
            .find(|a| a["name"] == "subscribe" && a["owner_ura"] == alice_ura)
            .expect("agent-owned subscribe must surface as the owner-local ability name");
        assert_eq!(
            subscribe["hints"]["streaming_only"],
            json!(true),
            "non-chat manifest-backed stream abilities must surface streaming_only"
        );
        assert_eq!(subscribe["call_mode"], json!("stream"));
        assert!(subscribe.get("class").is_none());

        let mcp_search = abilities
            .iter()
            .find(|a| a["name"] == "mcp_search" && a["owner_ura"] == alice_ura)
            .expect("dynamic MCP ability must surface as the owner-local ability name");
        assert_eq!(
            mcp_search["schema_summary"]["input"]["properties"]["query"]["type"],
            json!("string"),
            "dynamic overlay manifests must be visible to meta.list_abilities"
        );
    }

    #[test]
    fn agent_owned_static_registration_rejects_fallback_manifest_publication() {
        use crate::daemon::ability::dispatch::{AbilityAuthorityContext, OwnerKind};

        let mut live_reg = runtime_metadata_test_catalog(
            AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
                "easynet:///r/alice-realm/device/test-node",
                vec![crate::core::ura::agent_ura(
                    "alice-realm",
                    "user-alice",
                    "alice",
                )],
            )
            .expect("fixed Device context with hosted Agent"),
        );

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            live_reg.register_rpc_with_owner_and_action(
                "alice.legacy",
                OwnerKind::Agent("alice".to_string()),
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
                Arc::new(|_args| Ok(json!({}))),
            );
        }));
        let panic = result.expect_err(
            "agent-owned descriptor publication must require a provider-backed manifest",
        );
        let message = panic_message(panic);
        assert!(
            message.contains("requires an explicit manifest")
                && message.contains("fallback metadata"),
            "wrong fallback-manifest rejection: {message}"
        );
    }

    #[test]
    fn live_registry_catalog_drops_records_removed_from_control_plane() {
        use crate::daemon::ability::dispatch::{AbilityAuthorityContext, OwnerKind};
        use std::sync::OnceLock;

        let device_ura = "easynet:///r/test/device/01DEV";
        let mut live_reg = runtime_metadata_test_catalog(
            AbilityAuthorityContext::for_device_authority_root(device_ura)
                .expect("fixed Device authority context"),
        );
        live_reg.register_rpc_with_owner_and_action(
            "unowned.test",
            OwnerKind::runtime_introspection_system(),
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            Arc::new(|_args| Ok(json!({}))),
        );
        live_reg.clear_owner_for_test("unowned.test");

        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        handle.set(Arc::new(live_reg)).expect("set live registry");

        let mut reg = metadata_test_catalog();
        register(&mut reg, Vec::new, handle);
        let resp = invoke_list(&reg, device_ura, json!({})).unwrap();
        let names: Vec<_> = resp["abilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|ability| ability["name"].as_str())
            .collect();
        assert!(
            !names.contains(&"unowned.test"),
            "catalogue must follow the canonical control-plane removal: {resp}"
        );
    }

    #[test]
    fn list_abilities_surfaces_dynamic_manifest_schema_for_hot_registered_tools() {
        // Hot MCP reload writes handlers + manifests through the
        // dynamic side table. `meta.list_abilities` is the user-facing
        // catalogue backing SchemaForm, so it must read static OR
        // dynamic manifests rather than the static-only map.
        use crate::daemon::ability::dispatch::OwnerKind;
        use crate::daemon::persistence::local_agents::{save, LocalAgentsFile};
        use std::sync::OnceLock;

        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save(&LocalAgentsFile {
            host_device_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
            hosted_agents: Vec::new(),
        })
        .expect("seed local-agents.json");

        let live_reg = Arc::new(metadata_test_catalog_for_device(
            "easynet:///r/test-realm/device/dev-1",
        ));
        let manifest = crate::daemon::ability::manifest::AbilityManifest::new(
            "hot_echo",
            "Echo a hot-reloaded MCP payload.",
            json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": {"type": "string"}
                }
            }),
        )
        .and_then(|manifest| manifest.with_admission_action("invoke"))
        .expect("valid manifest");
        live_reg
            .hot_register_rpc_with_spec(
                "hot.echo",
                OwnerKind::plugin_management_system(),
                manifest,
                Arc::new(|_args| Ok(json!({}))),
            )
            .expect("dynamic RPC manifest registers");

        let mut reg = metadata_test_catalog_for_device("easynet:///r/test-realm/device/dev-1");
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        handle
            .set(Arc::clone(&live_reg))
            .expect("set live registry");
        register(&mut reg, Vec::new, handle);

        let resp = invoke_list(&reg, "easynet:///r/test-realm/device/dev-1", json!({})).unwrap();
        let ability = resp["abilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "hot.echo")
            .expect("hot-registered dynamic ability must appear");

        assert_eq!(
            ability["ability_ura"],
            "easynet:///r/test-realm/ability/system-agent.dev-1.plugin-management.hot.echo"
        );
        assert_eq!(ability["description"], "Echo a hot-reloaded MCP payload.");
        assert_eq!(
            ability["schema_summary"]["input"]["properties"]["text"]["type"], "string",
            "dynamic manifest schema must flow into meta.list_abilities: {ability}"
        );
    }

    #[cfg(feature = "remote-desktop")]
    #[test]
    fn list_abilities_surfaces_remote_desktop_plugin_manifest_schema() {
        use crate::daemon::persistence::local_agents::{save, LocalAgentsFile};
        use crate::daemon::plugins::{
            DaemonPluginBinder, PluginContributionBuilder, PluginContributionSet, PluginKind,
            PluginRequirementSet, PluginRuntimeLimits,
        };
        use std::sync::OnceLock;

        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save(&LocalAgentsFile {
            host_device_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
            hosted_agents: Vec::new(),
        })
        .expect("seed local-agents.json");

        let mut live_reg = metadata_test_catalog_for_device("easynet:///r/test-realm/device/dev-1");
        let limits = PluginRuntimeLimits::new(128, 8);
        let mut builder = PluginContributionBuilder::new(
            "easynet.remote_desktop",
            "0.1.0",
            PluginKind::Builtin,
            limits,
            PluginRequirementSet::default(),
            Vec::new(),
        );
        crate::daemon::plugins::remote_desktop::contribute(&mut builder, limits)
            .expect("remote desktop plugin contribution");
        let contribution = builder
            .finish()
            .expect("remote desktop package contribution");
        DaemonPluginBinder::static_catalog(&mut live_reg)
            .bind_set(&PluginContributionSet::new(vec![contribution]))
            .expect("bind remote desktop contribution");
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        handle.set(Arc::new(live_reg)).expect("set live registry");

        let mut reg = metadata_test_catalog_for_device("easynet:///r/test-realm/device/dev-1");
        register(&mut reg, Vec::new, handle);
        let resp = invoke_list(&reg, "easynet:///r/test-realm/device/dev-1", json!({})).unwrap();
        let ability = resp["abilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "remote_desktop.create_session")
            .expect("remote_desktop.create_session must appear in discovery");

        let desc = ability["description"].as_str().unwrap_or_default();
        assert!(
            !desc.contains("no manifest schema"),
            "remote desktop plugin abilities must publish manifest text, got: {desc:?}"
        );
        assert_eq!(
            ability["schema_summary"]["input"]["properties"]["mode"]["enum"][0],
            json!("view_only"),
            "plugin manifest schema must flow into meta.list_abilities: {ability}"
        );
        assert_eq!(ability["call_mode"], json!("rpc"));
        assert!(ability.get("class").is_none());
        assert_eq!(ability["source"], json!("daemon:control-plane"));
    }

    #[test]
    fn list_abilities_includes_hot_added_hosted_agent_from_local_agents_ura() {
        use crate::daemon::persistence::local_agents::{
            save, upsert_hosted_agent, LocalAgentsFile,
        };
        use std::sync::OnceLock;

        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_test_credentials("test-realm", "dev-1", "alice");

        let mut local = LocalAgentsFile {
            host_device_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
            hosted_agents: Vec::new(),
        };
        upsert_hosted_agent(
            &mut local,
            "llm",
            "alice",
            "easynet:///r/test-realm/agent/user-1.alice",
        );
        upsert_hosted_agent(
            &mut local,
            "llm",
            "bob",
            "easynet:///r/test-realm/agent/user-1.bob",
        );
        save(&local).expect("seed local-agents.json");

        // The live control plane, not local-agents.json, is the discovery
        // source. The persisted identity exists only to resolve each Agent's
        // canonical authority root while the registrar commits its rows.
        let live_registry = registry_with_hosted_agent_authorities(
            "easynet:///r/test-realm/device/dev-1",
            [
                "easynet:///r/test-realm/agent/user-1.alice",
                "easynet:///r/test-realm/agent/user-1.bob",
            ],
        );
        for agent_name in ["alice", "bob"] {
            live_registry
                .hot_register_rpc_with_spec(
                    format!("{agent_name}.chat"),
                    OwnerKind::Agent(agent_name.to_string()),
                    crate::daemon::ability::manifest::default_chat_manifest(),
                    std::sync::Arc::new(|_| Ok(Value::Null)),
                )
                .expect("commit hosted Agent chat authority row");
        }
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        handle
            .set(Arc::new(live_registry))
            .expect("set live registry");

        let mut reg = metadata_test_catalog_for_device("easynet:///r/test-realm/device/dev-1");
        register(&mut reg, Vec::new, handle);
        let resp = invoke_list(&reg, "easynet:///r/test-realm/device/dev-1", json!({})).unwrap();
        let abilities = resp["abilities"].as_array().unwrap();
        let chat = abilities
            .iter()
            .find(|a| {
                a["name"] == "chat"
                    && a["owner_ura"] == "easynet:///r/test-realm/agent/user-1.alice"
            })
            .expect("hot-added hosted agent chat must appear in meta.list_abilities");
        let chat_owners: std::collections::BTreeSet<&str> = abilities
            .iter()
            .filter(|a| a["name"] == "chat")
            .filter_map(|a| a["owner_ura"].as_str())
            .collect();
        assert!(
            chat_owners.contains("easynet:///r/test-realm/agent/user-1.alice")
                && chat_owners.contains("easynet:///r/test-realm/agent/user-1.bob"),
            "hot-added hosted-agent abilities must not collapse: {chat_owners:?}"
        );

        assert_eq!(
            chat["owner_ura"],
            "easynet:///r/test-realm/agent/user-1.alice"
        );
        assert_eq!(
            chat["description"],
            crate::daemon::ability::manifest::default_chat_manifest().description()
        );
        assert_eq!(
            chat["runtime_binding"]["state"],
            json!("bound"),
            "meta.list_abilities must carry observed execution binding outside the governed descriptor: {chat}"
        );
        assert_eq!(
            chat["runtime_binding"]["implementation_source"],
            json!("native_daemon")
        );
        assert!(
            chat["schema_summary"]["input"]["properties"]["prompt"].is_object(),
            "chat descriptor must carry the manifest input schema: {chat}"
        );
    }

    #[test]
    fn list_abilities_keeps_same_public_ability_name_for_multiple_hosted_agents() {
        use crate::daemon::persistence::local_agents::{
            save, upsert_hosted_agent, LocalAgentsFile,
        };
        use std::sync::OnceLock;

        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_test_credentials("test-realm", "dev-1", "alice");

        let mut local = LocalAgentsFile {
            host_device_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
            hosted_agents: Vec::new(),
        };
        upsert_hosted_agent(
            &mut local,
            "llm",
            "anthropic",
            "easynet:///r/test-realm/agent/user-1.anthropic",
        );
        upsert_hosted_agent(
            &mut local,
            "llm",
            "backend-engineer",
            "easynet:///r/test-realm/agent/user-1.backend-engineer",
        );
        save(&local).expect("seed local-agents.json");

        let live_registry = registry_with_hosted_agent_authorities(
            "easynet:///r/test-realm/device/dev-1",
            [
                "easynet:///r/test-realm/agent/user-1.anthropic",
                "easynet:///r/test-realm/agent/user-1.backend-engineer",
            ],
        );
        for agent_name in ["anthropic", "backend-engineer"] {
            live_registry
                .hot_register_rpc_with_spec(
                    format!("{agent_name}.chat"),
                    OwnerKind::Agent(agent_name.to_string()),
                    crate::daemon::ability::manifest::default_chat_manifest(),
                    std::sync::Arc::new(|_| Ok(Value::Null)),
                )
                .expect("commit hosted Agent chat authority row");
        }
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        handle
            .set(Arc::new(live_registry))
            .expect("set live registry");

        let mut reg = metadata_test_catalog_for_device("easynet:///r/test-realm/device/dev-1");
        register(&mut reg, Vec::new, handle);
        let resp = invoke_list(&reg, "easynet:///r/test-realm/device/dev-1", json!({})).unwrap();
        let chats: Vec<&Value> = resp["abilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["name"] == "chat")
            .collect();

        let owners: std::collections::BTreeSet<&str> = chats
            .iter()
            .filter_map(|a| a["owner_ura"].as_str())
            .collect();
        let ability_uras: std::collections::BTreeSet<&str> = chats
            .iter()
            .filter_map(|a| a["ability_ura"].as_str())
            .collect();
        assert_eq!(
            owners,
            std::collections::BTreeSet::from([
                "easynet:///r/test-realm/agent/user-1.anthropic",
                "easynet:///r/test-realm/agent/user-1.backend-engineer",
            ]),
            "agent-scoped chat names must be retained once per owner; got {chats:?}"
        );
        assert_eq!(
            ability_uras,
            std::collections::BTreeSet::from([
                "easynet:///r/test-realm/ability/user-1.anthropic.chat",
                "easynet:///r/test-realm/ability/user-1.backend-engineer.chat",
            ]),
            "catalog identity must be canonical ability URA, not string-spliced owner/name; got {chats:?}"
        );
    }

    #[test]
    fn describe_buckets_abilities_by_namespace() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = metadata_test_catalog();
        register(
            &mut reg,
            || {
                vec![
                    d("observe.health"),
                    d("agent.list"),
                    d(crate::daemon::ability::names::device_control::SESSION_LIST),
                    d("consent.subscribe"),
                ]
            },
            empty_registry_handle(),
        );
        let resp = invoke_describe(&reg, "easynet:///r/test/device/01DEV").unwrap();
        assert_eq!(resp["abilities_summary"]["total"], 4);
        let by_ns = resp["abilities_summary"]["by_namespace"]
            .as_object()
            .unwrap();
        assert_eq!(by_ns["observe"], 1);
        assert_eq!(by_ns["agent"], 1);
        assert_eq!(by_ns["session"], 1);
        assert_eq!(by_ns["consent"], 1);
    }

    #[test]
    fn describe_handles_empty_catalog() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = metadata_test_catalog();
        register(&mut reg, Vec::new, empty_registry_handle());
        let resp = invoke_describe(&reg, "easynet:///r/test/device/01DEV").unwrap();
        assert_eq!(resp["abilities_summary"]["total"], 0);
        // Empty by_namespace must be an object, not absent — caller
        // shouldn't have to special-case missing key.
        assert!(resp["abilities_summary"]["by_namespace"]
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn describe_input_schema_is_an_empty_object() {
        let s = describe_input_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"].as_object().unwrap().is_empty());
        assert_eq!(s["additionalProperties"], false);
    }

    #[test]
    fn list_abilities_schema_advertises_scope_param() {
        // RFC-001 v4.1.7 realm Authority broadcast contract added the optional
        // `scope` parameter (`local` | `realm`). Pin so a future
        // schema edit either keeps the enum or trips this test.
        let s = list_abilities_input_schema();
        assert_eq!(s["type"], "object");
        assert_eq!(s["additionalProperties"], false);
        let scope = &s["properties"]["scope"];
        assert_eq!(scope["type"], "string");
        let enum_values = scope["enum"].as_array().unwrap();
        let strs: Vec<&str> = enum_values.iter().filter_map(|v| v.as_str()).collect();
        assert!(strs.contains(&"local"));
        assert!(strs.contains(&"realm"));
    }
}

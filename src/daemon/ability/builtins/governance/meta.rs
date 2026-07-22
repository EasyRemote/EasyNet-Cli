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
use crate::daemon::ability::descriptors::{AbilityDescriptor, AbilityIdentity};
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore;
use crate::daemon::persistence::agent_aggregate::AgentAggregateRepository;
use serde_json::{json, Value};

pub const ABILITY_DESCRIBE: &str = crate::daemon::ability::names::governance::META_DESCRIBE;
pub const ABILITY_LIST_ABILITIES: &str =
    crate::daemon::ability::names::governance::META_LIST_ABILITIES;

/// Register both meta abilities on the registry.
///
/// `descriptors_provider` is the explicit embedded-consumer seam used only
/// when no live registry has been published. Daemon production reads the
/// committed control-plane descriptors and never overlays this provider.
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
    hub_published_abilities: Arc<HubPublishedAbilityStore>,
) where
    F: Fn() -> Vec<AbilityDescriptor> + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync> =
        Arc::new(descriptors_provider);
    let p_for_describe = Arc::clone(&provider);
    let handle_for_describe = Arc::clone(&registry_handle);
    let hub_published_abilities_for_describe = Arc::clone(&hub_published_abilities);
    let describe_handler: crate::daemon::ability::dispatch::LocalRpcHandlerWithEnvelope =
        Arc::new(move |envelope, _args: Value| {
            describe_handler(
                &p_for_describe,
                &handle_for_describe,
                &hub_published_abilities_for_describe,
                envelope.callee(),
            )
        });
    let p_for_list = Arc::clone(&provider);
    let handle_for_list = Arc::clone(&registry_handle);
    let hub_published_abilities_for_list = Arc::clone(&hub_published_abilities);
    let list_handler: crate::daemon::ability::dispatch::LocalRpcHandlerWithEnvelope =
        Arc::new(move |envelope, args: Value| {
            list_abilities_handler(
                &p_for_list,
                &handle_for_list,
                args,
                &hub_published_abilities_for_list,
                envelope.callee(),
            )
        });
    for owner in local_runtime_owners {
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

fn describe_handler(
    descriptors_provider: &Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync>,
    registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    hub_published_abilities: &HubPublishedAbilityStore,
    invocation_callee_ura: &str,
) -> anyhow::Result<Value> {
    // The envelope callee is the runtime authority being described. Identity
    // must never be reconstructed from Device-only persistence: Hub mode has
    // no local-agent identity, and Both mode has two distinct authority views.
    let invocation_callee =
        crate::core::ura::parse_ura(invocation_callee_ura).map_err(|error| {
            anyhow::anyhow!(
            "meta.describe: invocation callee `{invocation_callee_ura}` is not a canonical URA: \
             {error}"
        )
        })?;

    // `describe` is the lightweight summary of the same callee-scoped
    // catalogue returned by `list_abilities`. Reusing the canonical
    // projection prevents Hub/Both mode from reporting the Device profile's
    // full static template set under a Hub identity.
    let catalog = list_abilities_handler(
        descriptors_provider,
        registry_handle,
        json!({}),
        hub_published_abilities,
        invocation_callee_ura,
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

    let hosted_agent_count = if invocation_callee.kind == crate::core::ura::URAKind::Device {
        AgentAggregateRepository::load_hosted_identity_status()
            .map(|status| status.hosted_agent_count())
            .unwrap_or_default()
    } else {
        0
    };

    Ok(json!({
        "ura": invocation_callee_ura,
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

fn list_abilities_handler(
    descriptors_provider: &Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync>,
    registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    args: Value,
    hub_published_abilities: &HubPublishedAbilityStore,
    invocation_callee_ura: &str,
) -> anyhow::Result<Value> {
    let scope = AbilityListScope::from_args(&args)?;
    let live_registry = registry_handle.get();
    // The committed control plane is the sole production catalogue. Import
    // manifests and profile templates have already been normalized into its
    // governed descriptor, so discovery only filters and serializes those
    // aggregates. The provider remains an explicit embedded-consumer seam for
    // callers that deliberately publish no live registry.
    let mut catalog: BTreeMap<
        (AbilityIdentity, String, crate::daemon::ability::CallMode),
        AbilityDescriptor,
    > = BTreeMap::new();
    if let Some(registry) = live_registry {
        for row in registry.authority_ability_catalog_snapshot() {
            if !authority_row_visible_from_callee(&row, invocation_callee_ura) {
                continue;
            }
            if !ability_catalog::is_publishable_catalog_name(&row.name) {
                continue;
            }
            let descriptor = row.descriptor;
            if !scope.matches_descriptor(&descriptor) {
                continue;
            }
            insert_canonical_descriptor(&mut catalog, descriptor)?;
        }
    } else {
        // Explicit static-provider seam (embedded consumers and unit tests).
        // Only already-canonical descriptors may leave it. A `self` template
        // has schema value but no routable identity, so it is not a wire row.
        for descriptor in descriptors_provider() {
            if !descriptor_owner_is_canonical(&descriptor)
                || !ability_catalog::is_publishable_catalog_name(&descriptor.name)
                || !scope.matches_descriptor(&descriptor)
            {
                continue;
            }
            insert_canonical_descriptor(&mut catalog, descriptor)?;
        }
    }

    // Final pass — service-health metadata. Applied uniformly over
    // the assembled catalog so no insertion path has to remember it. The
    // store is keyed by canonical ability URA, the same
    // `owner_ability_ura` construction `canonical_ability_ura()`
    // uses, so the lookup cannot drift from the monitor's writes.
    // Advisory metadata only: absence means "not monitored", never
    // "down" — the invoke path does not consult this.
    let mut merged: Vec<Value> = catalog
        .into_values()
        .map(|d| {
            let descriptor = match d
                .canonical_ability_ura()
                .and_then(|ura| crate::daemon::ability::health::snapshot(&ura))
            {
                Some(health) => {
                    let mut d = d
                        .with_metadata_entry("health_status", health.status.as_wire_str())
                        .with_metadata_entry(
                            "health_checked_unix_ms",
                            health.checked_unix_ms.to_string(),
                        );
                    if !health.detail.is_empty() {
                        d = d.with_metadata_entry("health_detail", health.detail);
                    }
                    d
                }
                None => d,
            };
            serde_json::to_value(descriptor).unwrap_or(Value::Null)
        })
        .collect();

    // Phase 3: hub-published abilities. Only when the caller asked for realm
    // scope — the default-local path stays owner-local. The federation read
    // model has already validated every cached hub entry as a canonical
    // `AbilityDescriptor`, so the merged catalog always exposes the same
    // `ability_ura` / `descriptor_ref` facts as local registry rows.
    if scope.include_realm {
        for descriptor in hub_published_abilities.snapshot() {
            let mut row = serde_json::to_value(descriptor)?;
            if let Value::Object(ref mut map) = row {
                let source_is_empty = map
                    .get("source")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty();
                if source_is_empty {
                    map.insert(
                        "source".to_string(),
                        Value::String("hub:broadcast".to_string()),
                    );
                }
            }
            merged.push(row);
        }
    }

    scope.apply(&mut merged);
    Ok(json!({ "abilities": merged }))
}

fn insert_canonical_descriptor(
    catalog: &mut BTreeMap<
        (AbilityIdentity, String, crate::daemon::ability::CallMode),
        AbilityDescriptor,
    >,
    descriptor: AbilityDescriptor,
) -> anyhow::Result<()> {
    let identity = descriptor.identity().ok_or_else(|| {
        anyhow::anyhow!(
            "meta.list_abilities: canonical descriptor `{}` has no identity",
            descriptor.name
        )
    })?;
    let key = (
        identity.clone(),
        descriptor.version.clone(),
        descriptor.call_mode(),
    );
    match catalog.entry(key) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(descriptor);
        }
        std::collections::btree_map::Entry::Occupied(entry) if entry.get() == &descriptor => {}
        std::collections::btree_map::Entry::Occupied(entry) => {
            anyhow::bail!(
                "meta.list_abilities: conflicting canonical descriptors for identity `{}` \
                 version `{}` mode `{}` (hashes `{}` and `{}`)",
                identity.as_str(),
                descriptor.version,
                descriptor.call_mode().as_str(),
                entry.get().descriptor_hash_prefixed(),
                descriptor.descriptor_hash_prefixed(),
            );
        }
    }
    Ok(())
}

fn authority_row_visible_from_callee(
    row: &crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow,
    invocation_callee_ura: &str,
) -> bool {
    match &row.owner {
        OwnerKind::Device | OwnerKind::Hub => row.descriptor.owner_ura == invocation_callee_ura,
        OwnerKind::Agent(_) | OwnerKind::User(_) => {
            row.descriptor.owner_ura == invocation_callee_ura
                || invocation_callee_hosts_subordinate_owners(invocation_callee_ura)
        }
    }
}

fn invocation_callee_hosts_subordinate_owners(invocation_callee_ura: &str) -> bool {
    crate::core::ura::parse_ura(invocation_callee_ura)
        .map(|callee| callee.kind == crate::core::ura::URAKind::Device)
        .unwrap_or(false)
}

fn descriptor_owner_is_canonical(descriptor: &AbilityDescriptor) -> bool {
    crate::core::ura::parse_ura(&descriptor.owner_ura).is_ok()
        && descriptor
            .canonical_ability_ura()
            .is_some_and(|ability_ura| crate::core::ura::parse_ura(&ability_ura).is_ok())
}

struct AbilityListScope {
    include_realm: bool,
    owner_ura: Option<String>,
    ability_ura: Option<String>,
}

impl AbilityListScope {
    fn from_args(args: &Value) -> anyhow::Result<Self> {
        let object = args
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("meta.list_abilities: args must be a JSON object"))?;
        for key in object.keys() {
            match key.as_str() {
                "scope" | "agent_ura" | "subject_ura" => {}
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
        let owner_from_agent = string_arg(object, "agent_ura")
            .map(|ura| parse_owner_scope("agent_ura", &ura).map(|_| ura))
            .transpose()?;
        let subject = string_arg(object, "subject_ura")
            .map(|ura| AbilitySubjectScope::parse(&ura))
            .transpose()?;
        let owner_ura = merge_owner_scope(owner_from_agent, subject.as_ref())?;
        let ability_ura = subject.and_then(|scope| scope.ability_ura);

        Ok(Self {
            include_realm,
            owner_ura,
            ability_ura,
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
    }

    fn matches_descriptor(&self, descriptor: &AbilityDescriptor) -> bool {
        if let Some(owner_ura) = self.owner_ura.as_deref() {
            if descriptor.owner_ura != owner_ura {
                return false;
            }
        }
        if let Some(ability_ura) = self.ability_ura.as_deref() {
            return descriptor
                .canonical_ability_ura()
                .as_deref()
                .map(|candidate| candidate == ability_ura)
                .unwrap_or(false);
        }
        true
    }
}

struct AbilitySubjectScope {
    owner_ura: Option<String>,
    ability_ura: Option<String>,
}

impl AbilitySubjectScope {
    fn parse(subject_ura: &str) -> anyhow::Result<Self> {
        let parsed = crate::core::ura::parse_ura(subject_ura).map_err(|e| {
            anyhow::anyhow!("meta.list_abilities: invalid subject_ura {subject_ura:?}: {e}")
        })?;
        match parsed.kind {
            crate::core::ura::URAKind::Ability => Ok(Self {
                owner_ura: None,
                ability_ura: Some(subject_ura.to_string()),
            }),
            crate::core::ura::URAKind::Agent
            | crate::core::ura::URAKind::Device
            | crate::core::ura::URAKind::Authority
            | crate::core::ura::URAKind::User => Ok(Self {
                owner_ura: Some(subject_ura.to_string()),
                ability_ura: None,
            }),
            other => anyhow::bail!(
                "meta.list_abilities: subject_ura must be an owner URA or Ability URA, got {:?}",
                other
            ),
        }
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

fn parse_owner_scope(field: &str, ura: &str) -> anyhow::Result<()> {
    let parsed = crate::core::ura::parse_ura(ura)
        .map_err(|e| anyhow::anyhow!("meta.list_abilities: invalid {field} {ura:?}: {e}"))?;
    match parsed.kind {
        crate::core::ura::URAKind::Agent
        | crate::core::ura::URAKind::Device
        | crate::core::ura::URAKind::Authority
        | crate::core::ura::URAKind::User => Ok(()),
        other => anyhow::bail!(
            "meta.list_abilities: {field} must be an owner URA, got {:?}",
            other
        ),
    }
}

fn merge_owner_scope(
    owner_ura: Option<String>,
    subject: Option<&AbilitySubjectScope>,
) -> anyhow::Result<Option<String>> {
    match (owner_ura, subject) {
        (Some(owner_ura), Some(subject)) => {
            if let Some(subject_owner) = subject.owner_ura.as_deref() {
                if owner_ura != subject_owner {
                    anyhow::bail!(
                        "meta.list_abilities: agent_ura and subject_ura owner must match"
                    );
                }
            }
            if let Some(ability_ura) = subject.ability_ura.as_deref() {
                let matches_owner =
                    crate::core::ura::public_ability_name_from_ability_ura(&owner_ura, ability_ura)
                        .is_some();
                if !matches_owner {
                    anyhow::bail!(
                        "meta.list_abilities: agent_ura and subject_ura ability owner must match"
                    );
                }
            }
            Ok(Some(owner_ura))
        }
        (Some(owner_ura), None) => Ok(Some(owner_ura)),
        (None, Some(subject)) => Ok(subject.owner_ura.clone()),
        (None, None) => Ok(None),
    }
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
                     `realm` adds hub-published abilities the realm hub \
                     broadcast at join + heartbeat (RFC-001 v4.1.7)."
            },
            "agent_ura": {
                "type": "string",
                "description": "Canonical owner URA. Filters the catalogue to abilities published by that owner."
            },
            "subject_ura": {
                "type": "string",
                "description": "Owner URA or full Ability URA. Owner URAs filter by publisher; Ability URAs filter to one canonical ability."
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
mod tests {
    use super::*;
    use crate::daemon::ability::descriptors::{
        AbilityDescriptor, AdmissionAction, ScopeRule, Visibility,
    };

    fn d(name: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(
            name,
            "easynet:///r/test/device/01DEV",
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
        insert_canonical_descriptor(&mut catalog, d("fs.read")).expect("RPC v1 inserts");
        insert_canonical_descriptor(
            &mut catalog,
            d("fs.read").with_call_mode(crate::daemon::ability::CallMode::Stream),
        )
        .expect("Stream v1 inserts");
        insert_canonical_descriptor(
            &mut catalog,
            d("fs.read").with_version("2.0.0").expect("valid version"),
        )
        .expect("RPC v2 inserts");

        assert_eq!(catalog.len(), 3);
    }

    #[test]
    fn canonical_catalog_rejects_conflicting_same_identity_version_and_mode() {
        let mut catalog = BTreeMap::new();
        insert_canonical_descriptor(
            &mut catalog,
            d("fs.read").with_input_schema(json!({"type": "object"})),
        )
        .expect("first descriptor inserts");

        let error = insert_canonical_descriptor(
            &mut catalog,
            d("fs.read").with_input_schema(json!({"type": "string"})),
        )
        .expect_err("same identity/version/mode with different schema must fail closed");
        assert!(error
            .to_string()
            .contains("conflicting canonical descriptors"));
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
            vec![OwnerKind::Device],
            descriptors_provider,
            registry_handle,
            HubPublishedAbilityStore::new(),
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

    fn canonical_meta_fixtures() -> Vec<AbilityDescriptor> {
        const FIXTURE_OWNER: &str = "easynet:///r/test/device/meta-fixture";
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
        let mut registry = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            authority_context,
        );
        super::register(
            &mut registry,
            owners,
            canonical_meta_fixtures,
            Arc::clone(&handle),
            HubPublishedAbilityStore::new(),
        );
        let registry = Arc::new(registry);
        handle
            .set(Arc::clone(&registry))
            .expect("publish authority-bound meta registry");
        registry
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
        AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            authority_context,
        )
    }

    #[test]
    fn registration_makes_both_abilities_dispatchable() {
        let mut reg = AxonAbilityCatalog::new();
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
            AbilityAuthorityContext::for_hub_authority_root(&hub_ura)
                .expect("fixed Hub authority context"),
            vec![OwnerKind::Hub],
        );

        let response = invoke_list(&registry, &hub_ura, json!({})).unwrap();
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

        let describe = invoke_describe(&registry, &hub_ura).unwrap();
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
        let mut registry = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            AbilityAuthorityContext::for_hub_authority_root(&hub_ura)
                .expect("Hub authority context"),
        );
        super::register(
            &mut registry,
            vec![OwnerKind::Hub],
            move || -> Vec<AbilityDescriptor> {
                calls_for_provider.fetch_add(1, Ordering::SeqCst);
                panic!("Hub live catalogue must not call Device descriptor provider")
            },
            Arc::clone(&handle),
            HubPublishedAbilityStore::new(),
        );
        let registry = Arc::new(registry);
        handle
            .set(Arc::clone(&registry))
            .expect("publish Hub registry");

        let response = invoke_list(&registry, &hub_ura, json!({})).expect("Hub list");
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

        let describe = invoke_describe(&registry, &hub_ura).expect("Hub describe");
        assert_eq!(describe["abilities_summary"]["total"], 2);
        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn combined_runtime_projects_disjoint_device_and_hub_views_from_callee() {
        use crate::daemon::ability::dispatch::AbilityAuthorityContext;

        let device_ura = crate::core::ura::device_ura("both-view", "dev-1");
        let hub_ura = crate::core::ura::hub_ura("both-view");
        let registry = authority_bound_meta_registry(
            AbilityAuthorityContext::for_combined_authority_roots(&device_ura)
                .expect("combined authority context"),
            vec![OwnerKind::Device, OwnerKind::Hub],
        );

        let device_response = invoke_list(&registry, &device_ura, json!({})).unwrap();
        let hub_response = invoke_list(&registry, &hub_ura, json!({})).unwrap();
        let device_rows = device_response["abilities"].as_array().unwrap();
        let hub_rows = hub_response["abilities"].as_array().unwrap();
        assert_eq!(device_rows.len(), 2);
        assert_eq!(hub_rows.len(), 2);
        assert!(device_rows
            .iter()
            .all(|row| row["owner_ura"] == device_ura && row["owner_ura"] != "self"));
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
            crate::core::ura::device_ability_ura("both-view", "dev-1", ABILITY_LIST_ABILITIES)
        );
        assert_eq!(
            hub_list["ability_ura"],
            crate::core::ura::hub_ability_ura("both-view", ABILITY_LIST_ABILITIES)
        );
        assert_ne!(device_list["ability_ura"], hub_list["ability_ura"]);

        let device_describe = invoke_describe(&registry, &device_ura).unwrap();
        let hub_describe = invoke_describe(&registry, &hub_ura).unwrap();
        assert_eq!(device_describe["ura"], device_ura);
        assert_eq!(hub_describe["ura"], hub_ura);
        assert_eq!(
            device_describe["abilities_summary"]["total"], 2,
            "Device describe must summarize its callee-scoped catalogue: {device_describe}"
        );
        assert_eq!(hub_describe["abilities_summary"]["total"], 2);
    }

    #[test]
    fn list_abilities_projects_static_descriptors_to_public_catalog_names() {
        let mut reg = AxonAbilityCatalog::new();
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
    fn list_abilities_stamps_health_metadata_from_monitor_store() {
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

        let mut reg = AxonAbilityCatalog::new();
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
        assert_eq!(
            seeded["metadata"]["health_status"].as_str(),
            Some("unhealthy")
        );
        assert_eq!(
            seeded["metadata"]["health_checked_unix_ms"].as_str(),
            Some("1234")
        );
        assert_eq!(
            seeded["metadata"]["health_detail"].as_str(),
            Some("exit 7: connection refused")
        );

        // A descriptor with no record must NOT grow health keys —
        // absence means "not monitored", never a fabricated state.
        let plain = abilities
            .iter()
            .find(|a| a["name"].as_str() == Some("svc_plain"))
            .expect("plain ability row present");
        assert!(plain["metadata"]
            .as_object()
            .is_none_or(|m| !m.contains_key("health_status")));
    }

    #[test]
    fn list_abilities_realm_scope_includes_hub_published_entries() {
        // RFC-001 v4.1.7 hub-broadcast contract: when the caller
        // passes `scope = "realm"`, the merged catalogue includes
        // entries cached from `federation.{join,heartbeat}`. The
        // default-local path stays disjoint — pin both axes.
        use crate::daemon::federation::client::ability_contract::HubAbilityEntry;
        let hub_published_abilities = HubPublishedAbilityStore::new();

        let mut reg = AxonAbilityCatalog::new();
        super::register(
            &mut reg,
            vec![OwnerKind::Device],
            || vec![d("observe.health")],
            empty_registry_handle(),
            Arc::clone(&hub_published_abilities),
        );
        hub_published_abilities
            .apply_diff(
                crate::daemon::federation::client::ability_contract::HubAbilitiesDiff {
                    revision: 99,
                    added: vec![HubAbilityEntry {
                        name: "test.scope".to_string(),
                        descriptor: serde_json::to_value(
                            AbilityDescriptor::new(
                                "test.scope",
                                &crate::core::ura::hub_ura("test"),
                                Visibility::Public,
                                AdmissionAction::Read,
                            )
                            .expect("canonical hub descriptor")
                            .with_description("smoke entry"),
                        )
                        .expect("hub descriptor json"),
                    }],
                    removed: vec![],
                },
            )
            .expect("canonical hub ability diff");

        // Default scope: hub entry must NOT appear.
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
            "default scope must not leak hub-broadcast entries"
        );

        // Realm scope: hub entry must appear, with `source` stamped.
        let realm_resp = invoke_list(
            &reg,
            "easynet:///r/test/device/01DEV",
            json!({"scope": "realm"}),
        )
        .unwrap();
        let abilities = realm_resp["abilities"].as_array().unwrap();
        let hub_entry = abilities
            .iter()
            .find(|a| a["name"] == "test.scope")
            .expect("test.scope must be in realm-scope output");
        assert_eq!(hub_entry["source"], "hub:broadcast");
        assert!(
            hub_entry["descriptor_ref"]
                .as_str()
                .is_some_and(|descriptor_ref| descriptor_ref
                    .starts_with("easynet:///r/test/ability/authority.test.scope@")),
            "hub-published row must stay canonical: {hub_entry}"
        );
    }

    #[test]
    fn list_abilities_filters_by_agent_ura_and_ability_subject() {
        let alice = "easynet:///r/test-realm/agent/user-1.alice";
        let bob = "easynet:///r/test-realm/agent/user-1.bob";
        let mut reg = AxonAbilityCatalog::new();
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
            json!({ "agent_ura": alice }),
        )
        .unwrap();
        let abilities = by_owner["abilities"].as_array().unwrap();
        assert_eq!(
            abilities.len(),
            2,
            "agent_ura must scope to the selected owner: {by_owner}"
        );
        assert!(abilities.iter().all(|a| a["owner_ura"] == alice));

        let subject = crate::core::ura::owner_ability_ura(alice, "chat").unwrap();
        let by_subject = invoke_list(
            &reg,
            "easynet:///r/test-realm/device/test-device",
            json!({ "subject_ura": subject }),
        )
        .unwrap();
        let abilities = by_subject["abilities"].as_array().unwrap();
        assert_eq!(
            abilities.len(),
            1,
            "full Ability URA subject must scope to one ability: {by_subject}"
        );
        assert_eq!(abilities[0]["name"], "chat");
        assert_eq!(abilities[0]["owner_ura"], alice);

        let err = invoke_list(
            &reg,
            "easynet:///r/test-realm/device/test-device",
            json!({
                "agent_ura": bob,
                "subject_ura": crate::core::ura::owner_ability_ura(alice, "chat").unwrap(),
            }),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("must match"), "got {err}");
    }

    #[test]
    fn list_abilities_rejects_unknown_query_fields() {
        let mut reg = AxonAbilityCatalog::new();
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
    fn live_registry_surfaces_canonical_descriptor_normalized_from_manifest() {
        //
        // Authority identity is injected explicitly. The catalogue must not
        // reconstruct Agent owners from credentials or process-global HOME.
        use crate::daemon::ability::dispatch::{AbilityAuthorityContext, OwnerKind};
        use std::sync::OnceLock;

        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let device_ura = "easynet:///r/alice-realm/device/test-node";
        let alice_ura = crate::core::ura::device_agent_ura("alice-realm", "test-node", "alice");
        let bob_ura = crate::core::ura::device_agent_ura("alice-realm", "test-node", "bob");

        let mut reg = AxonAbilityCatalog::new();
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());

        // Registration imports the manifest into the governed descriptor;
        // meta.list_abilities reads that committed descriptor directly.
        let mut live_reg = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            AbilityAuthorityContext::for_device_authority_root(device_ura)
                .expect("fixed Device authority context"),
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
        // A schema-less static registration imports the pure system catalog
        // metadata before commit; discovery still performs no overlay.
        live_reg.register_rpc_with_owner_and_action(
            "alice.legacy",
            OwnerKind::Agent("alice".to_string()),
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            Arc::new(|_args| Ok(json!({}))),
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
            .find(|a| a["name"] == "alice.chat" && a["owner_ura"] == alice_ura)
            .unwrap_or_else(|| {
                panic!("device-sponsored Agent chat must retain its Agent namespace: {abilities:?}")
            });
        let chat_owners: std::collections::BTreeSet<&str> = abilities
            .iter()
            .filter(|a| {
                a["name"]
                    .as_str()
                    .is_some_and(|name| name.ends_with(".chat"))
            })
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
            .find(|a| a["name"] == "alice.subscribe")
            .expect("agent-owned subscribe must surface as the owner-local ability name");
        assert_eq!(
            subscribe["hints"]["streaming_only"],
            json!(true),
            "non-chat manifest-backed stream abilities must surface streaming_only"
        );
        assert_eq!(subscribe["call_mode"], json!("stream"));
        assert!(subscribe.get("class").is_none());

        let legacy = abilities
            .iter()
            .find(|a| a["name"] == "alice.legacy")
            .expect("agent-owned fallback ability must surface as the owner-local ability name");
        let legacy_desc = legacy["description"].as_str().unwrap_or_default();
        assert_eq!(legacy_desc, "(system ability)");
        assert_eq!(legacy["schema_summary"]["input"]["type"], "object");

        let mcp_search = abilities
            .iter()
            .find(|a| a["name"] == "alice.mcp_search")
            .expect("dynamic MCP ability must surface as the owner-local ability name");
        assert_eq!(
            mcp_search["schema_summary"]["input"]["properties"]["query"]["type"],
            json!("string"),
            "dynamic overlay manifests must be visible to meta.list_abilities"
        );
    }

    #[test]
    fn live_registry_catalog_drops_records_removed_from_control_plane() {
        use crate::daemon::ability::dispatch::{AbilityAuthorityContext, OwnerKind};
        use std::sync::OnceLock;

        let device_ura = "easynet:///r/test/device/01DEV";
        let mut live_reg = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            AbilityAuthorityContext::for_device_authority_root(device_ura)
                .expect("fixed Device authority context"),
        );
        live_reg.register_rpc_with_owner_and_action(
            "device.unowned.test",
            OwnerKind::Device,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            Arc::new(|_args| Ok(json!({}))),
        );
        live_reg.clear_owner_for_test("device.unowned.test");

        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        handle.set(Arc::new(live_reg)).expect("set live registry");

        let mut reg = AxonAbilityCatalog::new();
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
            host_device_agent_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
            hosted_agents: Vec::new(),
        })
        .expect("seed local-agents.json");

        let live_reg = Arc::new(AxonAbilityCatalog::new());
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
                "device.hot.echo",
                OwnerKind::Device,
                manifest,
                Arc::new(|_args| Ok(json!({}))),
            )
            .expect("dynamic RPC manifest registers");

        let mut reg = AxonAbilityCatalog::new();
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
            "easynet:///r/test-realm/ability/device.dev-1.hot.echo"
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
            host_device_agent_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
            hosted_agents: Vec::new(),
        })
        .expect("seed local-agents.json");

        let mut live_reg = AxonAbilityCatalog::new();
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

        let mut reg = AxonAbilityCatalog::new();
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
            host_device_agent_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
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

        let mut reg = AxonAbilityCatalog::new();
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
            host_device_agent_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
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

        let mut reg = AxonAbilityCatalog::new();
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
        let mut reg = AxonAbilityCatalog::new();
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
        let mut reg = AxonAbilityCatalog::new();
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
        // RFC-001 v4.1.7 hub-broadcast contract added the optional
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

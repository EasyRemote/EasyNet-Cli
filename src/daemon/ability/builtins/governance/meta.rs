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

use crate::daemon::ability::catalog::{self as ability_catalog, AbilityDiscoveryHintSnapshot};
use crate::daemon::ability::descriptors::{AbilityDescriptor, AbilityIdentity};
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore;
use serde_json::{json, Value};

pub const ABILITY_DESCRIBE: &str = crate::daemon::ability::names::governance::META_DESCRIBE;
pub const ABILITY_LIST_ABILITIES: &str =
    crate::daemon::ability::names::governance::META_LIST_ABILITIES;

/// Register both meta abilities on the registry.
///
/// `descriptors_provider` runs at handler-call time so future
/// hot-reload of the descriptor catalog is reflected without
/// re-registration. Same closure type as `mcp::bridge::register`
/// so the daemon wires both off `daemon::ability::catalog::profiles`.
///
/// `registry_handle` is a `OnceLock` populated by the build site
/// AFTER `Arc::new(reg)`. The list_abilities handler reads through
/// it to enumerate every CURRENTLY-REGISTERED ability — including
/// abilities registered AFTER `meta_ability::register` ran (e.g.
/// `mission.run`, per-agent executor-bound handlers, hot-materialized
/// agent abilities). The static profile
/// descriptor catalogue is merged on top so first-class abilities
/// (fs.read, http.request, ...) keep their full schemas;
/// runtime-only entries (mission.run, hot-reloaded agent abilities)
/// surface with a synthesized descriptor when the static catalogue
/// has nothing for them. Without this two-source merge, the LLM
/// asking `meta.list_abilities` would see a stale, profile-only view
/// that breaks every "discover then invoke" flow.
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
        crate::daemon::persistence::local_agents::load()
            .map(|local| local.hosted_agents.len())
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
    // A committed control-plane registry is the canonical catalogue. Do not
    // merge in profile or hosted-agent persistence after it exists: doing so
    // makes discovery depend on ambient HOME state and can advertise rows the
    // runtime never registered. The provider path remains an explicit seam
    // only for embedded consumers without a live registry.
    //
    // Hosted-agent/profile persistence is a Device-plane concern in that
    // seam. A Hub callee must never read it in either case.
    let build_context = live_registry
        .is_none()
        .then(|| AbilityCatalogBuildContext::load_for_callee(invocation_callee_ura))
        .flatten();

    // Scope parameter (RFC-001 v4.1.7 hub-broadcast contract):
    //   * `"local"` (default) — only abilities the device owns +
    //     hosts. Same payload shape as before this PR.
    //   * `"realm"` — local set merged with the hub-published cache
    //     (`HubPublishedAbilityStore`), so a peer browsing the
    //     realm sees both device-owned and hub-owned abilities
    //     through one call. Hub entries carry their original
    //     descriptor verbatim — the device does not invent
    //     fields.
    // Static profile descriptors are schema templates, never owner truth.
    // Production profiles may have been built before daemon mode and runtime
    // authority were known (historically they used `owner_ura = "self"`).
    // Live control-plane rows below re-project those schemas onto exact
    // authority roots.
    let static_descriptors = match build_context.as_ref() {
        Some(context) => {
            scoped_static_descriptors(&scope, context).unwrap_or_else(|| descriptors_provider())
        }
        // A live Hub catalogue already has canonical control-plane rows. Its
        // Device-profile provider is not a fallback: invoking it would read
        // local-agents persistence merely to obtain templates Hub rows do not
        // own. Minimal descriptors are synthesized from the authority rows.
        None if live_registry.is_some() => Vec::new(),
        // Embedded consumers without a published live registry deliberately
        // own their provider seam; it remains the sole static source there.
        None => descriptors_provider(),
    };
    let mut catalog: std::collections::BTreeMap<AbilityIdentity, AbilityDescriptor> =
        std::collections::BTreeMap::new();
    let static_templates = StaticDescriptorTemplates::new(&static_descriptors);

    // Dedup invariant (2026-05-26): static descriptors enter the
    // catalog first, then the live-registry phase inserts only when
    // `!catalog.contains_key(&identity)`. Identity is the canonical
    // ability URA, so two hosted agents that both expose `chat` get
    // distinct keys (different owner) and are both retained; a
    // dynamic hot-registration that collides with a static profile
    // (same owner + same public verb) is silently skipped in favour
    // of the static one. Old code de-duplicated on the bare `name`
    // string, which collapsed agent-owned namesakes — see test
    // `list_abilities_keeps_same_public_ability_name_for_multiple_hosted_agents`.
    //
    // Phase 2: live registry. Anything registered into
    // `AxonAbilityCatalog` that the static catalogue does NOT
    // already cover gets a synthesised minimal descriptor. This
    // catches (a) abilities registered AFTER meta_ability itself
    // (mission.run, easynet.* aliases), (b) per-agent verbs that the
    // hot registrar materializes at boot from each agent's workspace
    // `abilities/*.toml`, and (c) any future ability whose author
    // forgot to thread it through the profile catalogue.
    if let Some(registry) = live_registry {
        let hint_snapshot = AbilityDiscoveryHintSnapshot::from_registry(registry);
        // Owner identity comes directly from the committed authority row.
        // No credentials/local-agents reconstruction is permitted here: in
        // Hub mode that ambient state is intentionally absent, and in Both
        // mode a bare-name aggregate would erase the Device/Hub distinction.
        for row in registry.authority_ability_catalog_snapshot() {
            if !authority_row_visible_from_callee(&row, invocation_callee_ura) {
                continue;
            }
            let name = row.name.clone();
            // Keep the live registry on the same public-catalogue surface as
            // `published_abilities`, `easynet ability list`, and advertise.
            if !ability_catalog::is_publishable_catalog_name(&name) {
                continue;
            }
            let transport_hints = hint_snapshot.for_name(&name);
            let descriptor = descriptor_for_authority_row(
                &row,
                static_templates.for_row(&row),
                transport_hints,
            )?;
            if !scope.matches_descriptor(&descriptor) {
                continue;
            }
            let identity = descriptor.identity().ok_or_else(|| {
                anyhow::anyhow!(
                    "meta.list_abilities: authority row `{}` has no descriptor identity",
                    row.ability_ura
                )
            })?;
            catalog.insert(identity, descriptor);
        }

        if let Some(build_context) = build_context.as_ref() {
            synthesize_hot_hosted_agent_descriptors(
                &mut catalog,
                build_context,
                &hint_snapshot,
                &scope,
            );
        }
    } else {
        // Explicit static-provider seam (embedded consumers and unit tests).
        // Only already-canonical descriptors may leave it. A `self` template
        // has schema value but no routable identity, so it is not a wire row.
        for descriptor in static_descriptors {
            if !descriptor_owner_is_canonical(&descriptor)
                || !ability_catalog::is_publishable_catalog_name(&descriptor.name)
                || !scope.matches_descriptor(&descriptor)
            {
                continue;
            }
            if let Some(identity) = descriptor.identity() {
                catalog.insert(identity, descriptor);
            }
        }
    }

    // Final pass — service-health metadata. Applied uniformly over
    // the assembled catalog (static, live-registry, and hosted-synth
    // entries alike) so no insertion path has to remember it. The
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

    // Phase 3: hub-published abilities. Only when the caller asked
    // for realm scope — the default-local path stays byte-identical
    // to pre-v4.1.7. Each entry's `descriptor` is whatever shape
    // the hub published; we surface it verbatim so the
    // hub schema can evolve without forcing a Cli release.
    if scope.include_realm {
        for entry in hub_published_abilities.snapshot() {
            let mut desc = entry.descriptor;
            // Stamp the canonical name on top — hub deployments
            // sometimes omit it inside the descriptor body
            // (relying on the outer key). The merged catalogue's
            // consumers expect a `name` field.
            if let Value::Object(ref mut map) = desc {
                map.entry("name".to_string())
                    .or_insert(Value::String(entry.name.clone()));
                map.entry("source".to_string())
                    .or_insert(Value::String("hub:broadcast".to_string()));
            }
            merged.push(desc);
        }
    }

    scope.apply(&mut merged);
    Ok(json!({ "abilities": merged }))
}

struct StaticDescriptorTemplates {
    by_name: BTreeMap<String, AbilityDescriptor>,
}

impl StaticDescriptorTemplates {
    fn new(descriptors: &[AbilityDescriptor]) -> Self {
        let mut by_name = BTreeMap::new();
        for descriptor in descriptors {
            for key in [descriptor.name.clone(), descriptor.public_name()] {
                by_name.entry(key).or_insert_with(|| descriptor.clone());
            }
        }
        Self { by_name }
    }

    fn for_row(
        &self,
        row: &crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow,
    ) -> Option<AbilityDescriptor> {
        let public_name =
            crate::core::ura::descriptor_public_ability_name(&row.owner_ura, &row.name);
        self.by_name
            .get(&row.name)
            .or_else(|| self.by_name.get(&public_name))
            .cloned()
    }
}

fn authority_row_visible_from_callee(
    row: &crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow,
    invocation_callee_ura: &str,
) -> bool {
    match &row.owner {
        OwnerKind::Device | OwnerKind::Hub => row.owner_ura == invocation_callee_ura,
        OwnerKind::Agent(_) | OwnerKind::User(_) => {
            row.owner_ura == invocation_callee_ura
                || invocation_callee_hosts_subordinate_owners(invocation_callee_ura)
        }
    }
}

fn invocation_callee_hosts_subordinate_owners(invocation_callee_ura: &str) -> bool {
    crate::core::ura::parse_ura(invocation_callee_ura)
        .map(|callee| callee.kind == crate::core::ura::URAKind::Device)
        .unwrap_or(false)
}

fn descriptor_for_authority_row(
    row: &crate::daemon::ability::dispatch::AuthorityAbilityCatalogSnapshotRow,
    template: Option<AbilityDescriptor>,
    transport_hints: crate::daemon::ability::descriptors::AbilityHints,
) -> anyhow::Result<AbilityDescriptor> {
    let public_name = crate::core::ura::descriptor_public_ability_name(&row.owner_ura, &row.name);
    let descriptor = match template {
        Some(mut descriptor) => {
            let template_owner_ura = descriptor.owner_ura.clone();
            descriptor.name = public_name.clone();
            descriptor.owner_ura = row.owner_ura.clone();
            descriptor.scope_subjects = rebind_template_scope_rule(
                descriptor.scope_subjects,
                &template_owner_ura,
                &row.owner_ura,
            );
            descriptor.scope_agents = rebind_template_scope_rule(
                descriptor.scope_agents,
                &template_owner_ura,
                &row.owner_ura,
            );
            descriptor
        }
        None => {
            let descriptor = AbilityDescriptor::new(
                public_name,
                row.owner_ura.clone(),
                crate::daemon::ability::descriptors::Visibility::Scoped,
            )?
            .with_scope_subjects(
                crate::daemon::ability::descriptors::ScopeRule::OnlyMatching(vec![row
                    .owner_ura
                    .clone()]),
            )
            .with_scope_agents(
                crate::daemon::ability::descriptors::ScopeRule::OnlyMatching(vec![row
                    .owner_ura
                    .clone()]),
            );
            match row.manifest.as_ref() {
                Some(manifest) => {
                    let mut descriptor = descriptor
                        .with_version(manifest.descriptor_version())?
                        .with_description(manifest.description())
                        .with_input_schema(manifest.input_schema().clone())
                        .with_hints(transport_hints)
                        .with_source("registry");
                    if let Some(output) = manifest.output_schema() {
                        descriptor = descriptor.with_output_schema(output.clone());
                    }
                    descriptor
                }
                None => descriptor
                    .with_description(
                        "Registered local ability (no manifest schema; pass JSON arguments by \
                         trial or consult the workspace TOML if one exists)",
                    )
                    .with_hints(transport_hints)
                    .with_source("registry"),
            }
        }
    };

    let projected_ability_ura = descriptor.canonical_ability_ura().ok_or_else(|| {
        anyhow::anyhow!(
            "meta.list_abilities: cannot project canonical ability URA for owner `{}` name `{}`",
            descriptor.owner_ura,
            descriptor.name
        )
    })?;
    if projected_ability_ura != row.ability_ura {
        anyhow::bail!(
            "meta.list_abilities: projected ability URA `{projected_ability_ura}` does not match \
             control-plane ability URA `{}`",
            row.ability_ura
        );
    }
    Ok(descriptor)
}

fn rebind_template_scope_rule(
    rule: crate::daemon::ability::descriptors::ScopeRule,
    template_owner_ura: &str,
    authority_owner_ura: &str,
) -> crate::daemon::ability::descriptors::ScopeRule {
    use crate::daemon::ability::descriptors::ScopeRule;

    let ScopeRule::OnlyMatching(values) = rule else {
        return rule;
    };
    ScopeRule::OnlyMatching(
        values
            .into_iter()
            .map(|value| {
                if value == template_owner_ura || value == "self" {
                    return authority_owner_ura.to_string();
                }
                crate::core::ura::public_ability_name_from_ability_ura(template_owner_ura, &value)
                    .and_then(|name| {
                        crate::core::ura::owner_ability_ura(authority_owner_ura, &name)
                    })
                    .unwrap_or(value)
            })
            .collect(),
    )
}

fn descriptor_owner_is_canonical(descriptor: &AbilityDescriptor) -> bool {
    crate::core::ura::parse_ura(&descriptor.owner_ura).is_ok()
        && descriptor
            .canonical_ability_ura()
            .is_some_and(|ability_ura| crate::core::ura::parse_ura(&ability_ura).is_ok())
}

struct AbilityCatalogBuildContext {
    host_node_id: Option<String>,
    local: crate::daemon::persistence::local_agents::LocalAgentsFile,
    agents: Option<crate::daemon::persistence::agent_registry::AgentRegistry>,
}

impl AbilityCatalogBuildContext {
    fn load_for_callee(invocation_callee_ura: &str) -> Option<Self> {
        invocation_callee_hosts_subordinate_owners(invocation_callee_ura).then(Self::load)
    }

    fn load() -> Self {
        let credentials = crate::daemon::persistence::config::load_credentials().ok();
        let host_node_id = credentials
            .as_ref()
            .map(|c| c.node_id.trim().to_string())
            .filter(|s| !s.is_empty());
        Self {
            host_node_id,
            local: crate::daemon::persistence::local_agents::load().unwrap_or_default(),
            agents: crate::daemon::persistence::agent_registry::load_agents().ok(),
        }
    }
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

    fn requested_owner_ura(&self) -> Option<String> {
        self.owner_ura.clone().or_else(|| {
            self.ability_ura
                .as_deref()
                .and_then(|ability_ura| crate::core::ura::AbilitySelector::parse(ability_ura).ok())
                .map(|selector| selector.owner_ura().to_string())
        })
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
            | crate::core::ura::URAKind::Hub
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
        | crate::core::ura::URAKind::Hub
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

fn scoped_static_descriptors(
    scope: &AbilityListScope,
    context: &AbilityCatalogBuildContext,
) -> Option<Vec<AbilityDescriptor>> {
    let owner_ura = scope.requested_owner_ura()?;
    static_descriptors_for_owner(&owner_ura, context)
}

fn static_descriptors_for_owner(
    owner_ura: &str,
    context: &AbilityCatalogBuildContext,
) -> Option<Vec<AbilityDescriptor>> {
    if context.local.host_device_agent_ura == owner_ura {
        return Some(crate::daemon::ability::catalog::profiles::device::descriptors_for(owner_ura));
    }

    if crate::daemon::persistence::local_agents::lookup_hosted_ura(
        &context.local,
        "consent",
        "default",
    )
    .as_deref()
        == Some(owner_ura)
    {
        return Some(
            crate::daemon::ability::catalog::profiles::consent::descriptors_for(owner_ura),
        );
    }

    if crate::daemon::persistence::local_agents::lookup_hosted_ura(&context.local, "mcp", "default")
        .as_deref()
        == Some(owner_ura)
    {
        return Some(crate::daemon::ability::catalog::profiles::mcp::descriptors_for(owner_ura));
    }

    let llm_owner = context
        .local
        .hosted_agents
        .iter()
        .any(|entry| entry.profile == "llm" && entry.agent_ura == owner_ura);
    if llm_owner {
        let catalog =
            crate::daemon::ability::catalog::profiles::llm::LlmProfileAbilityCatalog::load();
        return Some(
            crate::daemon::ability::catalog::profiles::llm::descriptors_for_with_catalog(
                owner_ura, None, &catalog,
            ),
        );
    }

    None
}

fn synthesize_hot_hosted_agent_descriptors(
    catalog: &mut std::collections::BTreeMap<AbilityIdentity, AbilityDescriptor>,
    context: &AbilityCatalogBuildContext,
    hint_snapshot: &AbilityDiscoveryHintSnapshot,
    scope: &AbilityListScope,
) {
    use crate::daemon::ability::descriptors::Visibility;

    let Some(agents) = context.agents.as_ref() else {
        return;
    };

    for (agent_name, entry) in &agents.agents {
        let Some(owner_ura) = crate::daemon::persistence::local_agents::lookup_hosted_ura(
            &context.local,
            "llm",
            agent_name,
        ) else {
            continue;
        };
        if scope
            .owner_ura
            .as_deref()
            .is_some_and(|scope_owner| scope_owner != owner_ura)
        {
            continue;
        }
        if crate::core::ura::parse_ura(&owner_ura)
            .map(|u| u.kind != crate::core::ura::URAKind::Agent)
            .unwrap_or(true)
        {
            continue;
        }

        let default_chat_name =
            crate::daemon::ability::manifest::default_chat_manifest().qualified_name(&agent_name);
        for spec in
            crate::daemon::execution::mission::agent_ability_specs::abilities_for_publication(
                &agent_name,
                &entry,
            )
        {
            let public_name =
                crate::core::ura::descriptor_public_ability_name(&owner_ura, spec.name());
            if public_name.is_empty() {
                continue;
            }
            let Ok(mut descriptor) =
                AbilityDescriptor::new(public_name.clone(), &owner_ura, Visibility::Scoped)
            else {
                continue;
            };
            descriptor = descriptor
                .with_description(spec.description())
                .with_input_schema(spec.parameters().clone())
                .with_hints(hint_snapshot.for_name(spec.name()))
                .with_source(format!("agent:{agent_name}"))
                .with_metadata_entry("runtime", entry.agent_type.to_string())
                .with_metadata_entry("agent_type", entry.agent_type.to_string())
                .with_metadata_entry("base_runtime", entry.agent_type.to_string());
            if let Some(model) = entry.model.as_ref() {
                descriptor = descriptor
                    .with_metadata_entry("model", model.clone())
                    .with_metadata_entry("base_model", model.clone());
            }
            if let Some(node_id) = context.host_node_id.as_ref() {
                descriptor = descriptor.with_metadata_entry("host_node_id", node_id.clone());
            }
            if spec.name() == default_chat_name {
                let chat_manifest = crate::daemon::ability::manifest::default_chat_manifest();
                if let Some(output_schema) = chat_manifest.output_schema() {
                    descriptor = descriptor.with_output_schema(output_schema.clone());
                }
            }
            if !scope.matches_descriptor(&descriptor) {
                continue;
            }
            insert_or_upgrade_hosted_descriptor(catalog, descriptor);
        }
    }
}

/// Insert a manifest-backed hosted-agent descriptor, upgrading any
/// schema-less stub already catalogued under the same identity.
///
/// The live-registry pass (Phase 2) runs before the hosted-agent synth
/// and inserts name-only stubs for registration sites that carried no
/// manifest — the hot agent registrar does not (yet) register
/// `_with_spec`, so every TOML-declared agent ability lands there with
/// `schema_summary.input == Null`. The on-disk manifest is the
/// authoritative contract source for these abilities; discarding the
/// synth descriptor on key collision is what made the Frontend render
/// "No input required" for abilities that do declare an input schema.
/// An existing entry that already carries a schema (registered
/// `_with_spec`) keeps winning.
fn insert_or_upgrade_hosted_descriptor(
    catalog: &mut std::collections::BTreeMap<AbilityIdentity, AbilityDescriptor>,
    descriptor: AbilityDescriptor,
) {
    let Some(identity) = descriptor.identity() else {
        return;
    };
    if catalog
        .get(&identity)
        .is_some_and(|existing| !existing.schema_summary.input.is_null())
    {
        return;
    }
    catalog.insert(identity, descriptor);
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
    use crate::daemon::ability::descriptors::{AbilityDescriptor, ScopeRule, Visibility};

    fn d(name: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(name, "easynet:///r/test/device/01DEV", Visibility::Public)
            .expect("test descriptor")
    }

    fn d_for_owner(name: &str, owner_ura: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(name, owner_ura, Visibility::Scoped).expect("test descriptor")
    }

    #[test]
    fn hosted_descriptor_synth_upgrades_schema_less_stub_and_keeps_schemad_entry() {
        let owner = "easynet:///r/localhost/agent/dev.demo";
        let stub = d_for_owner("dev.demo.n8n_hello", owner)
            .with_description("Registered local ability (no manifest schema)");
        let manifest_backed = d_for_owner("dev.demo.n8n_hello", owner)
            .with_description("Trigger the n8n easynet-hello workflow via webhook")
            .with_input_schema(json!({
                "type": "object",
                "required": ["name"],
                "properties": {"name": {"type": "string"}},
            }));
        let identity = manifest_backed.identity().expect("identity");

        // Phase 2 stub first, synth second: the manifest schema must win.
        let mut catalog = std::collections::BTreeMap::new();
        insert_or_upgrade_hosted_descriptor(&mut catalog, stub.clone());
        insert_or_upgrade_hosted_descriptor(&mut catalog, manifest_backed.clone());
        assert!(
            !catalog[&identity].schema_summary.input.is_null(),
            "manifest-backed descriptor must upgrade the schema-less stub"
        );

        // An entry that already carries a schema is never downgraded.
        insert_or_upgrade_hosted_descriptor(&mut catalog, stub);
        assert_eq!(
            catalog[&identity].description,
            "Trigger the n8n easynet-hello workflow via webhook"
        );
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

    fn self_meta_templates() -> Vec<AbilityDescriptor> {
        [ABILITY_DESCRIBE, ABILITY_LIST_ABILITIES]
            .into_iter()
            .map(|name| {
                AbilityDescriptor::new(name, "self", Visibility::Scoped)
                    .expect("self descriptor template")
                    .with_scope_subjects(ScopeRule::OnlyMatching(vec!["self".to_string()]))
                    .with_scope_agents(ScopeRule::OnlyMatching(vec!["self".to_string()]))
            })
            .collect()
    }

    fn authority_bound_meta_registry(
        authority_context: crate::daemon::ability::dispatch::AbilityAuthorityContext,
        owners: Vec<OwnerKind>,
    ) -> Arc<AxonAbilityCatalog> {
        let handle = Arc::new(std::sync::OnceLock::new());
        let mut registry = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            easynet_axon::invocation::LocalRuntime::new(),
            authority_context,
        );
        super::register(
            &mut registry,
            owners,
            self_meta_templates,
            Arc::clone(&handle),
            HubPublishedAbilityStore::new(),
        );
        let registry = Arc::new(registry);
        handle
            .set(Arc::clone(&registry))
            .expect("publish authority-bound meta registry");
        registry
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
        assert_eq!(list["scope_subjects"]["uras"], json!([hub_ura]));
        assert_eq!(list["scope_agents"]["uras"], json!([hub_ura]));

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
    fn hub_callee_does_not_load_device_catalog_build_context() {
        let hub_ura = crate::core::ura::hub_ura("hub-view");
        assert!(
            AbilityCatalogBuildContext::load_for_callee(&hub_ura).is_none(),
            "Hub catalogue projection must not read Device credentials or hosted-agent persistence"
        );
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
            easynet_axon::invocation::LocalRuntime::new(),
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
        assert!(rows.iter().all(|row| row["source"] == "registry"));
        assert!(rows.iter().all(|row| {
            row["ability_ura"].as_str().is_some_and(|ability_ura| {
                ability_ura.starts_with("easynet:///r/hub-no-provider/ability/hub.")
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
        hub_published_abilities.apply_diff(
            crate::daemon::federation::client::ability_contract::HubAbilitiesDiff {
                revision: 99,
                added: vec![HubAbilityEntry {
                    name: "hub.test.scope".to_string(),
                    descriptor: serde_json::json!({
                        "name": "hub.test.scope",
                        "description": "smoke entry"
                    }),
                }],
                removed: vec![],
            },
        );

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
            !local_names.contains(&"hub.test.scope".to_string()),
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
            .find(|a| a["name"] == "hub.test.scope")
            .expect("hub.test.scope must be in realm-scope output");
        assert_eq!(hub_entry["source"], "hub:broadcast");
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
    fn live_registry_synth_surfaces_input_schema_when_manifest_registered() {
        // Pinning the new `register_*_with_spec` contract: when an
        // ability lands in the live registry with an
        // `AbilityManifest`, the synthesised descriptor on
        // `meta.list_abilities` carries the manifest's
        // description + input_schema verbatim. Without this, the
        // Frontend `InvokeAbilityDialog` falls back to "no
        // declared schema" for chat abilities and the user sees a
        // free-text JSON box with no hint about the args shape.
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

        // Live registry entry registered WITH a manifest. We use
        // a freshly-built `AxonAbilityCatalog` here (not the
        // one `register` runs against) and then publish it
        // through the OnceLock seam so the synth path picks it up.
        let mut live_reg = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            easynet_axon::invocation::LocalRuntime::new(),
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
        live_reg.register_stream_with_owner(
            "alice.subscribe",
            OwnerKind::Agent("alice".to_string()),
            Arc::new(|_args| {
                Ok(crate::daemon::ability::dispatch::StreamSource::Snapshot(
                    Vec::new(),
                ))
            }),
        );
        // A second entry registered the legacy way (no manifest)
        // exercises the fallback arm so we know synth still emits
        // the name-only stub when the manifest is absent.
        live_reg.register_rpc_with_owner(
            "alice.legacy",
            OwnerKind::Agent("alice".to_string()),
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
                .any(|row| row.name == "alice.chat" && row.owner_ura == alice_ura),
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
                panic!(
                    "agent-owned chat must surface as the owner-local ability name: {abilities:?}"
                )
            });
        let chat_owners: std::collections::BTreeSet<&str> = abilities
            .iter()
            .filter(|a| matches!(a["name"].as_str(), Some("alice.chat" | "bob.chat")))
            .filter_map(|a| a["owner_ura"].as_str())
            .collect();
        assert!(
            chat_owners.contains(alice_ura.as_str()) && chat_owners.contains(bob_ura.as_str()),
            "agent-scoped public names must preserve one descriptor per owner, got: {chat_owners:?}"
        );
        // Description must be the manifest's description, not the
        // generic "no manifest schema" stub.
        let desc = chat["description"].as_str().unwrap_or_default();
        assert!(
            !desc.contains("no manifest schema"),
            "description must come from the manifest, got: {desc:?}"
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
            "chat manifest declares `prompt` as a property; synth must surface it. \
             Got: {input_schema}"
        );
        assert_eq!(
            chat["hints"]["streaming_only"],
            json!(false),
            "chat stays on the unary/OpenAI control-plane path for now"
        );

        let subscribe = abilities
            .iter()
            .find(|a| a["name"] == "alice.subscribe")
            .expect("agent-owned subscribe must surface as the owner-local ability name");
        assert_eq!(
            subscribe["hints"]["streaming_only"],
            json!(true),
            "non-chat manifest-backed stream abilities must surface streaming_only"
        );
        assert_eq!(
            subscribe["class"],
            json!("stream"),
            "ability class is derived from the descriptor interface, not inferred by consumers"
        );

        let legacy = abilities
            .iter()
            .find(|a| a["name"] == "alice.legacy")
            .expect("agent-owned fallback ability must surface as the owner-local ability name");
        // Legacy register path leaves the input schema empty —
        // synth falls back to the name-only stub.
        let legacy_desc = legacy["description"].as_str().unwrap_or_default();
        assert!(
            legacy_desc.contains("no manifest schema"),
            "abilities registered without a manifest keep the fallback description, \
             got: {legacy_desc:?}"
        );

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
            easynet_axon::invocation::LocalRuntime::new(),
            AbilityAuthorityContext::for_device_authority_root(device_ura)
                .expect("fixed Device authority context"),
        );
        live_reg.register_rpc_with_owner(
            "device.unowned.test",
            OwnerKind::Device,
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
        assert_eq!(ability["class"], json!("query"));
        assert_eq!(ability["source"], json!("registry"));
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
        let live_registry = AxonAbilityCatalog::new();
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

        let live_registry = AxonAbilityCatalog::new();
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
                    d("session.list"),
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

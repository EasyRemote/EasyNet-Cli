// EasyNet CLI — meta.{describe, list_abilities} ability handlers
// =================================================================
//
// File: src/runtime/agents/meta_ability.rs
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

use crate::runtime::ability_descriptor::{AbilityDescriptor, AbilityIdentity};
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::services::hub_published_ability_store::HubPublishedAbilityStore;
use serde_json::{json, Value};

pub const ABILITY_DESCRIBE: &str = "meta.describe";
pub const ABILITY_LIST_ABILITIES: &str = "meta.list_abilities";

/// Register both meta abilities on the registry.
///
/// `descriptors_provider` runs at handler-call time so future
/// hot-reload of the descriptor catalog is reflected without
/// re-registration. Same closure type as `mcp_bridge_ability::register`
/// so the daemon wires both off `profiles::load_host_descriptors`.
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
    descriptors_provider: F,
    registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    pages_user: Option<String>,
    hub_published_abilities: Arc<HubPublishedAbilityStore>,
) where
    F: Fn() -> Vec<AbilityDescriptor> + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync> =
        Arc::new(descriptors_provider);
    let p_for_describe = Arc::clone(&provider);
    reg.register_rpc_with_owner(
        ABILITY_DESCRIBE,
        OwnerKind::Device,
        Arc::new(move |_args: Value| describe_handler(&p_for_describe)),
    );
    let p_for_list = Arc::clone(&provider);
    let handle_for_list = Arc::clone(&registry_handle);
    let hub_published_abilities_for_list = Arc::clone(&hub_published_abilities);
    // Capture the pages-user identity at registration time so the
    // synth path doesn't read EASYNET_PAGES_USER on every call.
    // Production passes the same value the registry build used;
    // tests pass `None` for unpaired-daemon shape or an explicit
    // string for paired-daemon shape, deterministic either way.
    let pages_user_for_list = pages_user.clone();
    let list_handler: crate::runtime::ability_dispatch::LocalRpcHandler =
        Arc::new(move |args: Value| {
            list_abilities_handler(
                &p_for_list,
                &handle_for_list,
                args,
                pages_user_for_list.as_deref(),
                &hub_published_abilities_for_list,
            )
        });
    reg.register_rpc_with_owner(ABILITY_LIST_ABILITIES, OwnerKind::Device, list_handler);
}

fn describe_handler(
    descriptors_provider: &Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync>,
) -> anyhow::Result<Value> {
    let descriptors = descriptors_provider();

    // Identity comes from local-agents.json. Pre-join state surfaces
    // as ura:"self" so a caller still sees a well-formed describe
    // response — they can re-poll after the daemon completes join.
    let local = crate::persistence::local_agents::load().unwrap_or_default();
    let host_ura = if local.host_device_agent_ura.is_empty() {
        "self".to_string()
    } else {
        local.host_device_agent_ura.clone()
    };
    let signing_authority = if local.host_device_agent_ura.is_empty() {
        "unprovisioned" // pre-join: no key bound yet
    } else {
        "self" // device-profile is Model A (own keypair)
    };

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
    for d in &descriptors {
        let ns = d
            .name
            .split_once('.')
            .map(|(ns, _)| ns.to_string())
            .unwrap_or_else(|| "(no-namespace)".to_string());
        *by_namespace.entry(ns).or_insert(0) += 1;
    }

    Ok(json!({
        "ura": host_ura,
        "identity_summary": {
            "signing_authority": signing_authority,
        },
        "abilities_summary": {
            "total": descriptors.len(),
            "by_namespace": by_namespace,
        },
        "metadata": {
            "hosted_agent_count": local.hosted_agents.len(),
        },
    }))
}

fn list_abilities_handler(
    descriptors_provider: &Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync>,
    registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    args: Value,
    pages_user: Option<&str>,
    hub_published_abilities: &HubPublishedAbilityStore,
) -> anyhow::Result<Value> {
    use crate::runtime::ability_descriptor::Visibility;
    let scope = AbilityListScope::from_args(&args)?;

    // Scope parameter (RFC-001 v4.1.7 hub-broadcast contract):
    //   * `"local"` (default) — only abilities the device owns +
    //     hosts. Same payload shape as before this PR.
    //   * `"realm"` — local set merged with the hub-published cache
    //     (`HubPublishedAbilityStore`), so a peer browsing the
    //     realm sees both device-owned and hub-owned abilities
    //     through one call. Hub entries carry their original
    //     descriptor verbatim — the device does not invent
    //     fields.
    // Phase 1: static profile descriptors (fs.*, http.*, shell.*,
    // <agent>.chat, …). These carry full input/output schemas and
    // descriptions read off the workspace ability TOMLs. We index by
    // the descriptor's canonical ability URA so two hosted agents can
    // both expose `chat` without one collapsing the other.
    let static_descriptors = descriptors_provider();
    let mut catalog: std::collections::BTreeMap<AbilityIdentity, AbilityDescriptor> =
        std::collections::BTreeMap::new();
    for d in static_descriptors {
        if !crate::runtime::agents::is_publishable_catalog_name(&d.name) {
            continue;
        }
        let Some(identity) = d.identity() else {
            continue;
        };
        catalog.insert(identity, d);
    }

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
    if let Some(registry) = registry_handle.get() {
        // Owner URAs for the synthesised descriptors below. These
        // entries are abilities registered into `AxonAbilityCatalog`
        // that no static profile descriptor covers (RFC-002
        // `device.keyring.*`, the runtime `easynet.*` / `mission.*`
        // aliases, hot-materialized per-agent entries, …). The owner-kind
        // for each ability is read from the registry's
        // `lookup_owner` table (M0 of the system-namespace
        // migration); we resolve that kind to an authoritative URA
        // here using credentials + local-agents.json:
        //
        //   * `OwnerKind::Hub`   → realm hub URA, derived from
        //                          `credentials.realm` via
        //                          `crate::ura::hub_ura`.
        //   * `OwnerKind::Device` → host device URA, read from
        //                          `local-agents.json::host_device_agent_ura`.
        //   * `OwnerKind::Agent(id)` → agent URA, composed from the
        //                          realm + canonical user-id +
        //                          the agent_id captured at register
        //                          time.
        //   * `OwnerKind::User(id)` → user URA, composed from the
        //                          realm + the user-id captured at
        //                          register time.
        //
        // All four sources are read independently — we do not
        // "derive" one from another, because they answer different
        // questions and conflating them ships a lie when one is
        // present but the others are not.
        //
        // Pre-join (the relevant source missing) we DROP the entry
        // rather than stamp a placeholder URA. A daemon that has
        // not joined a realm has no canonical owner to advertise;
        // a synth that invents one would feed bad URAs into every
        // downstream consumer (`mcp.bridge.list_tools`,
        // `easynet ability list`, federation `advertise_abilities`,
        // …). Dropping is the honest answer — once `easynet device
        // pair` finishes, the next list_abilities call sees the
        // populated state and emits the full catalogue.
        let realm = crate::persistence::config::load_credentials()
            .ok()
            .map(|c| c.realm.trim().to_string())
            .filter(|s| !s.is_empty());
        let local = crate::persistence::local_agents::load().unwrap_or_default();
        let device_owner_ura = if local.host_device_agent_ura.is_empty() {
            None
        } else {
            Some(local.host_device_agent_ura.clone())
        };
        let hub_owner_ura = realm.as_deref().map(crate::ura::hub_ura);
        // user-segment used for `OwnerKind::Agent(...)` resolution.
        // Captured at registration time from the same
        // `PagesIdentity` the registry build used, so the synth
        // and registration paths agree on which user-id is
        // canonical without reading process-wide env state on
        // each invocation.
        let user_segment = pages_user.map(str::to_string);
        for name in registry.list_abilities() {
            // Keep the live registry on the same public-catalogue surface as
            // `published_abilities`, `easynet ability list`, and advertise.
            if !crate::runtime::agents::is_publishable_catalog_name(&name) {
                continue;
            }
            // M0 commit 2: read the owner kind from the registry,
            // not by sniffing the name string. Compose the wire URA
            // from the kind. Falling through to None on missing
            // metadata is intentional — synth drops entries it
            // cannot stamp authoritatively.
            let owner_string = match registry.lookup_owner(&name) {
                Some(crate::runtime::ability_dispatch::OwnerKind::Hub) => hub_owner_ura.clone(),
                Some(crate::runtime::ability_dispatch::OwnerKind::Device) => {
                    device_owner_ura.clone()
                }
                Some(crate::runtime::ability_dispatch::OwnerKind::Agent(agent_id)) => {
                    match (realm.as_deref(), user_segment.as_deref()) {
                        (Some(r), Some(u)) => Some(crate::ura::agent_ura(r, u, &agent_id)),
                        _ => None,
                    }
                }
                Some(crate::runtime::ability_dispatch::OwnerKind::User(user_id)) => {
                    realm.as_deref().map(|r| crate::ura::user_ura(r, &user_id))
                }
                None => None,
            };
            let Some(owner) = owner_string.as_deref() else {
                continue;
            };
            let public_name = crate::ura::owner_local_ability_name(owner, &name);
            let transport_hints = crate::runtime::agents::discovery_hints_for(registry, &name);
            // Synthesised descriptor. When the registration site
            // landed an `AbilityManifest` via `register_*_with_spec`
            // (chat ability + the family that follows it), surface
            // its description + input_schema + output_schema so the
            // Frontend `InvokeAbilityDialog` renders a SchemaForm
            // and `meta.list_abilities` consumers see the same
            // schema as the static profile catalogue. When no
            // manifest is present (the bulk of system abilities,
            // pending the M0 follow-through that converts every
            // register site to `_with_spec`), fall back to the
            // name-only stub the synth has emitted since the
            // 2026-05-05 owner-aware refactor.
            if let Ok(d) = AbilityDescriptor::new(public_name.clone(), owner, Visibility::Scoped) {
                let descriptor = match registry.manifest_for_dynamic(&name) {
                    Some(manifest) => {
                        let mut d = d
                            .with_description(manifest.description())
                            .with_input_schema(manifest.input_schema().clone())
                            .with_hints(transport_hints.clone())
                            .with_source("registry");
                        if let Some(out) = manifest.output_schema() {
                            d = d.with_output_schema(out.clone());
                        }
                        d
                    }
                    None => d
                        .with_description(
                            "Registered local ability (no manifest schema; \
                             pass JSON arguments by trial or consult the \
                             workspace TOML if one exists)",
                        )
                        .with_hints(transport_hints)
                        .with_source("registry"),
                };
                let Some(identity) = descriptor.identity() else {
                    continue;
                };
                if catalog.contains_key(&identity) {
                    continue;
                }
                catalog.insert(identity, descriptor);
            }
        }

        synthesize_hot_hosted_agent_descriptors(&mut catalog, registry, &local);
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
                .and_then(|ura| crate::services::ability_health::snapshot(&ura))
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
}

struct AbilitySubjectScope {
    owner_ura: Option<String>,
    ability_ura: Option<String>,
}

impl AbilitySubjectScope {
    fn parse(subject_ura: &str) -> anyhow::Result<Self> {
        let parsed = crate::ura::parse_ura(subject_ura).map_err(|e| {
            anyhow::anyhow!("meta.list_abilities: invalid subject_ura {subject_ura:?}: {e}")
        })?;
        match parsed.kind {
            crate::ura::URAKind::Ability => Ok(Self {
                owner_ura: None,
                ability_ura: Some(subject_ura.to_string()),
            }),
            crate::ura::URAKind::Agent
            | crate::ura::URAKind::Device
            | crate::ura::URAKind::Hub
            | crate::ura::URAKind::User => Ok(Self {
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
    let parsed = crate::ura::parse_ura(ura)
        .map_err(|e| anyhow::anyhow!("meta.list_abilities: invalid {field} {ura:?}: {e}"))?;
    match parsed.kind {
        crate::ura::URAKind::Agent
        | crate::ura::URAKind::Device
        | crate::ura::URAKind::Hub
        | crate::ura::URAKind::User => Ok(()),
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
                    crate::ura::public_ability_name_from_ability_ura(&owner_ura, ability_ura)
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

fn synthesize_hot_hosted_agent_descriptors(
    catalog: &mut std::collections::BTreeMap<AbilityIdentity, AbilityDescriptor>,
    registry: &AxonAbilityCatalog,
    local: &crate::persistence::local_agents::LocalAgentsFile,
) {
    use crate::runtime::ability_descriptor::Visibility;

    let Ok(agents) = crate::registry::agents::load_agents() else {
        return;
    };
    let host_node_id = crate::persistence::config::load_credentials()
        .ok()
        .map(|c| c.node_id.trim().to_string())
        .filter(|s| !s.is_empty());

    for (agent_name, entry) in agents.agents {
        let Some(owner_ura) =
            crate::persistence::local_agents::lookup_hosted_ura(local, "llm", &agent_name)
        else {
            continue;
        };
        if crate::ura::parse_ura(&owner_ura)
            .map(|u| u.kind != crate::ura::URAKind::Agent)
            .unwrap_or(true)
        {
            continue;
        }

        let default_chat_name =
            crate::core::ability_spec::default_chat_manifest().qualified_name(&agent_name);
        for spec in crate::runtime::abilities::abilities_for_publication(&agent_name, &entry) {
            let public_name = crate::ura::owner_local_ability_name(&owner_ura, spec.name());
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
                .with_hints(crate::runtime::agents::discovery_hints_for(
                    registry,
                    spec.name(),
                ))
                .with_source(format!("agent:{agent_name}"))
                .with_metadata_entry("runtime", entry.agent_type.to_string())
                .with_metadata_entry("agent_type", entry.agent_type.to_string())
                .with_metadata_entry("base_runtime", entry.agent_type.to_string());
            if let Some(model) = entry.model.as_ref() {
                descriptor = descriptor
                    .with_metadata_entry("model", model.clone())
                    .with_metadata_entry("base_model", model.clone());
            }
            if let Some(node_id) = host_node_id.as_ref() {
                descriptor = descriptor.with_metadata_entry("host_node_id", node_id.clone());
            }
            if spec.name() == default_chat_name {
                let chat_manifest = crate::core::ability_spec::default_chat_manifest();
                if let Some(output_schema) = chat_manifest.output_schema() {
                    descriptor = descriptor.with_output_schema(output_schema.clone());
                }
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
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};

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
        crate::persistence::config::save_credentials(&crate::persistence::config::Credentials {
            node_id: node_id.to_string(),
            credential_token: "test-token".to_string(),
            hub_endpoint: "axon://hub.test:50051".to_string(),
            realm: realm.to_string(),
            username: Some(username.to_string()),
            ..Default::default()
        })
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
        pages_user: Option<String>,
    ) where
        F: Fn() -> Vec<AbilityDescriptor> + Send + Sync + 'static,
    {
        super::register(
            reg,
            descriptors_provider,
            registry_handle,
            pages_user,
            HubPublishedAbilityStore::new(),
        );
    }

    #[test]
    fn registration_makes_both_abilities_dispatchable() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, Vec::new, empty_registry_handle(), None);
        assert!(reg.get_rpc(ABILITY_DESCRIBE).is_some());
        assert!(reg.get_rpc(ABILITY_LIST_ABILITIES).is_some());
        // The legacy `device.easynet.discover` alias was removed
        // in RFC-001 v4.1.7 M2. The canonical name is the only
        // surface; assert the legacy literal is NOT registered so
        // future regressions that re-introduce the alias trip here.
        assert!(reg.get_rpc("device.easynet.discover").is_none());
    }

    #[test]
    fn list_abilities_projects_static_descriptors_to_public_catalog_names() {
        let mut reg = AxonAbilityCatalog::new();
        register(
            &mut reg,
            || vec![d("observe.health"), d("agent.list")],
            empty_registry_handle(),
            None,
        );
        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();
        let resp = handler(json!({})).unwrap();
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
        use crate::services::ability_health::{self, AbilityHealthRecord, HealthStatus};

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
            None,
        );
        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();
        let resp = handler(json!({})).unwrap();
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
        use crate::runtime::federation_client::HubAbilityEntry;
        let hub_published_abilities = HubPublishedAbilityStore::new();

        let mut reg = AxonAbilityCatalog::new();
        super::register(
            &mut reg,
            || vec![d("observe.health")],
            empty_registry_handle(),
            None,
            Arc::clone(&hub_published_abilities),
        );
        hub_published_abilities.apply_diff(crate::runtime::federation_client::HubAbilitiesDiff {
            revision: 99,
            added: vec![HubAbilityEntry {
                name: "hub.test.scope".to_string(),
                descriptor: serde_json::json!({
                    "name": "hub.test.scope",
                    "description": "smoke entry"
                }),
            }],
            removed: vec![],
        });

        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();

        // Default scope: hub entry must NOT appear.
        let local_resp = handler(json!({})).unwrap();
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
        let realm_resp = handler(json!({"scope": "realm"})).unwrap();
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
            None,
        );
        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();

        let by_owner = handler(json!({ "agent_ura": alice })).unwrap();
        let abilities = by_owner["abilities"].as_array().unwrap();
        assert_eq!(
            abilities.len(),
            2,
            "agent_ura must scope to the selected owner: {by_owner}"
        );
        assert!(abilities.iter().all(|a| a["owner_ura"] == alice));

        let subject = crate::ura::owner_ability_ura(alice, "chat").unwrap();
        let by_subject = handler(json!({ "subject_ura": subject })).unwrap();
        let abilities = by_subject["abilities"].as_array().unwrap();
        assert_eq!(
            abilities.len(),
            1,
            "full Ability URA subject must scope to one ability: {by_subject}"
        );
        assert_eq!(abilities[0]["name"], "chat");
        assert_eq!(abilities[0]["owner_ura"], alice);

        let err = handler(json!({
            "agent_ura": bob,
            "subject_ura": crate::ura::owner_ability_ura(alice, "chat").unwrap(),
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("must match"), "got {err}");
    }

    #[test]
    fn list_abilities_rejects_unknown_query_fields() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, Vec::new, empty_registry_handle(), None);
        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();
        let err = handler(json!({ "agent_id": "legacy" }))
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
        // Fixture isolation: the synth path drops Agent-owned
        // descriptors when `realm` is missing (no
        // credentials.json), so we point HOME at an empty dir
        // (HomeGuard) AND write a minimal credentials.json so
        // realm resolves to "alice-realm". Without this fixture,
        // the test passes when run alone (because it leaks the
        // developer's real $HOME credentials.json) and fails when
        // run with siblings that HomeGuard a clean dir — which is
        // the race we're closing.
        use crate::runtime::ability_dispatch::OwnerKind;
        use std::sync::OnceLock;

        let _home = crate::facade::cli::test_support::HomeGuard::new();
        seed_test_credentials("alice-realm", "test-node", "alice");

        let mut reg = AxonAbilityCatalog::new();
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());

        // Live registry entry registered WITH a manifest. We use
        // a freshly-built `AxonAbilityCatalog` here (not the
        // one `register` runs against) and then publish it
        // through the OnceLock seam so the synth path picks it up.
        let mut live_reg = AxonAbilityCatalog::new();
        live_reg.register_stream_with_spec(
            "alice.chat",
            OwnerKind::Agent("alice".to_string()),
            crate::core::ability_spec::default_chat_manifest(),
            Arc::new(|_args| {
                Ok(crate::runtime::ability_dispatch::StreamSource::Snapshot(
                    Vec::new(),
                ))
            }),
        );
        live_reg.register_stream_with_spec(
            "bob.chat",
            OwnerKind::Agent("bob".to_string()),
            crate::core::ability_spec::default_chat_manifest(),
            Arc::new(|_args| {
                Ok(crate::runtime::ability_dispatch::StreamSource::Snapshot(
                    Vec::new(),
                ))
            }),
        );
        live_reg.register_stream_with_owner(
            "alice.subscribe",
            OwnerKind::Agent("alice".to_string()),
            Arc::new(|_args| {
                Ok(crate::runtime::ability_dispatch::StreamSource::Snapshot(
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
                crate::core::ability_spec::AbilityManifest::new(
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
                    Ok(crate::runtime::ability_dispatch::StreamSource::Snapshot(
                        Vec::new(),
                    ))
                }),
            )
            .expect("dynamic stream manifest registers");
        handle.set(Arc::new(live_reg)).expect("set OnceLock");

        register(&mut reg, Vec::new, handle, Some("user-1".to_string()));
        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();
        let resp = handler(json!({})).unwrap();
        let abilities = resp["abilities"].as_array().unwrap();

        let chat = abilities
            .iter()
            .find(|a| {
                a["name"] == "chat"
                    && a["owner_ura"] == "easynet:///r/alice-realm/agent/user-1.alice"
            })
            .expect("agent-owned chat must surface as the owner-local ability name");
        let chat_owners: std::collections::BTreeSet<&str> = abilities
            .iter()
            .filter(|a| a["name"] == "chat")
            .filter_map(|a| a["owner_ura"].as_str())
            .collect();
        assert!(
            chat_owners.contains("easynet:///r/alice-realm/agent/user-1.alice")
                && chat_owners.contains("easynet:///r/alice-realm/agent/user-1.bob"),
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
            .find(|a| a["name"] == "subscribe")
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
            .find(|a| a["name"] == "legacy")
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
            .find(|a| a["name"] == "mcp_search")
            .expect("dynamic MCP ability must surface as the owner-local ability name");
        assert_eq!(
            mcp_search["schema_summary"]["input"]["properties"]["query"]["type"],
            json!("string"),
            "dynamic overlay manifests must be visible to meta.list_abilities"
        );
    }

    #[test]
    fn live_registry_synth_drops_entries_without_owner_metadata() {
        use crate::runtime::ability_dispatch::OwnerKind;
        use std::sync::OnceLock;

        let mut live_reg = AxonAbilityCatalog::new();
        live_reg.register_rpc_with_owner(
            "device.unowned.test",
            OwnerKind::Device,
            Arc::new(|_args| Ok(json!({}))),
        );
        live_reg.clear_owner_for_test("device.unowned.test");

        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        handle.set(Arc::new(live_reg)).expect("set live registry");

        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, Vec::new, handle, None);
        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();
        let resp = handler(json!({})).unwrap();
        let names: Vec<_> = resp["abilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a["name"].as_str())
            .collect();

        assert!(
            !names.contains(&"device.unowned.test"),
            "meta.list_abilities must not synthesize an owner for entries missing registry metadata"
        );
    }

    #[test]
    fn list_abilities_surfaces_dynamic_manifest_schema_for_hot_registered_tools() {
        // Hot MCP reload writes handlers + manifests through the
        // dynamic side table. `meta.list_abilities` is the user-facing
        // catalogue backing SchemaForm, so it must read static OR
        // dynamic manifests rather than the static-only map.
        use crate::persistence::local_agents::{save, LocalAgentsFile};
        use crate::runtime::ability_dispatch::OwnerKind;
        use std::sync::OnceLock;

        let _home = crate::facade::cli::test_support::HomeGuard::new();
        save(&LocalAgentsFile {
            host_device_agent_ura: "easynet:///r/test-realm/device/dev-1".to_string(),
            hosted_agents: Vec::new(),
        })
        .expect("seed local-agents.json");

        let live_reg = Arc::new(AxonAbilityCatalog::new());
        let manifest = crate::core::ability_spec::AbilityManifest::new(
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
        register(&mut reg, Vec::new, handle, None);

        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();
        let resp = handler(json!({})).unwrap();
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

    #[test]
    fn list_abilities_includes_hot_added_hosted_agent_from_local_agents_ura() {
        use crate::persistence::local_agents::{save, upsert_hosted_agent, LocalAgentsFile};
        use crate::registry::agents::{save_agents, AgentEntry, AgentRegistry, AgentType};
        use std::sync::OnceLock;

        let _home = crate::facade::cli::test_support::HomeGuard::new();
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

        let mut agents = AgentRegistry::default();
        agents.agents.insert(
            "alice".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, Some("sonnet".to_string())),
        );
        agents.agents.insert(
            "bob".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, Some("opus".to_string())),
        );
        save_agents(&agents).expect("seed agents.json");

        let mut reg = AxonAbilityCatalog::new();
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        handle
            .set(Arc::new(AxonAbilityCatalog::new()))
            .expect("set empty live registry");

        register(&mut reg, Vec::new, handle, Some("user-1".to_string()));
        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();
        let resp = handler(json!({})).unwrap();
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
        assert_eq!(chat["metadata"]["host_node_id"], "dev-1");
        assert_eq!(chat["metadata"]["runtime"], "claude-code");
        assert_eq!(chat["metadata"]["model"], "sonnet");
        assert_eq!(
            chat["description"],
            crate::core::ability_spec::default_chat_manifest().description()
        );
        assert!(
            chat["schema_summary"]["input"]["properties"]["prompt"].is_object(),
            "chat descriptor must carry the manifest input schema: {chat}"
        );
    }

    #[test]
    fn list_abilities_keeps_same_public_ability_name_for_multiple_hosted_agents() {
        use crate::persistence::local_agents::{save, upsert_hosted_agent, LocalAgentsFile};
        use crate::registry::agents::{save_agents, AgentEntry, AgentRegistry, AgentType};
        use std::sync::OnceLock;

        let _home = crate::facade::cli::test_support::HomeGuard::new();
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

        let mut agents = AgentRegistry::default();
        agents.agents.insert(
            "anthropic".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, Some("sonnet".to_string())),
        );
        agents.agents.insert(
            "backend-engineer".to_string(),
            AgentEntry::new(AgentType::Codex, Some("gpt-5.4".to_string())),
        );
        save_agents(&agents).expect("seed agents.json");

        let mut reg = AxonAbilityCatalog::new();
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        handle
            .set(Arc::new(AxonAbilityCatalog::new()))
            .expect("set empty live registry");

        register(&mut reg, Vec::new, handle, Some("user-1".to_string()));
        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();
        let resp = handler(json!({})).unwrap();
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
            None,
        );
        let handler = reg.get_rpc(ABILITY_DESCRIBE).unwrap();
        let resp = handler(json!({})).unwrap();
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
        register(&mut reg, Vec::new, empty_registry_handle(), None);
        let handler = reg.get_rpc(ABILITY_DESCRIBE).unwrap();
        let resp = handler(json!({})).unwrap();
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

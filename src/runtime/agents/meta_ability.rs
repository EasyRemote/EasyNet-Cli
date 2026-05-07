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
//   * meta.describe — `{ uri, identity_summary, abilities_summary,
//                         metadata }` for the host device-profile.
//                     identity_summary surfaces the canonical URA
//                     and signing-authority hint; abilities_summary
//                     is the count + namespace breakdown so a caller
//                     can decide whether to follow up with a full
//                     meta.list_abilities.
//   * meta.list_abilities — `{ abilities: AbilityDescriptor[] }`.
//                           The same descriptor catalog mcp.bridge.
//                           list_tools projects to MCP, but in the
//                           native ontology shape (no MCP wrapper).
//                           This is the canonical Invoke surface for
//                           ability discovery; the MCP ability is
//                           the edge-protocol projection of the same
//                           data.
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

use serde_json::{json, Value};

use crate::runtime::ability_descriptor::AbilityDescriptor;
use crate::runtime::ability_dispatch::{LocalAbilityRegistry, OwnerKind};

pub const ABILITY_DESCRIBE: &str = "device.meta.describe";
pub const ABILITY_LIST_ABILITIES: &str = "device.meta.list_abilities";

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
/// `mission.run`, per-agent `<agent>.<verb>` chat-translation
/// handlers, the dynamic per-agent fallback resolver). The static
/// profile descriptor catalogue is merged on top so first-class
/// abilities (fs.read, http.request, ...) keep their full schemas;
/// runtime-only entries (mission.run, hot-reloaded agent abilities)
/// surface with a synthesized descriptor when the static catalogue
/// has nothing for them. Without this two-source merge, the LLM
/// asking `meta.list_abilities` would see a stale, profile-only view
/// that breaks every "discover then invoke" flow.
pub fn register<F>(
    reg: &mut LocalAbilityRegistry,
    descriptors_provider: F,
    registry_handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
    pages_user: Option<String>,
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
            )
        });
    reg.register_rpc_with_owner(ABILITY_LIST_ABILITIES, OwnerKind::Device, list_handler);
}

fn describe_handler(
    descriptors_provider: &Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync>,
) -> anyhow::Result<Value> {
    let descriptors = descriptors_provider();

    // Identity comes from local-agents.json. Pre-join state surfaces
    // as uri:"self" so a caller still sees a well-formed describe
    // response — they can re-poll after the daemon completes join.
    let local = crate::persistence::local_agents::load().unwrap_or_default();
    let host_uri = if local.host_device_agent_uri.is_empty() {
        "self".to_string()
    } else {
        local.host_device_agent_uri.clone()
    };
    let signing_authority = if local.host_device_agent_uri.is_empty() {
        "unprovisioned" // pre-join: no key bound yet
    } else {
        "self" // device-profile is Model A (own keypair)
    };

    // abilities_summary = count + per-namespace count. The breakdown
    // is what makes the response useful to a caller deciding whether
    // to fetch the full catalogue: "12 abilities, 4 in fleet.* and
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
        "uri": host_uri,
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
    registry_handle: &Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
    args: Value,
    pages_user: Option<&str>,
) -> anyhow::Result<Value> {
    use crate::runtime::ability_descriptor::Visibility;

    // Scope parameter (RFC-001 v4.1.7 hub-broadcast contract):
    //   * `"local"` (default) — only abilities the device owns +
    //     hosts. Same payload shape as before this PR.
    //   * `"realm"` — local set merged with the hub-published cache
    //     (`HubPublishedAbilityStore`), so a peer browsing the
    //     realm sees both device-owned and hub-owned abilities
    //     through one call. Hub entries carry their original
    //     descriptor verbatim — the device does not invent
    //     fields.
    let include_realm = args
        .get("scope")
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("realm"))
        .unwrap_or(false);

    // Phase 1: static profile descriptors (fs.*, http.*, shell.*,
    // <agent>.chat, …). These carry full input/output schemas and
    // descriptions read off the workspace ability TOMLs. We index by
    // name so the live-registry merge below can keep them as-is.
    let static_descriptors = descriptors_provider();
    let mut by_name: std::collections::BTreeMap<String, AbilityDescriptor> =
        std::collections::BTreeMap::new();
    for d in static_descriptors {
        // M2 of the system-namespace migration: drop legacy-named
        // descriptors. The static catalogue is built off
        // `published_ability_names()` which now filters to
        // canonical (M2 commit), so this filter normally drops
        // nothing — it's defence-in-depth so a stale call site
        // emitting a legacy descriptor cannot leak into the
        // synth output.
        if !crate::runtime::agents::is_canonical_or_unmapped(&d.name) {
            continue;
        }
        by_name.insert(d.name.clone(), d);
    }

    // Phase 2: live registry. Anything registered into
    // `LocalAbilityRegistry` that the static catalogue does NOT
    // already cover gets a synthesised minimal descriptor. This
    // catches (a) abilities registered AFTER meta_ability itself
    // (mission.run, easynet.* aliases), (b) per-agent verbs that the
    // dynamic fallback resolver wires up at boot from each agent's
    // workspace `abilities/*.toml`, and (c) any future ability whose
    // author forgot to thread it through the profile catalogue.
    if let Some(registry) = registry_handle.get() {
        // Owner URAs for the synthesised descriptors below. These
        // entries are abilities registered into `LocalAbilityRegistry`
        // that no static profile descriptor covers (RFC-002
        // `device.keyring.*`, the runtime `easynet.*` / `mission.*`
        // aliases, dynamic per-agent fallbacks, …). The owner-kind
        // for each ability is read from the registry's
        // `lookup_owner` table (M0 of the system-namespace
        // migration); we resolve that kind to an authoritative URA
        // here using credentials + local-agents.json:
        //
        //   * `OwnerKind::Hub`   → realm hub URA, derived from
        //                          `credentials.tenant_id` via
        //                          `crate::uri::hub_uri`.
        //   * `OwnerKind::Device` → host device URA, read from
        //                          `local-agents.json::host_device_agent_uri`.
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
            .map(|c| c.tenant_id.trim().to_string())
            .filter(|s| !s.is_empty());
        let local = crate::persistence::local_agents::load().unwrap_or_default();
        let device_owner_uri = if local.host_device_agent_uri.is_empty() {
            None
        } else {
            Some(local.host_device_agent_uri.clone())
        };
        let hub_owner_uri = realm.as_deref().map(crate::uri::hub_uri);
        // user-segment used for `OwnerKind::Agent(...)` resolution.
        // Captured at registration time from the same
        // `PagesIdentity` the registry build used, so the synth
        // and registration paths agree on which user-id is
        // canonical without reading process-wide env state on
        // each invocation.
        let user_segment = pages_user.map(str::to_string);
        for name in registry.list_abilities() {
            if by_name.contains_key(&name) {
                continue;
            }
            // M2 of the system-namespace migration: filter the live
            // registry to canonical names only. M1 dual-aliasing
            // registered both legacy (`fs.read`, `01HUB.openai.*`,
            // …) and canonical (`device.fs.read`, `device.openai.*`,
            // …) entries pointing at the same handler; the
            // catalogue surface (this synth, `published_abilities`,
            // `easynet ability list`, advertise prelude) emits
            // canonical only. Inbound dispatch still answers
            // legacy via the registry's binding; M3 deletes the
            // legacy registrations and this filter becomes a
            // no-op.
            if !crate::runtime::agents::is_canonical_or_unmapped(&name) {
                continue;
            }
            // M0 commit 2: read the owner kind from the registry,
            // not by sniffing the name string. Compose the wire URA
            // from the kind. Falling through to None on missing
            // metadata is intentional — synth drops entries it
            // cannot stamp authoritatively.
            let owner_string = match registry.lookup_owner(&name) {
                Some(crate::runtime::ability_dispatch::OwnerKind::Hub) => hub_owner_uri.clone(),
                Some(crate::runtime::ability_dispatch::OwnerKind::Device) => {
                    device_owner_uri.clone()
                }
                Some(crate::runtime::ability_dispatch::OwnerKind::Agent(agent_id)) => {
                    match (realm.as_deref(), user_segment.as_deref()) {
                        (Some(r), Some(u)) => Some(crate::uri::agent_uri(r, u, agent_id)),
                        _ => None,
                    }
                }
                Some(crate::runtime::ability_dispatch::OwnerKind::User(user_id)) => {
                    realm.as_deref().map(|r| crate::uri::user_uri(r, user_id))
                }
                None => {
                    // No owner metadata recorded — the registration
                    // path predates M0 (or the ability landed via
                    // the dynamic fallback resolver, which today
                    // does not stamp owner). Default to the device
                    // URA, matching the legacy synth behaviour.
                    // M0 commit 6 will tighten this to a panic /
                    // hard error once every register site has been
                    // converted.
                    device_owner_uri.clone()
                }
            };
            let Some(owner) = owner_string.as_deref() else {
                continue;
            };
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
            if let Ok(d) = AbilityDescriptor::new(name.clone(), owner, Visibility::Scoped) {
                let descriptor = match registry.manifest_for(&name) {
                    Some(manifest) => {
                        let mut d = d
                            .with_description(manifest.description())
                            .with_input_schema(manifest.input_schema().clone())
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
                        .with_source("registry"),
                };
                by_name.insert(name.clone(), descriptor);
            }
        }
    }

    let mut merged: Vec<Value> = by_name
        .into_values()
        .map(|d| serde_json::to_value(d).unwrap_or(Value::Null))
        .collect();

    // Phase 3: hub-published abilities. Only when the caller asked
    // for realm scope — the default-local path stays byte-identical
    // to pre-v4.1.7. Each entry's `descriptor` is whatever shape
    // the hub published; we surface it verbatim so the
    // hub schema can evolve without forcing a Cli release.
    if include_realm {
        let store = crate::services::hub_published_ability_store::global();
        for entry in store.snapshot() {
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

    Ok(json!({ "abilities": merged }))
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
        AbilityDescriptor::new(name, "easynet:///r/test/agent/01DEV", Visibility::Public)
            .expect("test descriptor")
    }

    /// Empty OnceLock used by tests that don't care about the
    /// live-registry merge — they only exercise the static
    /// descriptor path. The list_abilities handler tolerates an
    /// unset OnceLock (returns the static catalogue alone), so
    /// passing an empty one is the cheapest fixture.
    fn empty_registry_handle() -> Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>> {
        Arc::new(std::sync::OnceLock::new())
    }

    #[test]
    fn registration_makes_both_abilities_dispatchable() {
        let mut reg = LocalAbilityRegistry::new();
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
    fn list_abilities_returns_descriptors_verbatim() {
        let mut reg = LocalAbilityRegistry::new();
        register(
            &mut reg,
            || vec![d("device.observe.health"), d("device.fleet.list_agents")],
            empty_registry_handle(),
            None,
        );
        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();
        let resp = handler(json!({})).unwrap();
        let abilities = resp["abilities"].as_array().unwrap();
        assert_eq!(abilities.len(), 2);
        // Round-trips through serde — full descriptor shape preserved.
        // Post-M3 names are canonical (`device.*` partition).
        let names: Vec<&str> = abilities
            .iter()
            .filter_map(|a| a["name"].as_str())
            .collect();
        assert!(names.contains(&"device.observe.health"));
        assert!(names.contains(&"device.fleet.list_agents"));
    }

    #[test]
    fn list_abilities_realm_scope_includes_hub_published_entries() {
        // RFC-001 v4.1.7 hub-broadcast contract: when the caller
        // passes `scope = "realm"`, the merged catalogue includes
        // entries cached from `federation.{join,heartbeat}`. The
        // default-local path stays disjoint — pin both axes.
        use crate::runtime::federation_client::HubAbilityEntry;
        use crate::services::hub_published_ability_store as store_mod;

        let mut reg = LocalAbilityRegistry::new();
        register(
            &mut reg,
            || vec![d("device.observe.health")],
            empty_registry_handle(),
            None,
        );
        // Seed the process-wide store. Tests in this binary share
        // the singleton; we tolerate residue from earlier tests by
        // looking for `hub.test.scope` specifically rather than
        // asserting an exact count.
        store_mod::global().apply_diff(crate::runtime::federation_client::HubAbilitiesDiff {
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
        assert!(local_names.contains(&"device.observe.health".to_string()));
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
    fn live_registry_synth_surfaces_input_schema_when_manifest_registered() {
        // Pinning the new `register_*_with_spec` contract: when an
        // ability lands in the live registry with an
        // `AbilityManifest`, the synthesised descriptor on
        // `meta.list_abilities` carries the manifest's
        // description + input_schema verbatim. Without this, the
        // Frontend `InvokeAbilityDialog` falls back to "no
        // declared schema" for chat abilities and the user sees a
        // free-text JSON box with no hint about the args shape.
        use crate::runtime::ability_dispatch::OwnerKind;
        use std::sync::OnceLock;

        let mut reg = LocalAbilityRegistry::new();
        let handle: Arc<OnceLock<Arc<LocalAbilityRegistry>>> = Arc::new(OnceLock::new());

        // Live registry entry registered WITH a manifest. We use
        // a freshly-built `LocalAbilityRegistry` here (not the
        // one `register` runs against) and then publish it
        // through the OnceLock seam so the synth path picks it up.
        let mut live_reg = LocalAbilityRegistry::new();
        live_reg.register_rpc_with_spec(
            "alice.chat",
            OwnerKind::Agent("alice".to_string()),
            crate::core::ability_spec::default_chat_manifest(),
            Arc::new(|_args| Ok(json!({}))),
        );
        // A second entry registered the legacy way (no manifest)
        // exercises the fallback arm so we know synth still emits
        // the name-only stub when the manifest is absent.
        live_reg.register_rpc_with_owner(
            "alice.legacy",
            OwnerKind::Agent("alice".to_string()),
            Arc::new(|_args| Ok(json!({}))),
        );
        handle.set(Arc::new(live_reg)).expect("set OnceLock");

        register(&mut reg, Vec::new, handle, Some("alice".to_string()));
        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();
        let resp = handler(json!({})).unwrap();
        let abilities = resp["abilities"].as_array().unwrap();

        let chat = abilities
            .iter()
            .find(|a| a["name"] == "alice.chat")
            .expect("alice.chat must surface from the live registry");
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

        let legacy = abilities
            .iter()
            .find(|a| a["name"] == "alice.legacy")
            .expect("alice.legacy must surface from the live registry");
        // Legacy register path leaves the input schema empty —
        // synth falls back to the name-only stub.
        let legacy_desc = legacy["description"].as_str().unwrap_or_default();
        assert!(
            legacy_desc.contains("no manifest schema"),
            "abilities registered without a manifest keep the fallback description, \
             got: {legacy_desc:?}"
        );
    }

    #[test]
    fn describe_buckets_abilities_by_namespace() {
        let mut reg = LocalAbilityRegistry::new();
        register(
            &mut reg,
            || {
                vec![
                    d("device.observe.health"),
                    d("device.fleet.list_agents"),
                    d("device.fleet.list_sessions"),
                    d("device.consent.subscribe"),
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
        // Post-M3 every system verb is partitioned under `device.*`,
        // so the namespace summary buckets all 4 under "device".
        // The split into fleet / observe / consent is preserved as
        // the SECOND dotted segment but `by_namespace` keys on the
        // first segment by spec.
        assert_eq!(by_ns["device"], 4);
    }

    #[test]
    fn describe_handles_empty_catalog() {
        let mut reg = LocalAbilityRegistry::new();
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

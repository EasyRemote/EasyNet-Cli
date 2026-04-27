// EasyNet CLI — Ability publishing via federation.advertise_*
// =============================================================
//
// File: src/runtime/publish.rs
//
// Per AXON-RFC-001 §A4 + plan v4.1.2 §1, abilities are published to
// the realm directory by invoking the hub-profile Agent's
// `federation.advertise_agent` + `federation.advertise_abilities`
// abilities — NOT by the legacy `register_runtime_local_mcp_tool`
// path that was deleted in P1.2.a.
//
// Pre-RFC history this module replaces
// ------------------------------------
// The pre-RFC publish.rs registered every per-agent manifest +
// every "system ability" against an in-memory MCP catalog held by
// the local axon-runtime. That layer was the single biggest source
// of "frontend Skills page is empty" bugs because the catalog was
// not persistent and the MCP path was load-bearing for Hub-mediated
// discovery. P1.2.a deleted the underlying RPC; the module then
// stubbed every public function to `Ok(false)` until P3+ shipped
// the federation alternative.
//
// What this module does now
// -------------------------
//   * `republish_abilities_via_advertise(invoker, tenant, plan)`
//     bootstraps URAs, persists local-agents.json, advertises
//     every enabled Agent + its descriptors. The single entry
//     point the daemon-boot path and `easynet agent add` both
//     call.
//   * `unpublish_abilities_via_revoke(invoker, tenant, realm,
//     agent_uri)` revokes one Agent's directory entry — used by
//     `easynet agent remove`. Maps to `federation.revoke` per
//     plan §18.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::persistence::local_agents::{self, LocalAgentsFile};
use crate::runtime::advertise::{
    self, AbilityInvoker, AdvertiseOutcome,
};
use crate::runtime::agents::profiles::{
    bootstrap::{self, BootstrapOutcome, BootstrapPlan, UriMinter, UuidMinter},
    self as profiles_mod,
};
use serde_json::Value;

/// Per-call summary returned by `republish_abilities_via_advertise`.
/// Each row is one Agent the daemon advertised — either the device
/// itself (Selfsigned, Model A) or a hosted profile (HostedBy,
/// Model B). The CLI / daemon-boot output layer renders these.
#[derive(Debug, Clone)]
pub struct PublishOutcome {
    /// Canonical URA the advertise call targeted. Empty when
    /// bootstrap returned no rows (operator hasn't enabled any
    /// hosted profiles).
    pub agent_uri: String,
    /// Free-form descriptor of which Agent this row corresponds to,
    /// e.g. `device`, `consent/default`, `llm/claude`. Used for
    /// log lines, not for protocol decisions.
    pub label: String,
    /// `Ok(())` on a clean advertise round trip; `Err(msg)` on any
    /// failure. Per the historical contract, this layer is best-
    /// effort: callers log + continue rather than abort startup.
    pub result: Result<(), String>,
}

/// The single entry point the daemon-boot path and `easynet agent
/// add` use to keep the realm directory in sync with the local
/// install state.
///
/// Steps:
///   1. Run `bootstrap_local_agents` to mint or reuse URAs for
///      every enabled hosted profile.
///   2. Persist the resulting `local-agents.json` (mode 0600).
///   3. Advertise the device-profile Agent itself (Selfsigned).
///   4. Advertise each hosted Agent (HostedBy).
///   5. Advertise the AbilityDescriptors emitted by each profile
///      module's `descriptors_for(...)`.
///
/// Returns a flat Vec<PublishOutcome> the caller renders. The
/// function never panics on a failed advertise — every per-row
/// error becomes one Err entry.
pub fn republish_abilities_via_advertise<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    plan: &BootstrapPlan,
) -> Vec<PublishOutcome> {
    republish_with_minter(invoker, tenant_id, plan, &UuidMinter)
}

/// Same as `republish_abilities_via_advertise` but accepts a
/// custom URI minter. Used by tests with a deterministic minter.
pub fn republish_with_minter<I: AbilityInvoker, M: UriMinter>(
    invoker: &I,
    tenant_id: &str,
    plan: &BootstrapPlan,
    minter: &M,
) -> Vec<PublishOutcome> {
    let mut outcomes = Vec::new();

    // Step 1+2: bootstrap + persist.
    let mut file = match local_agents::load() {
        Ok(f) => f,
        Err(e) => {
            outcomes.push(PublishOutcome {
                agent_uri: String::new(),
                label: "local-agents.json".into(),
                result: Err(format!("read failed; using empty file: {e}")),
            });
            LocalAgentsFile::default()
        }
    };
    let bootstrap_outcomes = bootstrap::bootstrap_local_agents(plan, &mut file, minter);
    if let Err(e) = local_agents::save(&file) {
        outcomes.push(PublishOutcome {
            agent_uri: String::new(),
            label: "local-agents.json".into(),
            result: Err(format!("save failed: {e}")),
        });
        // Continue — in-memory state still allows advertise to run.
    }

    if plan.realm.is_empty() || plan.host_device_uri.is_empty() {
        // Pre-join: nothing to advertise yet (the hub-profile that
        // would receive the call doesn't know us). The bootstrap
        // file has been persisted with `<unjoined>` placeholders;
        // a post-join boot will retry.
        outcomes.push(PublishOutcome {
            agent_uri: String::new(),
            label: "skipped".into(),
            result: Err("daemon not yet joined to a realm; advertise deferred".into()),
        });
        return outcomes;
    }

    // Step 3: advertise the device-profile (Selfsigned, Model A).
    let device_outcome = advertise::advertise_self_signed_device(
        invoker,
        tenant_id,
        &plan.realm,
        &plan.host_device_uri,
        // P5 supplies the actual public_key_hex; P4.8a ships an
        // empty placeholder so the advertise wire shape stays
        // stable. The hub still records the URA + status.
        "",
    );
    outcomes.push(advertise_outcome_to_publish_outcome(
        device_outcome,
        "device".into(),
    ));

    // Lookup tables from bootstrap_outcomes for the descriptor
    // advertise step that follows.
    let consent_uri = first_uri(&bootstrap_outcomes, "consent", "default");
    let policy_uri = first_uri(&bootstrap_outcomes, "policy", "default");
    let mcp_uri = first_uri(&bootstrap_outcomes, "mcp", "default");
    let llm_uris: Vec<(String, String)> = bootstrap_outcomes
        .iter()
        .filter(|o| o.profile == "llm")
        .map(|o| (o.name.clone(), o.agent_uri.clone()))
        .collect();

    // Step 4: advertise each hosted Agent (HostedBy, Model B).
    for o in &bootstrap_outcomes {
        let outcome = advertise::advertise_hosted_agent(
            invoker,
            tenant_id,
            &plan.realm,
            &o.agent_uri,
            &plan.host_device_uri,
        );
        outcomes.push(advertise_outcome_to_publish_outcome(
            outcome,
            format!("{}/{}", o.profile, o.name),
        ));
    }

    // Step 5: advertise descriptors per Agent. We use the
    // profiles aggregator so each Agent's descriptor list is
    // computed once from the live registry.
    let mut descriptors = profiles_mod::all_descriptors_for_host(
        &plan.host_device_uri,
        consent_uri.as_deref(),
        policy_uri.as_deref(),
        mcp_uri.as_deref(),
        &llm_uris,
    );

    // Step 5b: advertise the abilities OWNED by each user-installed
    // agent (e.g. `alice.chat` and any per-agent verbs declared in
    // `<workspace>/abilities/*.ability.toml`). The `llm` profile's
    // descriptors_for() only emits the generic conversation/session/
    // meta/skill prefixes — without this step the realm directory
    // never learns that `alice.chat` exists, so the EasyNet
    // frontend's Abilities catalog cannot list it and the user
    // cannot invoke per-agent abilities through the UI.
    //
    // Read the live registry once, look up each user agent's URA
    // in `llm_uris` (bootstrap minted these earlier), call
    // `abilities_for(name, entry)` to get the per-agent specs, and
    // convert to AbilityDescriptors owned by the user-agent URA.
    // A registry-load failure degrades to "no per-agent advertise
    // this cycle" rather than blocking the rest of publish — the
    // outcome row surfaces the reason.
    match crate::registry::agents::load_agents() {
        Ok(reg) => {
            for (name, entry) in &reg.agents {
                let owner_uri = match llm_uris
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, u)| u.clone())
                {
                    Some(u) => u,
                    None => continue, // bootstrap didn't mint a URA for this agent
                };
                let specs = crate::runtime::abilities::abilities_for(name, entry);
                for spec in specs {
                    let desc = crate::runtime::ability_descriptor::AbilityDescriptor::new(
                        spec.name(),
                        &owner_uri,
                        crate::runtime::ability_descriptor::Visibility::Scoped,
                    );
                    match desc {
                        Ok(d) => {
                            let d = d
                                .with_description(spec.description())
                                .with_input_schema(spec.parameters().clone())
                                .with_source(format!("agent:{name}"));
                            descriptors.push(d);
                        }
                        Err(e) => {
                            outcomes.push(PublishOutcome {
                                agent_uri: owner_uri.clone(),
                                label: format!("agent-ability/{}", spec.name()),
                                result: Err(format!("descriptor build failed: {e}")),
                            });
                        }
                    }
                }
            }
        }
        Err(e) => {
            outcomes.push(PublishOutcome {
                agent_uri: String::new(),
                label: "user-agent-abilities".into(),
                result: Err(format!(
                    "load agent registry failed; per-agent abilities not advertised this cycle: {e}"
                )),
            });
        }
    }

    // Group descriptors by owner Agent and advertise each group.
    let mut by_owner: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
    for d in descriptors {
        by_owner.entry(d.owner_agent_uri.clone()).or_default().push(d);
    }
    for (owner_uri, abilities) in by_owner {
        let result = advertise::advertise_abilities(
            invoker,
            tenant_id,
            &plan.realm,
            &owner_uri,
            &abilities,
        );
        outcomes.push(PublishOutcome {
            agent_uri: owner_uri.clone(),
            label: format!("abilities/{}", abilities.len()),
            result: result.map(|_| ()),
        });
    }

    outcomes
}

/// Revoke one Agent's directory entry. Used by `easynet agent
/// remove` to keep the hub's directory in sync with the local
/// install state.
pub fn unpublish_abilities_via_revoke<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_uri: &str,
    reason: &str,
) -> PublishOutcome {
    let resource_uri = format!(
        "easynet:///r/private/hub/{realm}/abilities/federation.revoke@1"
    );
    let payload = serde_json::json!({
        "agent_uri": agent_uri,
        "reason": reason,
    });
    let result = invoker
        .invoke_ability(tenant_id, &resource_uri, payload)
        .map(|_| ());
    PublishOutcome {
        agent_uri: agent_uri.into(),
        label: "revoke".into(),
        result,
    }
}

fn advertise_outcome_to_publish_outcome(
    outcome: AdvertiseOutcome,
    label: String,
) -> PublishOutcome {
    PublishOutcome {
        agent_uri: outcome.agent_uri,
        label,
        result: outcome.result.map(|_receipt| ()),
    }
}

fn first_uri(
    outcomes: &[BootstrapOutcome],
    profile: &str,
    name: &str,
) -> Option<String> {
    outcomes
        .iter()
        .find(|o| o.profile == profile && o.name == name)
        .map(|o| o.agent_uri.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::cli::test_support::HomeGuard;
    use crate::runtime::agents::profiles::bootstrap::LlmSubAgent;
    use std::cell::RefCell;

    /// Recording fake invoker; mirrors the one in advertise.rs but
    /// counts calls per resource URI so we can assert the expected
    /// federation.* sequence happened.
    struct CountingInvoker {
        calls: RefCell<Vec<(String, Value)>>,
        reply: Value,
    }

    impl CountingInvoker {
        fn new(reply: Value) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                reply,
            }
        }
        fn calls(&self) -> Vec<(String, Value)> {
            self.calls.borrow().clone()
        }
    }

    impl AbilityInvoker for CountingInvoker {
        fn invoke_ability(
            &self,
            _tenant_id: &str,
            resource_uri: &str,
            payload_json: Value,
        ) -> Result<Value, String> {
            self.calls
                .borrow_mut()
                .push((resource_uri.to_string(), payload_json));
            Ok(self.reply.clone())
        }
    }

    struct FailingInvoker;
    impl AbilityInvoker for FailingInvoker {
        fn invoke_ability(
            &self,
            _: &str,
            _: &str,
            _: Value,
        ) -> Result<Value, String> {
            Err("transport down".into())
        }
    }

    /// Same deterministic minter we used in bootstrap tests.
    struct CountingMinter(std::cell::Cell<usize>);
    impl CountingMinter {
        fn new() -> Self {
            Self(std::cell::Cell::new(0))
        }
    }
    impl UriMinter for CountingMinter {
        fn mint_id(&self, profile: &str, name: &str) -> String {
            let n = self.0.get();
            self.0.set(n + 1);
            format!("{profile}-{name}-{n}")
        }
    }

    fn good_reply() -> Value {
        serde_json::json!({"ack": true, "replaced_prior": false})
    }

    fn plan_for(realm: &str, host: &str) -> BootstrapPlan {
        BootstrapPlan {
            realm: realm.into(),
            host_device_uri: host.into(),
            consent: true,
            policy: false,
            mcp: false,
            llm_sub_agents: vec![LlmSubAgent {
                name: "claude".into(),
                agent_type_display: "claude-code".into(),
            }],
        }
    }

    #[test]
    fn republish_emits_device_advertise_then_each_hosted_then_descriptors() {
        let _h = HomeGuard::new();
        let invoker = CountingInvoker::new(good_reply());
        let plan = plan_for("acme", "easynet:///r/acme/agent/01DEV");
        let outcomes = republish_with_minter(&invoker, "tenant", &plan, &CountingMinter::new());

        // We expect: 1 device-advertise + N hosted-advertises + M
        // ability-advertises. With consent + claude enabled, hosted
        // count = 2 (consent/default + llm/claude).
        let calls = invoker.calls();
        let resource_seq: Vec<&str> = calls.iter().map(|(u, _)| u.as_str()).collect();
        let device_count = resource_seq
            .iter()
            .filter(|u| u.contains("federation.advertise_agent@1"))
            .count();
        let abilities_count = resource_seq
            .iter()
            .filter(|u| u.contains("federation.advertise_abilities@1"))
            .count();
        assert_eq!(
            device_count, 3,
            "1 device + 2 hosted = 3 advertise_agent calls; got resource sequence {resource_seq:?}"
        );
        assert!(
            abilities_count >= 1,
            "at least one advertise_abilities call expected; got {resource_seq:?}"
        );
        // No outcome should be Err on a clean reply.
        for o in &outcomes {
            if o.label == "skipped" {
                panic!("post-join plan produced a skipped outcome: {o:?}");
            }
            assert!(o.result.is_ok(), "unexpected Err outcome: {o:?}");
        }
    }

    #[test]
    fn republish_skips_advertise_when_realm_empty() {
        let _h = HomeGuard::new();
        let invoker = CountingInvoker::new(good_reply());
        let mut plan = plan_for("", "");
        plan.consent = true;
        let outcomes =
            republish_with_minter(&invoker, "tenant", &plan, &CountingMinter::new());
        // Pre-join: bootstrap still ran but advertise was skipped.
        // We should see ZERO calls to the bridge.
        assert!(invoker.calls().is_empty(), "no advertise calls should have happened");
        // The single outcome must report the skip.
        let skipped = outcomes
            .iter()
            .find(|o| o.label == "skipped")
            .expect("expected a 'skipped' outcome");
        assert!(skipped.result.is_err());
    }

    #[test]
    fn republish_surfaces_per_call_failure_without_aborting() {
        let _h = HomeGuard::new();
        let plan = plan_for("acme", "easynet:///r/acme/agent/01DEV");
        let outcomes =
            republish_with_minter(&FailingInvoker, "tenant", &plan, &CountingMinter::new());
        // Every advertise call must turn into one Err PublishOutcome.
        assert!(
            outcomes.iter().all(|o| {
                o.label == "skipped" || o.result.is_err() || o.label == "local-agents.json"
            }),
            "every advertise must surface its error; got {outcomes:?}"
        );
        let failed = outcomes
            .iter()
            .filter(|o| o.result.is_err() && o.label != "local-agents.json")
            .count();
        assert!(failed > 0, "at least one advertise failure expected");
    }

    #[test]
    fn republish_advertises_user_agent_chat_ability_under_user_agent_owner() {
        // Reproduces the gap caught by an end-to-end audit: when a
        // user installs a claude-code agent named `alice`, the daemon
        // must advertise `alice.chat` (and any other per-agent
        // verbs from <workspace>/abilities/*.ability.toml) so the
        // EasyNet frontend's Abilities catalog can list it AND the
        // backend can route invokes back to alice. Pre-fix the LLM
        // profile only published the generic conversation/session/
        // meta/skill prefixes, so `alice.chat` never reached the
        // realm directory and the UI could not see it.
        let _h = HomeGuard::new();

        // Persist an `alice` AgentEntry into the registry so that
        // `load_agents()` inside republish_with_minter sees it.
        let mut reg = crate::registry::agents::AgentRegistry::default();
        reg.agents.insert(
            "alice".to_string(),
            crate::registry::agents::AgentEntry::new(
                crate::registry::agents::AgentType::ClaudeCode,
                None,
            ),
        );
        crate::registry::agents::save_agents(&reg).expect("save alice into registry");

        // Plan: realm joined, alice listed as an LLM sub-agent so
        // bootstrap mints a URA for her.
        let plan = BootstrapPlan {
            realm: "acme".into(),
            host_device_uri: "easynet:///r/acme/agent/01DEV".into(),
            consent: false,
            policy: false,
            mcp: false,
            llm_sub_agents: vec![LlmSubAgent {
                name: "alice".into(),
                agent_type_display: "claude-code".into(),
            }],
        };
        let invoker = CountingInvoker::new(good_reply());
        let outcomes = republish_with_minter(&invoker, "tenant", &plan, &CountingMinter::new());

        // Locate the `alice` URA via the persisted local-agents file.
        let file_back = local_agents::load().expect("load local-agents.json");
        let alice_uri = &file_back
            .hosted_agents
            .iter()
            .find(|e| e.profile == "llm" && e.name == "alice")
            .expect("bootstrap must have minted a URA for alice")
            .agent_uri;

        // Find the advertise_abilities call whose payload's
        // `agent_uri` is alice's URA, and assert `alice.chat`
        // appears in its abilities list. The daemon may pack
        // multiple abilities per call; we scan, not require the
        // first match.
        let calls = invoker.calls();
        let alice_advert = calls
            .iter()
            .find(|(u, p)| {
                u.contains("federation.advertise_abilities@1")
                    && p["agent_uri"].as_str() == Some(alice_uri.as_str())
            })
            .unwrap_or_else(|| {
                panic!(
                    "expected an advertise_abilities call owned by {alice_uri:?}; \
                     resource_seq = {:?}",
                    calls.iter().map(|(u, _)| u).collect::<Vec<_>>()
                )
            });
        let abilities = alice_advert.1["abilities"]
            .as_array()
            .expect("abilities array on advertise_abilities payload");
        let names: Vec<&str> = abilities
            .iter()
            .filter_map(|a| a["name"].as_str())
            .collect();
        assert!(
            names.iter().any(|n| *n == "alice.chat"),
            "alice.chat must appear in advertised abilities for {alice_uri:?}; got names = {names:?}"
        );

        // Sanity: outcomes carry one row per advertise_abilities
        // group; alice's row must be Ok.
        let alice_row = outcomes
            .iter()
            .find(|o| o.agent_uri == *alice_uri && o.label.starts_with("abilities/"))
            .expect("alice's abilities-advertise outcome row must exist");
        assert!(alice_row.result.is_ok(), "alice abilities advertise: {alice_row:?}");
    }

    #[test]
    fn republish_does_not_lose_device_descriptors_when_user_agent_added() {
        // Regression guard: stitching per-agent descriptors into the
        // existing list must not displace device-level ones. If a
        // refactor accidentally replaces (rather than appends) the
        // descriptors Vec, the device-profile abilities (fs.read,
        // shell.run, …) would silently drop off the wire.
        let _h = HomeGuard::new();
        let mut reg = crate::registry::agents::AgentRegistry::default();
        reg.agents.insert(
            "alice".into(),
            crate::registry::agents::AgentEntry::new(
                crate::registry::agents::AgentType::ClaudeCode,
                None,
            ),
        );
        crate::registry::agents::save_agents(&reg).unwrap();
        let plan = BootstrapPlan {
            realm: "acme".into(),
            host_device_uri: "easynet:///r/acme/agent/01DEV".into(),
            consent: false,
            policy: false,
            mcp: false,
            llm_sub_agents: vec![LlmSubAgent {
                name: "alice".into(),
                agent_type_display: "claude-code".into(),
            }],
        };
        let invoker = CountingInvoker::new(good_reply());
        let _ = republish_with_minter(&invoker, "tenant", &plan, &CountingMinter::new());

        // The device-owner advertise must still carry at least one
        // device-level ability (fs.read is the canary — it has been
        // in the device profile since Tier 2.5 baseline locomotion).
        let calls = invoker.calls();
        let device_advert = calls
            .iter()
            .find(|(u, p)| {
                u.contains("federation.advertise_abilities@1")
                    && p["agent_uri"].as_str() == Some("easynet:///r/acme/agent/01DEV")
            })
            .expect("device-owner advertise_abilities call must still exist");
        let abilities = device_advert.1["abilities"]
            .as_array()
            .expect("abilities array on device advertise");
        let names: Vec<&str> = abilities
            .iter()
            .filter_map(|a| a["name"].as_str())
            .collect();
        assert!(
            names.iter().any(|n| *n == "fs.read"),
            "device descriptors must survive per-agent stitch; got names = {names:?}"
        );
    }

    #[test]
    fn unpublish_targets_federation_revoke_resource_uri() {
        let invoker = CountingInvoker::new(good_reply());
        let outcome = unpublish_abilities_via_revoke(
            &invoker,
            "tenant",
            "acme",
            "easynet:///r/acme/agent/01OLD",
            "operator removed",
        );
        assert!(outcome.result.is_ok());
        let calls = invoker.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].0.contains("federation.revoke@1"));
        assert_eq!(calls[0].1["agent_uri"], "easynet:///r/acme/agent/01OLD");
        assert_eq!(calls[0].1["reason"], "operator removed");
    }

    #[test]
    fn republish_persists_local_agents_file_with_minted_uris() {
        let _h = HomeGuard::new();
        let invoker = CountingInvoker::new(good_reply());
        let plan = plan_for("acme", "easynet:///r/acme/agent/01DEV");
        let _ = republish_with_minter(&invoker, "tenant", &plan, &CountingMinter::new());
        let calls = invoker.calls();
        // Find the consent-hosted advertise and assert the URA shape.
        let consent_call = calls
            .iter()
            .find(|(u, p)| {
                u.contains("federation.advertise_agent")
                    && p["signing_authority"]["kind"] == "hosted_by"
                    && p["agent_uri"].as_str().unwrap().contains("consent-default")
            });
        assert!(
            consent_call.is_some(),
            "expected a hosted_by advertise for consent/default, got calls = {calls:#?}"
        );

        // Persistence end-to-end: read local-agents.json back from
        // the isolated $HOME and confirm the consent + llm rows
        // landed with stable URAs.
        let file_back = local_agents::load().expect("load after save must succeed");
        assert_eq!(file_back.host_device_agent_uri, "easynet:///r/acme/agent/01DEV");
        let consent_row = file_back
            .hosted_agents
            .iter()
            .find(|e| e.profile == "consent" && e.name == "default")
            .expect("consent/default row must be persisted");
        assert!(consent_row.agent_uri.contains("consent-default"));
        let llm_row = file_back
            .hosted_agents
            .iter()
            .find(|e| e.profile == "llm" && e.name == "claude")
            .expect("llm/claude row must be persisted");
        assert!(llm_row.agent_uri.contains("llm-claude"));
    }

    #[test]
    fn second_republish_reuses_persisted_uris_no_duplicate_advertise() {
        let _h = HomeGuard::new();
        let plan = plan_for("acme", "easynet:///r/acme/agent/01DEV");
        let invoker_a = CountingInvoker::new(good_reply());
        let _ = republish_with_minter(&invoker_a, "tenant", &plan, &CountingMinter::new());
        let first_calls = invoker_a.calls();
        let consent_uri_v1 = first_calls
            .iter()
            .find_map(|(u, p)| {
                if u.contains("federation.advertise_agent")
                    && p["signing_authority"]["kind"] == "hosted_by"
                    && p["agent_uri"].as_str().unwrap().contains("consent-default")
                {
                    p["agent_uri"].as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .expect("first run must have advertised consent/default");

        // Second run with a fresh minter — if the persistence path
        // works, we must NOT mint a new URI; the second advertise
        // must carry the same URA as the first.
        let invoker_b = CountingInvoker::new(good_reply());
        let _ = republish_with_minter(&invoker_b, "tenant", &plan, &CountingMinter::new());
        let consent_uri_v2 = invoker_b
            .calls()
            .iter()
            .find_map(|(u, p)| {
                if u.contains("federation.advertise_agent")
                    && p["signing_authority"]["kind"] == "hosted_by"
                    && p["agent_uri"].as_str().unwrap().contains("consent-default")
                {
                    p["agent_uri"].as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .expect("second run must still advertise consent/default");

        assert_eq!(
            consent_uri_v1, consent_uri_v2,
            "second republish must reuse the persisted URA for consent/default"
        );
    }
}

// EasyNet CLI — Hosted Agent bootstrap (RFC-001 §1.4 / §A13)
// =============================================================
//
// File: src/runtime/agents/profiles/bootstrap.rs
//
// Mints + persists canonical URAs for every Agent this daemon
// hosts. Per RFC §1.4 [P2] every hosted Agent URI MUST be:
//
//   1. Minted by the hosting device-profile on first boot.
//   2. Persisted to ~/.easynet/local-agents.json.
//   3. Reused across daemon restarts (no churn — every restart
//      that re-mints makes the hub's directory accumulate dead
//      entries that only TTL-expire).
//
// What this module does
// ---------------------
// `bootstrap_local_agents` walks a `BootstrapPlan` (which says
// which profiles are enabled and what realm we're in), reads the
// existing local-agents.json, mints fresh URAs for any
// (profile, name) pair not already present, and writes the file
// back. Returns the resulting in-memory `LocalAgentsFile` so the
// caller has the URA → profile mapping it needs to advertise.
//
// What this module does NOT do
// ----------------------------
// - Does not call `federation.advertise_agent`. That's the
//   advertise.rs module's job, called by the daemon-boot wiring
//   that consumes this module's output.
// - Does not register handlers in LocalAbilityRegistry. That's
//   already wired in `runtime::agents::build_registry_for_daemon`.
// - Does not assume credentials.json exists. The host_device URI
//   may be "" if join hasn't run; the persisted file just records
//   the hosted URIs alongside, with `signing_authority`
//   placeholder until the next save after join.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::persistence::local_agents::{upsert_hosted_agent, HostedAgentEntry, LocalAgentsFile};

/// Configuration for one bootstrap pass. The daemon-boot wiring
/// fills this from `~/.easynet/config.toml` (`[profiles]` section)
/// and from the loaded `registry::AgentRegistry`.
#[derive(Debug, Clone, Default)]
pub struct BootstrapPlan {
    /// Realm name from credentials.json. Used to mint canonical
    /// URAs of shape `easynet:///r/<realm>/agent/<user>.<id>`.
    /// Empty when the daemon hasn't joined yet — we still pre-mint
    /// URAs into the file but flag them in the resulting save.
    pub realm: String,
    /// User UUID from credentials (`username` field, which carries
    /// the user-uuid in v4.1.4). All hosted agents this daemon
    /// owns are anchored under this user. Empty pre-join, in which
    /// case the URI is flagged with the literal `<unjoined>` user
    /// so the first post-join save can repair it.
    pub user_id: String,
    /// Device-profile URA from credentials.json. Empty pre-join.
    /// When non-empty, `local-agents.json::host_device_agent_uri`
    /// is set to this on save.
    pub host_device_uri: String,
    /// Whether each hosted profile should have a URA minted +
    /// advertised. Mirrors `[profiles]` config booleans.
    pub consent: bool,
    pub policy: bool,
    pub mcp: bool,
    /// One entry per LLM sub-agent the daemon supervises.
    /// `(name, agent_type_display)` — `agent_type_display` is
    /// the legacy AgentType's string form, surfaced into the
    /// LLM descriptor's metadata per P4.4.
    pub llm_sub_agents: Vec<LlmSubAgent>,
}

#[derive(Debug, Clone)]
pub struct LlmSubAgent {
    pub name: String,
    pub agent_type_display: String,
}

/// One outcome row from `bootstrap_local_agents`. Useful for
/// the boot-time advertise step (P4.7's downstream consumer)
/// to know which URAs to advertise as new vs reused.
#[derive(Debug, Clone, PartialEq)]
pub struct BootstrapOutcome {
    pub profile: String,
    pub name: String,
    pub agent_uri: String,
    /// `true` when this row was already in `local-agents.json`
    /// from a previous boot; `false` when freshly minted.
    pub reused: bool,
}

/// URI minter trait. Production callers pass `UlidMinter`; tests
/// pass deterministic minters that emit predictable strings so
/// assertions don't depend on real ULID generation.
pub trait UriMinter {
    /// Produce a fresh canonical URA suffix (the part after
    /// `easynet:///r/<realm>/agent/`). Must be unique across all
    /// past calls in this process.
    fn mint_id(&self, profile: &str, name: &str) -> String;
}

/// Production URI minter — uses `common::new_id` from the
/// internal helper that the rest of the codebase uses for run/
/// invocation IDs.
pub struct UuidMinter;

impl UriMinter for UuidMinter {
    fn mint_id(&self, _profile: &str, _name: &str) -> String {
        // Match axon's hub-profile minting style ("a-<uuid>").
        // Keeping the prefix short (`a` for "agent") preserves
        // grep-ability without cluttering the URA.
        format!("a-{}", uuid::Uuid::new_v4().simple())
    }
}

/// Walk the plan, mint URAs for any missing entries, persist,
/// and return the per-row outcomes. Pure-function except for
/// the persistence layer — accepts a `UriMinter` so tests can
/// drive deterministic IDs.
pub fn bootstrap_local_agents<M: UriMinter>(
    plan: &BootstrapPlan,
    file: &mut LocalAgentsFile,
    minter: &M,
) -> Vec<BootstrapOutcome> {
    // Update host URI before any minting so upserts record the
    // correct `signing_authority`.
    if !plan.host_device_uri.is_empty() {
        file.host_device_agent_uri = plan.host_device_uri.clone();
    }
    let mut outcomes = Vec::new();
    let mut process = |profile: &str, name: &str, outcomes: &mut Vec<BootstrapOutcome>| {
        let existing = file
            .hosted_agents
            .iter()
            .find(|e| e.profile == profile && e.name == name)
            .map(|e| e.agent_uri.clone());
        let (uri, reused) = match existing {
            Some(existing_uri)
                if uri_contains_unjoined(&existing_uri)
                    && !plan.realm.is_empty()
                    && !plan.user_id.is_empty() =>
            {
                // Repair path: this row was minted before the device
                // joined a realm. `<unjoined>` placeholders cause the
                // hub-side `federation.resolve` visibility gate to
                // skip the agent (the gate keys off a real user URA),
                // so the agent shows up in `agent list` locally but
                // never appears in backend `/api/v1/agents`. Now that
                // we know the real realm + user_id post-join, keep
                // the original agent_id (operators may have
                // referenced it elsewhere) and rebuild a canonical
                // URI around it. Persist so the repair is idempotent.
                let agent_id = extract_agent_id_tail(&existing_uri)
                    .unwrap_or_else(|| minter.mint_id(profile, name));
                let repaired = crate::uri::agent_uri(&plan.realm, &plan.user_id, &agent_id);
                upsert_hosted_agent(file, profile, name, &repaired);
                (repaired, false)
            }
            Some(uri) => (uri, true),
            None => {
                let id = minter.mint_id(profile, name);
                // URI v4.1.4: agent URI is user-anchored
                // (`<user>.<agent-id>`). Pre-join state lacks both
                // realm and user_id; flag with literal `<unjoined>`
                // for both. The repair branch above corrects it on
                // the next bootstrap pass after join.
                let realm = if plan.realm.is_empty() {
                    "<unjoined>"
                } else {
                    plan.realm.as_str()
                };
                let user_id = if plan.user_id.is_empty() {
                    "<unjoined>"
                } else {
                    plan.user_id.as_str()
                };
                let uri = crate::uri::agent_uri(realm, user_id, &id);
                upsert_hosted_agent(file, profile, name, &uri);
                (uri, false)
            }
        };
        outcomes.push(BootstrapOutcome {
            profile: profile.to_string(),
            name: name.to_string(),
            agent_uri: uri,
            reused,
        });
    };
    if plan.consent {
        process("consent", "default", &mut outcomes);
    }
    if plan.policy {
        process("policy", "default", &mut outcomes);
    }
    if plan.mcp {
        process("mcp", "default", &mut outcomes);
    }
    for sub in &plan.llm_sub_agents {
        process("llm", &sub.name, &mut outcomes);
    }
    outcomes
}

/// Lookup the agent_type_display for an LLM sub-agent name from
/// the bootstrap plan. Used by the advertise step to stamp
/// `descriptor.metadata["agent_type"]` per P4.4.
pub fn agent_type_for(plan: &BootstrapPlan, sub_name: &str) -> Option<String> {
    plan.llm_sub_agents
        .iter()
        .find(|s| s.name == sub_name)
        .map(|s| s.agent_type_display.clone())
}

/// Lookup convenience: filter a bootstrap result to one profile.
pub fn outcomes_for_profile<'a>(
    outcomes: &'a [BootstrapOutcome],
    profile: &str,
) -> Vec<&'a BootstrapOutcome> {
    outcomes.iter().filter(|o| o.profile == profile).collect()
}

/// Convenience accessor: every hosted-agent row currently in the
/// file (after a bootstrap pass), as `(profile, name, uri)`.
pub fn hosted_uris(file: &LocalAgentsFile) -> Vec<(String, String, String)> {
    file.hosted_agents
        .iter()
        .map(|e: &HostedAgentEntry| (e.profile.clone(), e.name.clone(), e.agent_uri.clone()))
        .collect()
}

/// True iff the URI is one of the pre-join placeholders
/// (`easynet:///r/<unjoined>/...` or `.../agent/<unjoined>.<id>`).
/// Used by the bootstrap-time repair branch to decide whether a row
/// in `agents.json` should be re-minted with the now-known realm +
/// user_id. The check is intentionally substring-based — both the
/// realm slot and the user_id slot can carry the placeholder, and
/// either one is enough to fail backend-side visibility filtering.
fn uri_contains_unjoined(uri: &str) -> bool {
    uri.contains("<unjoined>")
}

/// Pull the agent_id tail from a `.../agent/<user>.<agent_id>` URI.
/// Returns None when the URI doesn't parse as a v4.1.5 agent URA so
/// the caller can fall back to a fresh mint instead of embedding the
/// malformed remainder.
///
/// Strict-parser rule (silan's `feedback_no_legacy_ura`): every URI
/// parse goes through `crate::uri::parse_ura`. Hand-rolled
/// `split("/agent/")` would silently accept legacy `r/<scope>/reg/agent.<id>`
/// URNs and other non-v4.1.5 shapes, defeating the canonical-shape
/// invariant the rest of the codebase has been clamped to.
fn extract_agent_id_tail(uri: &str) -> Option<String> {
    let parsed = crate::uri::parse_ura(uri).ok()?;
    if parsed.kind != crate::uri::URAKind::Agent {
        return None;
    }
    if parsed.agent_id.is_empty() {
        return None;
    }
    Some(parsed.agent_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic minter for tests. Emits "<profile>-<name>-<seq>"
    /// so failure messages name exactly which row went wrong.
    struct CountingMinter {
        seq: std::cell::Cell<usize>,
    }

    impl CountingMinter {
        fn new() -> Self {
            Self {
                seq: std::cell::Cell::new(0),
            }
        }
    }

    impl UriMinter for CountingMinter {
        fn mint_id(&self, profile: &str, name: &str) -> String {
            let n = self.seq.get();
            self.seq.set(n + 1);
            format!("{profile}-{name}-{n}")
        }
    }

    fn plan_with(consent: bool, policy: bool, mcp: bool, llms: &[(&str, &str)]) -> BootstrapPlan {
        BootstrapPlan {
            realm: "acme".into(),
            user_id: "u1".into(),
            host_device_uri: "easynet:///r/acme/device/01DEV".into(),
            consent,
            policy,
            mcp,
            llm_sub_agents: llms
                .iter()
                .map(|(n, t)| LlmSubAgent {
                    name: (*n).into(),
                    agent_type_display: (*t).into(),
                })
                .collect(),
        }
    }

    #[test]
    fn bootstrap_mints_uri_for_each_enabled_profile() {
        let plan = plan_with(true, true, true, &[("claude", "claude-code")]);
        let mut file = LocalAgentsFile::default();
        let minter = CountingMinter::new();
        let outcomes = bootstrap_local_agents(&plan, &mut file, &minter);

        assert_eq!(outcomes.len(), 4); // consent + policy + mcp + 1 llm
        for o in &outcomes {
            assert!(
                !o.reused,
                "first bootstrap pass must mint fresh URIs, got reused for {:?}",
                o.profile
            );
            // v4.1.4: hosted-agent URA is user-anchored
            // (`r/<realm>/agent/<user>.<id>`). plan_with seeds
            // user_id="u1".
            assert!(
                o.agent_uri.starts_with("easynet:///r/acme/agent/u1."),
                "expected user-anchored agent URA, got {:?}",
                o.agent_uri
            );
        }
        // The daemon's own self-URI uses the device segment.
        assert_eq!(file.host_device_agent_uri, "easynet:///r/acme/device/01DEV");
        assert_eq!(file.hosted_agents.len(), 4);
    }

    #[test]
    fn second_bootstrap_reuses_existing_uris() {
        let plan = plan_with(true, false, false, &[]);
        let mut file = LocalAgentsFile::default();
        let minter = CountingMinter::new();
        let first = bootstrap_local_agents(&plan, &mut file, &minter);
        let consent_uri_v1 = first[0].agent_uri.clone();

        // Same plan, same file — every outcome must report reused=true
        // and the URI must match exactly.
        let second = bootstrap_local_agents(&plan, &mut file, &minter);
        assert_eq!(second.len(), 1);
        assert!(second[0].reused);
        assert_eq!(second[0].agent_uri, consent_uri_v1);
        assert_eq!(file.hosted_agents.len(), 1, "must not duplicate rows");
    }

    #[test]
    fn disabling_a_profile_leaves_existing_uri_intact_in_file() {
        // Mint consent + policy on first boot. On second boot the
        // operator turned policy=false; we expect the policy row to
        // remain in the file (we don't garbage-collect on disable —
        // operators may toggle profiles temporarily, and we don't
        // want to mint fresh URIs on every flip).
        let mut file = LocalAgentsFile::default();
        let minter = CountingMinter::new();
        let _ = bootstrap_local_agents(&plan_with(true, true, false, &[]), &mut file, &minter);
        assert_eq!(file.hosted_agents.len(), 2);
        let _ = bootstrap_local_agents(&plan_with(true, false, false, &[]), &mut file, &minter);
        // policy row still present
        assert!(file.hosted_agents.iter().any(|e| e.profile == "policy"));
    }

    #[test]
    fn pre_join_realm_emits_unjoined_placeholder() {
        // Realm empty until join completes — bootstrap can still
        // run (so the daemon has stable URIs the moment it joins),
        // but the URI carries `<unjoined>` so an operator inspecting
        // the file sees what's wrong.
        let mut plan = plan_with(true, false, false, &[]);
        plan.realm = String::new();
        plan.host_device_uri = String::new();
        let mut file = LocalAgentsFile::default();
        let outcomes = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert!(outcomes[0].agent_uri.contains("<unjoined>"));
        assert!(file.host_device_agent_uri.is_empty());
    }

    #[test]
    fn agent_type_for_returns_display_string() {
        let plan = plan_with(false, false, false, &[("claude", "claude-code")]);
        assert_eq!(
            agent_type_for(&plan, "claude").as_deref(),
            Some("claude-code")
        );
        assert_eq!(agent_type_for(&plan, "missing"), None);
    }

    #[test]
    fn outcomes_for_profile_filters_correctly() {
        let plan = plan_with(true, true, false, &[("claude", "claude-code")]);
        let mut file = LocalAgentsFile::default();
        let outcomes = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert_eq!(outcomes_for_profile(&outcomes, "consent").len(), 1);
        assert_eq!(outcomes_for_profile(&outcomes, "llm").len(), 1);
        assert_eq!(outcomes_for_profile(&outcomes, "mcp").len(), 0);
    }

    #[test]
    fn hosted_uris_returns_every_row_after_bootstrap() {
        let mut file = LocalAgentsFile::default();
        let _ = bootstrap_local_agents(
            &plan_with(true, false, true, &[]),
            &mut file,
            &CountingMinter::new(),
        );
        let rows = hosted_uris(&file);
        let profiles: std::collections::HashSet<&str> =
            rows.iter().map(|(p, _, _)| p.as_str()).collect();
        assert!(profiles.contains("consent"));
        assert!(profiles.contains("mcp"));
    }

    #[test]
    fn host_device_uri_persists_after_post_join_save() {
        // Pre-join: file has hosted rows but no host URI.
        let mut file = LocalAgentsFile::default();
        let mut plan = plan_with(true, false, false, &[]);
        plan.realm = String::new();
        plan.host_device_uri = String::new();
        let _ = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert!(file.host_device_agent_uri.is_empty());

        // Post-join: realm + host URI now known. Re-run bootstrap.
        plan.realm = "acme".into();
        // URI v4.1.4: host device URA uses the `/device/` role
        // segment (Phase 2F) — the legacy `/agent/01DEV` collapsed
        // every profile under one role.
        plan.host_device_uri = "easynet:///r/acme/device/01DEV".into();
        let _ = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert_eq!(file.host_device_agent_uri, "easynet:///r/acme/device/01DEV");
    }

    // ── extract_agent_id_tail: strict v4.1.5 parser only ─────────

    #[test]
    fn extract_agent_id_tail_returns_id_for_canonical_agent_uri() {
        let uri = "easynet:///r/acme/agent/alice.claude";
        assert_eq!(extract_agent_id_tail(uri), Some("claude".into()));
    }

    #[test]
    fn extract_agent_id_tail_returns_id_for_pre_join_placeholder_uri() {
        // The repair branch reads pre-join URIs out of agents.json
        // when the device joins a realm. parse_ura admits the
        // `<unjoined>` placeholder because it only rejects `.`/`/`/
        // empty in the user-id slot, not arbitrary opaque tokens.
        let uri = "easynet:///r/<unjoined>/agent/<unjoined>.consent-default-0";
        assert_eq!(extract_agent_id_tail(uri), Some("consent-default-0".into()));
    }

    #[test]
    fn extract_agent_id_tail_rejects_legacy_reg_form() {
        // Legacy `r/<scope>/reg/agent.<id>` URN — the v1 RFC-001
        // shape that pre-dates the six-role v4.1.5 ontology. Strict
        // parser refuses; old hand-rolled `split("/agent/")` would
        // have silently returned a malformed tail.
        let uri = "easynet:///r/acme/reg/agent.claude";
        assert!(extract_agent_id_tail(uri).is_none());
    }

    #[test]
    fn extract_agent_id_tail_rejects_device_uri() {
        // device URA has no agent_id slot; reject so the caller
        // mints a fresh one rather than embedding a malformed tail.
        let uri = "easynet:///r/acme/device/01DEV";
        assert!(extract_agent_id_tail(uri).is_none());
    }

    #[test]
    fn extract_agent_id_tail_rejects_garbage() {
        assert!(extract_agent_id_tail("").is_none());
        assert!(extract_agent_id_tail("not-a-uri").is_none());
        assert!(extract_agent_id_tail("easynet:///r/acme").is_none());
    }
}

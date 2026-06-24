// EasyNet CLI — Hosted Agent bootstrap (RFC-001 §1.4 / §A13)
// =============================================================
//
// File: src/runtime/agents/profiles/bootstrap.rs
//
// Mints + persists canonical URAs for every Agent this daemon
// hosts. Per RFC §1.4 [P2] every hosted Agent URA MUST be:
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
// existing local-agents.json, keeps only canonical hosted-agent
// URAs that belong to the current realm/user, mints fresh URAs for
// any enabled (profile, name) pair not already present, and writes
// the file back. Returns the resulting in-memory `LocalAgentsFile`
// so the caller has the URA → profile mapping it needs to advertise.
//
// What this module does NOT do
// ----------------------------
// - Does not call `federation.advertise_agent`. That's the
//   advertise.rs module's job, called by the daemon-boot wiring
//   that consumes this module's output.
// - Does not register handlers in AxonAbilityCatalog. That's
//   already wired in `runtime::agents::build_registry_for_daemon`.
// - Does not assume credentials.json exists. Before join, there is
//   no canonical realm/user identity, so hosted-agent minting is
//   skipped rather than writing placeholder URAs.
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
    /// Empty when the daemon hasn't joined yet; hosted-agent URA
    /// minting is skipped until a canonical realm exists.
    pub realm: String,
    /// User UUID from credentials (`username` field, which carries
    /// the user-uuid in v4.1.4). All hosted agents this daemon
    /// owns are anchored under this user. Empty pre-join, in which
    /// case hosted-agent URA minting is skipped until a canonical
    /// user identity exists.
    pub user_id: String,
    /// Device-profile URA from credentials.json. Empty pre-join.
    /// When non-empty, `local-agents.json::host_device_agent_ura`
    /// is set to this on save.
    pub host_device_ura: String,
    /// Whether each hosted profile should have a URA minted +
    /// advertised. Mirrors `[profiles]` config booleans.
    pub consent: bool,
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
    pub model: Option<String>,
}

/// Build the daemon bootstrap plan from credentials-derived identity
/// plus the persisted hosted-agent registry.
///
/// This is bootstrapping state, so it intentionally lives in runtime
/// rather than CLI facade code. Normal post-boot management flows go
/// through daemon-hosted Axon abilities.
pub fn build_plan_from_registry(
    tenant_id: &str,
    node_id: &str,
    username: &str,
) -> anyhow::Result<BootstrapPlan> {
    let registry = crate::registry::agents::load_agents()
        .map_err(|err| anyhow::anyhow!("load agent registry: {err}"))?;
    let llm_sub_agents: Vec<LlmSubAgent> = registry
        .agents
        .iter()
        .map(|(name, entry)| LlmSubAgent {
            name: name.clone(),
            agent_type_display: entry.agent_type.to_string(),
            model: entry.model.clone(),
        })
        .collect();

    Ok(BootstrapPlan {
        realm: tenant_id.to_string(),
        user_id: username.to_string(),
        host_device_ura: crate::ura::device_ura(tenant_id, node_id),
        consent: true,
        mcp: false,
        llm_sub_agents,
    })
}

/// One outcome row from `bootstrap_local_agents`. Useful for
/// the boot-time advertise step (P4.7's downstream consumer)
/// to know which URAs to advertise as new vs reused.
#[derive(Debug, Clone, PartialEq)]
pub struct BootstrapOutcome {
    pub profile: String,
    pub name: String,
    pub agent_ura: String,
    /// `true` when this row was already in `local-agents.json`
    /// from a previous boot; `false` when freshly minted.
    pub reused: bool,
}

/// URA minter trait. Production callers pass `UlidMinter`; tests
/// pass deterministic minters that emit predictable strings so
/// assertions don't depend on real ULID generation.
pub trait UraMinter {
    /// Produce a fresh canonical URA suffix (the part after
    /// `easynet:///r/<realm>/agent/`). Must be unique across all
    /// past calls in this process.
    fn mint_id(&self, profile: &str, name: &str) -> String;
}

/// Production URA minter — encodes the operator-meaningful name
/// into the agent URA's agent-id slug so the friendly identity
/// lives **inside** the canonical URA, not as out-of-band metadata.
///
/// Two minting rules, switched on profile class:
///
///   * **LLM profile** (`profile == "llm"`): the operator already
///     gave a meaningful name through `easynet agent add <name>`
///     (`probe-agent`, `codex`, `web-builder`, …). The minted
///     agent-id is the raw `name` — no `llm-` prefix. This keeps
///     the dispatch registry's `<agent>.chat` entries (registered
///     by `runtime::agents::mod::build_registry_with_services`
///     under `agents.json::keys`, i.e. the raw name) aligned with
///     what `local-agents.json` advertises to the hub. Without
///     this alignment, `/v1/chat/completions` resolves
///     `model: "probe-agent"` to `agent/<user>.probe-agent` while
///     the hub PresenceRegistry only knows
///     `agent/<user>.llm-probe-agent`, and routing fails with
///     "not in PresenceRegistry; either offline or never connected
///     to this hub" (RFC-006-C v0.1 §INV-2).
///
///   * **System-managed profiles** (`consent`, `mcp`):
///     the name is auto-generated and generic (`default`,
///     `fs-bridge`). The bare name would collide across profile
///     classes (for example `consent/default` and `mcp/default`
///     would map to `agent/<user>.default`), which violates URA
///     uniqueness. Keep the `<profile>-<name>`
///     prefix so each profile carves its own agent-id space:
///
///       consent / default      → `consent-default`
///       mcp     / fs-bridge    → `mcp-fs-bridge`
///
/// Why encode the name (vs. the prior `a-<uuid>` style): the
/// Frontend Agents page reads agent URAs and renders `<agent_id>`
/// as the display label when no `display_name` metadata is
/// present. With a uuid-hash agent_id the user sees
/// `a-8c4523c3f3c94ed6931670c98a4e457e` — meaningless. With the
/// encoded name they see `probe-agent` (LLM) or `consent-default`
/// (system) and immediately know which agent they're looking at.
/// The agent URA is the canonical identity and always carries
/// the full information the resolver needs; piggybacking the
/// friendly name on it (rather than threading a parallel
/// `display_name` field through advertise → resolve → backend →
/// frontend) is the honest fix.
///
/// Why `-` (not `.`) as separator on the system path: URA v4.1.5
/// §A.URA-3 forbids dots inside `agent_id` — that namespace
/// belongs to ability URAs (`agent/<user>.<agent>.<verb>` where
/// each `<…>` is a single dot-free segment).
///
/// Stability: the encoded name MUST stay stable across daemon
/// restarts while the daemon remains paired to the same realm and
/// user. `local-agents.json` persists the URA on first mint; later
/// bootstrap passes reuse it only if it still matches the current
/// canonical identity.
///
/// Sanitisation: we trust the operator's `easynet agent add <name>`
/// argument (the CLI rejects names with `/`, `.`, whitespace, or
/// uppercase per `agent_spec.rs::validate_name`) so no further
/// stripping is needed.
pub struct UuidMinter;

impl UraMinter for UuidMinter {
    fn mint_id(&self, profile: &str, name: &str) -> String {
        if profile == "llm" {
            name.to_string()
        } else {
            format!("{profile}-{name}")
        }
    }
}

/// Walk the plan, mint URAs for any missing entries, persist,
/// and return the per-row outcomes. Pure-function except for
/// the persistence layer — accepts a `UraMinter` so tests can
/// drive deterministic IDs.
pub fn bootstrap_local_agents<M: UraMinter>(
    plan: &BootstrapPlan,
    file: &mut LocalAgentsFile,
    minter: &M,
) -> Vec<BootstrapOutcome> {
    // Update host URA before any minting so upserts record the
    // correct `signing_authority`.
    if !plan.host_device_ura.is_empty() {
        file.host_device_agent_ura = plan.host_device_ura.clone();
    }
    if plan.realm.is_empty() || plan.user_id.is_empty() {
        return Vec::new();
    }

    file.hosted_agents
        .retain(|e| is_current_agent_ura(&e.agent_ura, &plan.realm, &plan.user_id));
    if !file.host_device_agent_ura.is_empty() {
        let signing_authority = format!("hosted_by:{}", file.host_device_agent_ura);
        for entry in &mut file.hosted_agents {
            entry.signing_authority = signing_authority.clone();
        }
    }

    let mut outcomes = Vec::new();
    let mut process = |profile: &str, name: &str, outcomes: &mut Vec<BootstrapOutcome>| {
        let existing = file
            .hosted_agents
            .iter()
            .find(|e| e.profile == profile && e.name == name)
            .map(|e| e.agent_ura.clone());
        let (ura, reused) = match existing {
            Some(ura) => (ura, true),
            None => {
                let id = minter.mint_id(profile, name);
                let ura = crate::ura::agent_ura(&plan.realm, &plan.user_id, &id);
                upsert_hosted_agent(file, profile, name, &ura);
                (ura, false)
            }
        };
        outcomes.push(BootstrapOutcome {
            profile: profile.to_string(),
            name: name.to_string(),
            agent_ura: ura,
            reused,
        });
    };
    if plan.consent {
        process("consent", "default", &mut outcomes);
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
/// file (after a bootstrap pass), as `(profile, name, ura)`.
pub fn hosted_uras(file: &LocalAgentsFile) -> Vec<(String, String, String)> {
    file.hosted_agents
        .iter()
        .map(|e: &HostedAgentEntry| (e.profile.clone(), e.name.clone(), e.agent_ura.clone()))
        .collect()
}

/// True iff the URA is a canonical hosted-agent URA for the daemon's
/// current realm and user. Bootstrap reuses only rows that satisfy
/// this predicate; malformed, non-agent, and stale-realm rows are
/// local projection garbage under the clean RFC-005 identity model.
fn is_current_agent_ura(ura: &str, realm: &str, user_id: &str) -> bool {
    let Ok(parsed) = crate::ura::parse_ura(ura) else {
        return false;
    };
    if parsed.kind != crate::ura::URAKind::Agent {
        return false;
    }
    // Device-sponsored System Agents (`agent/device.<id>.<agent>`,
    // DEC-F048) intentionally fall out here: `agent_ids()` is None
    // for them and a device-owned agent never belongs to "the
    // current user" — None → false is the correct verdict, not an
    // unhandled grammar case (F-047 point 8).
    parsed.realm == realm && parsed.agent_ids().map(|(user, _)| user) == Some(user_id)
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

    impl UraMinter for CountingMinter {
        fn mint_id(&self, profile: &str, name: &str) -> String {
            let n = self.seq.get();
            self.seq.set(n + 1);
            format!("{profile}-{name}-{n}")
        }
    }

    fn plan_with(consent: bool, mcp: bool, llms: &[(&str, &str)]) -> BootstrapPlan {
        BootstrapPlan {
            realm: "acme".into(),
            user_id: "u1".into(),
            host_device_ura: "easynet:///r/acme/device/01DEV".into(),
            consent,
            mcp,
            llm_sub_agents: llms
                .iter()
                .map(|(n, t)| LlmSubAgent {
                    name: (*n).into(),
                    agent_type_display: (*t).into(),
                    model: None,
                })
                .collect(),
        }
    }

    #[test]
    fn bootstrap_mints_ura_for_each_enabled_profile() {
        let plan = plan_with(true, true, &[("claude", "claude-code")]);
        let mut file = LocalAgentsFile::default();
        let minter = CountingMinter::new();
        let outcomes = bootstrap_local_agents(&plan, &mut file, &minter);

        assert_eq!(outcomes.len(), 3); // consent + mcp + 1 llm
        for o in &outcomes {
            assert!(
                !o.reused,
                "first bootstrap pass must mint fresh URAs, got reused for {:?}",
                o.profile
            );
            // v4.1.4: hosted-agent URA is user-anchored
            // (`r/<realm>/agent/<user>.<id>`). plan_with seeds
            // user_id="u1".
            let expected_prefix = format!("{}u1.", crate::ura::realm_agent_prefix("acme"));
            assert!(o.agent_ura.starts_with(&expected_prefix));
        }
        // The daemon's own self-URA uses the device segment.
        assert_eq!(file.host_device_agent_ura, "easynet:///r/acme/device/01DEV");
        assert_eq!(file.hosted_agents.len(), 3);
    }

    #[test]
    fn second_bootstrap_reuses_existing_uras() {
        let plan = plan_with(true, false, &[]);
        let mut file = LocalAgentsFile::default();
        let minter = CountingMinter::new();
        let first = bootstrap_local_agents(&plan, &mut file, &minter);
        let consent_uri_v1 = first[0].agent_ura.clone();

        // Same plan, same file — every outcome must report reused=true
        // and the URA must match exactly.
        let second = bootstrap_local_agents(&plan, &mut file, &minter);
        assert_eq!(second.len(), 1);
        assert!(second[0].reused);
        assert_eq!(second[0].agent_ura, consent_uri_v1);
        assert_eq!(file.hosted_agents.len(), 1, "must not duplicate rows");
    }

    #[test]
    fn disabling_a_profile_leaves_existing_ura_intact_in_file() {
        // Mint consent + mcp on first boot. On second boot the operator
        // turned mcp=false; we expect the mcp row to remain in the file
        // (we don't garbage-collect on disable — operators may toggle
        // profiles temporarily, and we don't want to mint fresh URAs on
        // every flip).
        let mut file = LocalAgentsFile::default();
        let minter = CountingMinter::new();
        let _ = bootstrap_local_agents(&plan_with(true, true, &[]), &mut file, &minter);
        assert_eq!(file.hosted_agents.len(), 2);
        let _ = bootstrap_local_agents(&plan_with(true, false, &[]), &mut file, &minter);
        // mcp row still present
        assert!(file.hosted_agents.iter().any(|e| e.profile == "mcp"));
    }

    #[test]
    fn pre_join_realm_does_not_mint_placeholder_uras() {
        // Without a realm there is no canonical hosted-agent URA.
        // Clean RFC-005 bootstrap skips minting instead of writing
        // fake `<unjoined>` identities and repairing them later.
        let mut plan = plan_with(true, false, &[]);
        plan.realm = String::new();
        plan.host_device_ura = String::new();
        let mut file = LocalAgentsFile::default();
        let outcomes = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert!(outcomes.is_empty());
        assert!(file.host_device_agent_ura.is_empty());
        assert!(file.hosted_agents.is_empty());
    }

    #[test]
    fn uuid_minter_keeps_llm_names_bare_and_prefixes_system_profiles() {
        // The dispatch registry registers `<agent>.chat` /
        // `<agent>.invoke` / `<agent>.discover` under the raw
        // `agents.json::keys` (i.e. the operator's
        // `easynet agent add <name>` argument). The friendly URA
        // minted into `local-agents.json` MUST agree with that
        // raw name for the LLM profile, otherwise
        // `/v1/chat/completions model=<name>` resolves to
        // `agent/<user>.<name>` while the hub PresenceRegistry
        // only knows `agent/<user>.llm-<name>`, and routing
        // breaks with "not in PresenceRegistry".
        let m = UuidMinter;
        assert_eq!(m.mint_id("llm", "probe-agent"), "probe-agent");
        assert_eq!(m.mint_id("llm", "codex"), "codex");
        // System-managed profiles auto-generate generic names
        // (`default`, `fs-bridge`); the prefix carves a per-class
        // namespace so they never collide.
        assert_eq!(m.mint_id("consent", "default"), "consent-default");
        assert_eq!(m.mint_id("mcp", "fs-bridge"), "mcp-fs-bridge");
    }

    #[test]
    fn agent_type_for_returns_display_string() {
        let plan = plan_with(false, false, &[("claude", "claude-code")]);
        assert_eq!(
            agent_type_for(&plan, "claude").as_deref(),
            Some("claude-code")
        );
        assert_eq!(agent_type_for(&plan, "missing"), None);
    }

    #[test]
    fn outcomes_for_profile_filters_correctly() {
        let plan = plan_with(true, false, &[("claude", "claude-code")]);
        let mut file = LocalAgentsFile::default();
        let outcomes = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert_eq!(outcomes_for_profile(&outcomes, "consent").len(), 1);
        assert_eq!(outcomes_for_profile(&outcomes, "llm").len(), 1);
        assert_eq!(outcomes_for_profile(&outcomes, "mcp").len(), 0);
    }

    #[test]
    fn hosted_uras_returns_every_row_after_bootstrap() {
        let mut file = LocalAgentsFile::default();
        let _ = bootstrap_local_agents(
            &plan_with(true, true, &[]),
            &mut file,
            &CountingMinter::new(),
        );
        let rows = hosted_uras(&file);
        let profiles: std::collections::HashSet<&str> =
            rows.iter().map(|(p, _, _)| p.as_str()).collect();
        assert!(profiles.contains("consent"));
        assert!(profiles.contains("mcp"));
    }

    #[test]
    fn host_device_ura_persists_after_post_join_save() {
        // Pre-join: no canonical realm, so no hosted URA is minted.
        let mut file = LocalAgentsFile::default();
        let mut plan = plan_with(true, false, &[]);
        plan.realm = String::new();
        plan.host_device_ura = String::new();
        let _ = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert!(file.host_device_agent_ura.is_empty());
        assert!(file.hosted_agents.is_empty());

        // Post-join: realm + host URA now known. Re-run bootstrap.
        plan.realm = "acme".into();
        // URA v4.1.4: host device URA uses the `/device/` role
        // segment (Phase 2F) — the legacy `/agent/01DEV` collapsed
        // every profile under one role.
        plan.host_device_ura = "easynet:///r/acme/device/01DEV".into();
        let _ = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert_eq!(file.host_device_agent_ura, "easynet:///r/acme/device/01DEV");
    }

    #[test]
    fn existing_rows_refresh_signing_authority_when_host_device_changes() {
        let mut file = LocalAgentsFile {
            host_device_agent_ura: "easynet:///r/acme/device/OLD".into(),
            hosted_agents: vec![HostedAgentEntry {
                profile: "llm".into(),
                name: "claude".into(),
                agent_ura: "easynet:///r/acme/agent/u1.claude".into(),
                signing_authority: "hosted_by:easynet:///r/acme/device/OLD".into(),
                first_seen_at: String::new(),
            }],
        };
        let plan = plan_with(false, false, &[("claude", "claude-code")]);
        let outcomes = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].reused);
        assert_eq!(file.host_device_agent_ura, "easynet:///r/acme/device/01DEV");
        assert_eq!(
            file.hosted_agents[0].signing_authority,
            "hosted_by:easynet:///r/acme/device/01DEV"
        );
    }

    // ── current canonical row reuse ───────────────────────────────

    #[test]
    fn malformed_agent_ura_referenced_by_plan_is_replaced() {
        let mut file = LocalAgentsFile {
            hosted_agents: vec![HostedAgentEntry {
                profile: "consent".into(),
                name: "default".into(),
                agent_ura: "easynet:///r/acme/agent/consent-default-0".into(),
                signing_authority: String::new(),
                first_seen_at: String::new(),
            }],
            ..Default::default()
        };
        let plan = plan_with(true, false, &[]);
        let outcomes = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].reused, "malformed row must NOT be reused");
        assert_eq!(
            outcomes[0].agent_ura,
            "easynet:///r/acme/agent/u1.consent-default-0"
        );
        assert_eq!(file.hosted_agents.len(), 1);
        assert_eq!(file.hosted_agents[0].agent_ura, outcomes[0].agent_ura);
    }

    #[test]
    fn stale_rows_are_pruned_post_join() {
        let mut file = LocalAgentsFile {
            hosted_agents: vec![
                HostedAgentEntry {
                    profile: "llm".into(),
                    name: "old-agent".into(),
                    agent_ura: "easynet:///r/old-realm/agent/old-tail".into(),
                    signing_authority: String::new(),
                    first_seen_at: String::new(),
                },
                HostedAgentEntry {
                    profile: "consent".into(),
                    name: "default".into(),
                    agent_ura: "easynet:///r/acme/agent/u1.consent-keep".into(),
                    signing_authority: String::new(),
                    first_seen_at: String::new(),
                },
            ],
            ..Default::default()
        };
        let plan = plan_with(true, false, &[]);
        let _ = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        let names: Vec<_> = file
            .hosted_agents
            .iter()
            .map(|e| (e.profile.clone(), e.name.clone()))
            .collect();
        assert!(
            !names.contains(&("llm".into(), "old-agent".into())),
            "orphan malformed row must be pruned"
        );
        assert!(
            names.contains(&("consent".into(), "default".into())),
            "canonical row referenced by plan must survive"
        );
    }

    #[test]
    fn pre_join_does_not_prune_existing_rows() {
        // Pre-join bootstrap has no canonical identity to compare
        // against. It does not mint and it does not mutate hosted
        // rows; post-join bootstrap performs the clean prune.
        let mut file = LocalAgentsFile {
            hosted_agents: vec![HostedAgentEntry {
                profile: "llm".into(),
                name: "old-agent".into(),
                agent_ura: "easynet:///r/old-realm/agent/old-tail".into(),
                signing_authority: String::new(),
                first_seen_at: String::new(),
            }],
            ..Default::default()
        };
        let mut plan = plan_with(false, false, &[]);
        plan.realm = String::new();
        plan.user_id = String::new();
        plan.host_device_ura = String::new();
        let _ = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert_eq!(file.hosted_agents.len(), 1, "pre-join must not prune");
    }

    #[test]
    fn stale_realm_canonical_ura_is_replaced_without_preserving_tail() {
        let mut file = LocalAgentsFile {
            hosted_agents: vec![HostedAgentEntry {
                profile: "llm".into(),
                name: "claude".into(),
                agent_ura: "easynet:///r/old/agent/old-user.a-keep-me".into(),
                signing_authority: String::new(),
                first_seen_at: String::new(),
            }],
            ..Default::default()
        };
        let plan = plan_with(false, false, &[("claude", "claude-code")]);
        let outcomes = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].reused);
        assert_eq!(
            outcomes[0].agent_ura,
            "easynet:///r/acme/agent/u1.llm-claude-0"
        );
    }

    // ── helpers: is_current_agent_ura ────────────────────

    #[test]
    fn is_current_agent_ura_accepts_current_agent_shape() {
        assert!(is_current_agent_ura(
            "easynet:///r/acme/agent/u1.claude",
            "acme",
            "u1"
        ));
    }

    #[test]
    fn is_current_agent_ura_rejects_non_current_or_non_agent_shapes() {
        assert!(!is_current_agent_ura(
            "easynet:///r/old/agent/u1.a-1",
            "acme",
            "u1"
        ));
        assert!(!is_current_agent_ura(
            "easynet:///r/acme/agent/old-user.a-1",
            "acme",
            "u1"
        ));
        assert!(!is_current_agent_ura(
            "easynet:///r/acme/agent/collapsed-tail",
            "acme",
            "u1"
        ));
        assert!(!is_current_agent_ura(
            "easynet:///r/acme/device/01DEV",
            "acme",
            "u1"
        ));
        assert!(!is_current_agent_ura("agent://self", "acme", "u1"));
    }
}

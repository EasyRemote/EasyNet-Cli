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

/// Production URI minter — encodes the operator-meaningful name
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
///   * **System-managed profiles** (`consent`, `policy`, `mcp`):
///     the name is auto-generated and generic (`default`,
///     `fs-bridge`). The bare name would collide across profile
///     classes (a hypothetical `consent/default` and
///     `policy/default` would map to `agent/<user>.default`),
///     which violates URA uniqueness. Keep the `<profile>-<name>`
///     prefix so each profile carves its own agent-id space:
///
///       consent / default      → `consent-default`
///       policy  / default      → `policy-default`
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
/// belongs to ability URIs (`agent/<user>.<agent>.<verb>` where
/// each `<…>` is a single dot-free segment).
///
/// Stability: the encoded name MUST stay stable across daemon
/// restarts (re-pairing must produce the same URA so backend
/// receipts + frontend bookmarks keep working). `local-agents.json`
/// persists the URA on first mint; the bootstrap repair path
/// reuses it on subsequent runs.
///
/// Sanitisation: we trust the operator's `easynet agent add <name>`
/// argument (the CLI rejects names with `/`, `.`, whitespace, or
/// uppercase per `agent_spec.rs::validate_name`) so no further
/// stripping is needed.
pub struct UuidMinter;

impl UriMinter for UuidMinter {
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
    // Drop orphan hosted-agent rows whose URI is structurally
    // malformed under v4.1.5 §A.URA-3 (agent tail must be
    // `<user>.<agent>` with both segments non-empty). These are
    // legacy rows persisted by daemons that pre-date the strict
    // tail rule — `easynet:///r/acme/agent/consent-default-0` and
    // friends, where the entire tail collapsed into one bare
    // segment. We only drop rows the current plan does NOT
    // re-reference: rows the plan still owns by `(profile, name)`
    // hit the repair branch below and get re-minted with the
    // canonical realm + user_id.
    //
    // Rows referenced by the current plan are left to the repair
    // branch even if malformed — we want repair to win over drop
    // there so a transient plan-flip (operator toggles policy
    // off/on) does not lose the persisted agent_id continuity.
    //
    // Pre-join (no realm yet) we leave malformed rows alone — the
    // `<unjoined>` placeholders ARE structurally valid agent URAs
    // (tail parses, just opaque), and any other malformed row
    // will be reconsidered on the post-join bootstrap pass.
    if !plan.realm.is_empty() && !plan.user_id.is_empty() {
        let referenced = plan_referenced_keys(plan);
        file.hosted_agents.retain(|e| {
            if is_canonical_agent_uri(&e.agent_uri) {
                return true;
            }
            referenced.contains(&(e.profile.clone(), e.name.clone()))
        });
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
                if needs_repair(&existing_uri, &plan.realm, &plan.user_id)
                    && !plan.realm.is_empty()
                    && !plan.user_id.is_empty() =>
            {
                // Repair path: this row was minted under a stale
                // shape (`<unjoined>` placeholder from pre-join,
                // legacy non-`<user>.<agent>` tail from a v1 daemon,
                // or a different realm/user from a previous pairing).
                // Any of those break hub-side `federation.resolve`
                // visibility filtering, so the agent shows up locally
                // but never via `/api/v1/agents`. With the real realm
                // + user_id known we keep the agent_id tail when it
                // parses canonically (operators may have referenced
                // it elsewhere) and otherwise mint a fresh one.
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

/// True iff the URI parses as a canonical v4.1.5 agent URA:
/// `easynet:///r/<realm>/agent/<user>.<agent>` with both tail
/// segments non-empty and free of internal dots / slashes. This
/// is what `parse_ura(uri).kind == Agent` already enforces; we
/// surface it as a named predicate so callers reading the
/// bootstrap logic can match it against the prose ("structurally
/// valid agent URA").
fn is_canonical_agent_uri(uri: &str) -> bool {
    matches!(
        crate::uri::parse_ura(uri).map(|p| p.kind),
        Ok(crate::uri::URAKind::Agent)
    )
}

/// True iff the URI either fails the strict canonical shape check
/// OR was minted under a different realm/user than the current
/// plan. Either condition means the row's identity is stale for
/// this daemon's current pairing — repair re-mints it under the
/// real `(realm, user_id)` so hub-side `federation.resolve`
/// visibility filtering and every downstream URA consumer (the
/// `device.meta.list_abilities` synth, advertise, …) see one truth.
///
/// The `<unjoined>` placeholders parse fine structurally but
/// always trip the realm/user mismatch arm because the literal
/// strings `<unjoined>` cannot be the real realm or user_id.
fn needs_repair(uri: &str, realm: &str, user_id: &str) -> bool {
    let Ok(parsed) = crate::uri::parse_ura(uri) else {
        return true;
    };
    if parsed.kind != crate::uri::URAKind::Agent {
        return true;
    }
    parsed.realm != realm || parsed.user_id != user_id
}

/// Set of `(profile, name)` keys the current plan still owns.
/// Used by the orphan-pruning pass at the top of
/// `bootstrap_local_agents` to decide which malformed rows can be
/// safely dropped vs which ones the repair branch will rewrite.
fn plan_referenced_keys(plan: &BootstrapPlan) -> std::collections::HashSet<(String, String)> {
    let mut s = std::collections::HashSet::new();
    if plan.consent {
        s.insert(("consent".into(), "default".into()));
    }
    if plan.policy {
        s.insert(("policy".into(), "default".into()));
    }
    if plan.mcp {
        s.insert(("mcp".into(), "default".into()));
    }
    for sub in &plan.llm_sub_agents {
        s.insert(("llm".into(), sub.name.clone()));
    }
    s
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
        assert_eq!(m.mint_id("policy", "default"), "policy-default");
        assert_eq!(m.mint_id("mcp", "fs-bridge"), "mcp-fs-bridge");
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

    // ── repair / prune of malformed legacy rows ────────────────────

    #[test]
    fn malformed_agent_uri_referenced_by_plan_is_repaired() {
        // Legacy daemon persisted `easynet:///r/acme/agent/consent-default-0`
        // — tail collapsed into one bare segment, fails v4.1.5
        // §A.URA-3. Today's plan still owns `(consent, default)`,
        // so the row must be re-minted (NOT reused as-is) with the
        // current realm + user_id.
        let mut file = LocalAgentsFile {
            hosted_agents: vec![HostedAgentEntry {
                profile: "consent".into(),
                name: "default".into(),
                agent_uri: "easynet:///r/acme/agent/consent-default-0".into(),
                signing_authority: String::new(),
                first_seen_at: String::new(),
            }],
            ..Default::default()
        };
        let plan = plan_with(true, false, false, &[]);
        let outcomes = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].reused, "malformed row must NOT be reused");
        assert!(
            outcomes[0]
                .agent_uri
                .starts_with("easynet:///r/acme/agent/u1."),
            "expected canonical user-anchored URA, got {:?}",
            outcomes[0].agent_uri
        );
        assert!(
            crate::uri::parse_ura(&outcomes[0].agent_uri).is_ok(),
            "repaired URA must parse strictly"
        );
    }

    #[test]
    fn malformed_agent_uri_orphan_row_is_pruned_post_join() {
        // Legacy stale row from a previous pairing — different
        // realm, malformed shape, NOT referenced by the current
        // plan. It is dead data: drop on first post-join boot.
        let mut file = LocalAgentsFile {
            hosted_agents: vec![
                HostedAgentEntry {
                    profile: "llm".into(),
                    name: "old-agent".into(),
                    agent_uri: "easynet:///r/old-realm/agent/old-tail".into(),
                    signing_authority: String::new(),
                    first_seen_at: String::new(),
                },
                HostedAgentEntry {
                    profile: "consent".into(),
                    name: "default".into(),
                    agent_uri: "easynet:///r/acme/agent/u1.consent-keep".into(),
                    signing_authority: String::new(),
                    first_seen_at: String::new(),
                },
            ],
            ..Default::default()
        };
        // Plan does NOT include (llm, old-agent); it includes
        // (consent, default) under the canonical URA already.
        let plan = plan_with(true, false, false, &[]);
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
    fn pre_join_does_not_prune_malformed_rows() {
        // Pre-join (no realm yet): leave the file alone. Even
        // malformed rows must survive — they get reconsidered on
        // the post-join pass when we know the real realm + user.
        let mut file = LocalAgentsFile {
            hosted_agents: vec![HostedAgentEntry {
                profile: "llm".into(),
                name: "old-agent".into(),
                agent_uri: "easynet:///r/old-realm/agent/old-tail".into(),
                signing_authority: String::new(),
                first_seen_at: String::new(),
            }],
            ..Default::default()
        };
        let mut plan = plan_with(false, false, false, &[]);
        plan.realm = String::new();
        plan.user_id = String::new();
        plan.host_device_uri = String::new();
        let _ = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert_eq!(file.hosted_agents.len(), 1, "pre-join must not prune");
    }

    #[test]
    fn stale_realm_canonical_uri_is_repaired() {
        // Row parses as a canonical agent URA but its realm and
        // user_id belong to a previous pairing. We are now paired
        // to a different realm; repair must re-mint under the
        // current `(realm, user_id)` so federation visibility
        // filtering keys off the right user.
        let mut file = LocalAgentsFile {
            hosted_agents: vec![HostedAgentEntry {
                profile: "llm".into(),
                name: "claude".into(),
                agent_uri: "easynet:///r/old/agent/old-user.a-keep-me".into(),
                signing_authority: String::new(),
                first_seen_at: String::new(),
            }],
            ..Default::default()
        };
        let plan = plan_with(false, false, false, &[("claude", "claude-code")]);
        let outcomes = bootstrap_local_agents(&plan, &mut file, &CountingMinter::new());
        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].reused);
        // agent_id tail is preserved (operators may have referenced it).
        assert_eq!(
            outcomes[0].agent_uri,
            "easynet:///r/acme/agent/u1.a-keep-me"
        );
    }

    // ── helpers: is_canonical_agent_uri / needs_repair ─────────────

    #[test]
    fn is_canonical_agent_uri_accepts_v4_1_5_agent_shape() {
        assert!(is_canonical_agent_uri(
            "easynet:///r/acme/agent/alice.claude"
        ));
    }

    #[test]
    fn is_canonical_agent_uri_rejects_collapsed_tail() {
        // The bug shape: tail with no `.` separator. Pre-v4.1.5
        // daemons emitted this and it has been silently reused
        // across boots.
        assert!(!is_canonical_agent_uri(
            "easynet:///r/acme/agent/consent-default-0"
        ));
    }

    #[test]
    fn is_canonical_agent_uri_rejects_non_agent_kinds() {
        assert!(!is_canonical_agent_uri("easynet:///r/acme/device/01DEV"));
        assert!(!is_canonical_agent_uri("easynet:///r/acme/hub"));
        assert!(!is_canonical_agent_uri("agent://self"));
        assert!(!is_canonical_agent_uri(""));
    }

    #[test]
    fn needs_repair_flags_realm_or_user_drift() {
        // Same shape, wrong realm → repair.
        assert!(needs_repair("easynet:///r/old/agent/u1.a-1", "acme", "u1"));
        // Same shape, wrong user → repair.
        assert!(needs_repair(
            "easynet:///r/acme/agent/old-user.a-1",
            "acme",
            "u1"
        ));
        // Match on both → no repair.
        assert!(!needs_repair(
            "easynet:///r/acme/agent/u1.a-1",
            "acme",
            "u1"
        ));
        // Unparseable → repair.
        assert!(needs_repair(
            "easynet:///r/acme/agent/collapsed-tail",
            "acme",
            "u1"
        ));
    }
}

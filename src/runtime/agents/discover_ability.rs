// EasyNet CLI — <self>.discover ability handler
// =================================================================
//
// File: src/runtime/agents/discover_ability.rs
//
// Per-agent ability discovery walking the three-tier ladder taught
// by the `delegate` SKILL.md:
//
//   Tier 1  scope = "self"     — abilities owned by the calling agent
//   Tier 2  scope = "device"   — abilities published by other agents
//                                on this device whose [access].
//                                visibility ≥ device
//   Tier 3  scope = "user"     — abilities published within the caller's
//                                realm. Calls `federation.resolve` against
//                                the realm's hub (Hub-profile ability,
//                                RFC-001 §A14) and projects each
//                                `ResolvedAgent` into the same `Candidate`
//                                envelope as the local tiers.
//   Tier 4  scope = "public"   — explicit cross-tenant hub catalogue.
//                                Federation failures (no realm joined /
//                                hub call dropped) surface as typed
//                                `federation_not_joined` /
//                                `federation_unavailable` envelopes so the
//                                LLM falls through gracefully.
//
// Why "<self>.discover" and not the legacy "easynet.discover"
// -----------------------------------------------------------
// The ability-only model says every ability belongs to some owner.
// `easynet.*` was a single shared registration, which made the
// "discover" verb look anonymous; in practice each agent has its own
// view of the registry (its own "self" tier), so the handler must be
// owner-aware. Registering under `<agent>.discover` per agent gives
// the handler the agent's identity by construction.
//
// Retired flat discover aliases must not own this verb. The `delegate`
// skill teaches the owner-namespaced form so a fresh install lands on
// the canonical name.
//
// Output shape
// ------------
//   { "candidates": [
//       {
//         "qualified_name": "easynet:///r/acme/ability/user-1.claude.weather",
//         "owner":          "claude",
//         "ability":        "weather",
//         "description":    "...",
//         "input_schema":   { ... },
//         "visibility":     "device",
//         "score":          0.92,
//         "reason":         "title match + tag match",
//         "scope_matched":  "device"
//       },
//       ...
//     ],
//     "scope":   "device",
//     "query":   "weather"
//   }
//
// `scope_matched` is the actual tier the candidate came from (lets a
// caller that asked for `scope: "device"` see which entries are
// strictly self vs broader-but-allowed).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::core::ability_spec::{AbilityManifest, Visibility};
use crate::registry::agents::{AgentEntry, AgentRegistry};
use crate::runtime::ability_dispatch::AxonAbilityCatalog;

/// Verb portion of the per-agent discover ability. Combined with the
/// owning agent's name to form the wire-level `<agent>.discover`.
pub const ABILITY_VERB: &str = "discover";

/// Register `<agent_name>.discover` on the registry. Each agent gets
/// its own copy of this self-bundle ability — the handler closes over
/// the agent's name so calls from MCP / EAL never need to pass an
/// explicit caller identity.
///
/// `agent_registry_provider` is invoked at handler-call time so that
/// hot-added or hot-removed peer agents are reflected on the next
/// discover call without re-registration.
pub fn register_for_agent<F>(
    reg: &mut AxonAbilityCatalog,
    agent_name: String,
    agent_registry_provider: F,
    dispatch_registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) where
    F: Fn() -> AgentRegistry + Send + Sync + 'static,
{
    use crate::runtime::ability_dispatch::OwnerKind;
    let provider: Arc<dyn Fn() -> AgentRegistry + Send + Sync> = Arc::new(agent_registry_provider);
    let qualified = format!("{agent_name}.{ABILITY_VERB}");
    let agent = agent_name.clone();
    reg.register_rpc_with_spec(
        &qualified,
        OwnerKind::Agent(agent_name),
        manifest(),
        Arc::new(move |args: Value| dispatch(&agent, &provider, &dispatch_registry_handle, args)),
    );
}

/// Public per-call entry point. Validates `scope`, applies `query`
/// filtering, returns the standardised `{candidates, scope, query}`
/// envelope.
///
/// Provider routing
/// ----------------
/// When the call passes `provider = "<owner>.<verb>"`, dispatch
/// hands off to that ability instead of running the builtin
/// BM25-lite scorer. The provider must satisfy the same input/
/// output contract (accepts `{scope, query, top_k}`, returns
/// `{candidates, scope, query}`). Builtin is the default.
///
/// Exposed so the dynamic per-agent fallback resolver in
/// `chat_ability::register_dynamic_agent_fallback` can synthesise a
/// handler for a hot-added agent without re-running this module's
/// register_for_agent (which requires `&mut AxonAbilityCatalog`).
pub fn dispatch(
    self_agent: &str,
    agent_registry_provider: &Arc<dyn Fn() -> AgentRegistry + Send + Sync>,
    dispatch_registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    args: Value,
) -> anyhow::Result<Value> {
    // Provider routing happens BEFORE scope/query parsing because the
    // delegate provider may interpret the args differently (the
    // shape is contract-stable, but a third-party provider can be
    // stricter than ours). We forward the args verbatim and let
    // the provider validate.
    if let Some(provider_name) = args.get("provider").and_then(Value::as_str) {
        if !provider_name.is_empty() {
            return delegate_to_provider(
                provider_name,
                dispatch_registry_handle,
                strip_provider_field(&args),
            );
        }
    }

    let scope = parse_scope(&args)?;
    let query = parse_query(&args);
    let top_k = parse_top_k(&args)?;

    if scope.is_federated() {
        // Tier 3 — federation. Dial the daemon's hub via
        // `federation.resolve`, project the receipt into the same
        // `Candidate[]` shape the local tiers return so the LLM
        // sees a uniform output regardless of where the candidate
        // came from.
        //
        // `User` scope passes the auto-fill empty filter so the hub
        // scopes results to the caller's own tenant — the answer
        // to "what agents do I have on my account, across all my
        // devices". `Public` scope passes the explicit `*` literal
        // to opt into cross-tenant catalog browsing.
        //
        // Three failure modes, all surfaced as a typed envelope so
        // the LLM falls through gracefully (no Err that the
        // dispatch layer would escalate):
        //
        //   * `federation_not_joined`   — daemon isn't joined to a
        //                                  realm yet; nothing to query.
        //   * `federation_unavailable`  — hub call failed (transport,
        //                                  hub-side rejection).
        //   * `federation_empty`        — hub returned no agents at
        //                                  all. Surfaced as ok with
        //                                  `candidates = []`, not as
        //                                  an error.
        return resolve_via_federation(scope, query.as_deref(), top_k);
    }

    let agents = agent_registry_provider();
    let local_agent_uras = LocalAgentAbilityOwners::load();
    let mut rows: Vec<Candidate> = Vec::new();
    for (peer_name, peer_entry) in agents.agents.iter() {
        let manifests = crate::runtime::abilities::manifests_for(peer_name, peer_entry);
        for m in manifests {
            push_candidate(
                &mut rows,
                &local_agent_uras,
                self_agent,
                peer_name,
                peer_entry,
                &m,
                scope,
            );
        }
    }

    if let Some(q) = &query {
        score_against_query(&mut rows, q);
        rows.retain(|c| c.score > 0.0);
    } else {
        // No query → uniform 1.0 so ordering falls back to the
        // alphabetical tiebreaker.
        for r in rows.iter_mut() {
            r.score = 1.0;
        }
    }

    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.qualified_name.cmp(&b.qualified_name))
    });
    rows.truncate(top_k);

    let candidates: Vec<Value> = rows.iter().map(Candidate::to_json).collect();
    Ok(json!({
        "candidates": candidates,
        "scope": scope.as_str(),
        "query": query,
    }))
}

/// Forward a discover call to a third-party provider ability. The
/// provider is named in `<owner>.<verb>` form and resolved through
/// the same dispatch registry every other ability uses; we strip
/// the `provider` field so the downstream handler sees the args it
/// declared in its own input_schema, not a recursion-trigger.
fn delegate_to_provider(
    provider_name: &str,
    dispatch_registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    args: Value,
) -> anyhow::Result<Value> {
    if !provider_name.contains('.') {
        anyhow::bail!("discover: provider {provider_name:?} must use the `<owner>.<verb>` form");
    }
    let registry = dispatch_registry_handle.get().ok_or_else(|| {
        anyhow::anyhow!(
            "internal_error: dispatch registry handle not yet set; \
             discover provider routing requires the daemon's live registry"
        )
    })?;
    registry
        .invoke_rpc_json(provider_name, args)
        .map_err(|err| {
            anyhow::anyhow!(
                "discover: provider {provider_name:?} is not registered or failed. Pick from \
             the abilities your `<self>.discover()` lists with names ending \
             in `.discover` (or `.semantic_discover` etc), or omit the \
             `provider` argument to use the builtin BM25 matcher. ({err})"
            )
        })
}

/// Build a clone of `args` with the top-level `provider` field
/// stripped. Used before forwarding to the named provider so it
/// doesn't see a self-reference and re-enter routing.
fn strip_provider_field(args: &Value) -> Value {
    match args {
        Value::Object(map) => {
            let mut clone = map.clone();
            clone.remove("provider");
            Value::Object(clone)
        }
        other => other.clone(),
    }
}

/// Federation-tier resolution. Dials the realm's hub via
/// `federation.resolve`, parses the receipt, projects each
/// `ResolvedAgent` into the `Candidate` shape the local tiers
/// already return.
///
/// Why a helper that builds its own bridge per call rather than a
/// shared OnceLock-stashed pool: federation discovery is a
/// human-paced action (LLM probing the realm for an ability), not
/// a tight loop. The construction cost (~1 ms for a fresh bridge)
/// is dominated by the hub round-trip; the operational complexity
/// of threading another OnceLock through the dispatch layer
/// outweighs the perf win. If realm discovery becomes hot, swap to a
/// stashed `BridgePool` here without touching callers.
fn resolve_via_federation(
    scope: Scope,
    query: Option<&str>,
    top_k: usize,
) -> anyhow::Result<Value> {
    let (bridge, state) = match crate::persistence::config::load_and_connect() {
        Ok(pair) => pair,
        Err(e) => {
            return Ok(error_envelope(
                "federation_not_joined",
                &format!(
                    "no usable runtime state ({e}); start the daemon and join \
                     a realm before scope=\"user\" or scope=\"public\""
                ),
                scope,
                query,
            ));
        }
    };

    let creds = match crate::persistence::config::load_credentials() {
        Ok(c) => c,
        Err(_) => {
            return Ok(error_envelope(
                "federation_not_joined",
                "no credentials.json; run `easynet device join` to register \
                 with a hub before scope=\"user\" or scope=\"public\"",
                scope,
                query,
            ));
        }
    };
    // Realm doubles as tenant in the v1 wire shape (see
    // `build_bootstrap_plan` in facade::cli::start). A future config
    // split separates them; until then the same string flows into
    // both fields and `federation.resolve` accepts it as the realm
    // segment.
    let _ = state;
    let realm = creds.realm.clone();
    let tenant = creds.realm.as_str();
    if realm.is_empty() {
        return Ok(error_envelope(
            "federation_not_joined",
            "credentials.json carries an empty tenant; rejoin via \
             `easynet device join` before scope=\"user\" or scope=\"public\"",
            scope,
            query,
        ));
    }

    // Pin the caller URA to the daemon's own device-profile Agent
    // URA. Without this the bridge synthesises a hub-literal caller
    // (`agents/easynet:prv:hub:<realm>`) and the hub's membership
    // gate rejects with AXON_MEMBERSHIP_REQUIRED — same caller-URA
    // fix we apply at the daemon-boot advertise call site (see
    // `facade::cli::start::republish_via_federation_best_effort`).
    let device_caller_ura = crate::ura::device_ura(tenant, &creds.node_id);
    let invoker = crate::runtime::advertise::BridgeAbilityInvoker::with_caller_ura(
        &bridge,
        device_caller_ura,
    );
    // Tenant_filter wire shape mirrors RFC-002 §5 update:
    //   * User scope → None: hub auto-fills caller_tenant.
    //   * Public scope → "*": cross-tenant catalog listing.
    let tenant_filter = match scope {
        Scope::Public => Some("*".to_string()),
        _ => None,
    };
    let resolved = match crate::runtime::advertise::resolve_agents_with_filter(
        &invoker,
        tenant,
        &realm,
        "",
        true,
        tenant_filter,
    ) {
        Ok(r) => r,
        Err(e) => {
            return Ok(error_envelope(
                "federation_unavailable",
                &format!("federation.resolve against realm {realm:?} failed: {e}"),
                scope,
                query,
            ));
        }
    };

    let mut rows: Vec<Candidate> = Vec::new();
    for agent in resolved {
        if agent.status != "active" {
            // Skip revoked / suspended agents — the LLM shouldn't
            // pick a candidate the hub knows is gone.
            continue;
        }
        let owner = agent.ura.clone();
        for desc in agent.ability_summaries {
            let Some(summary) = crate::runtime::owner_projection::summary_from_value(&desc) else {
                continue;
            };
            let Some(bare_ability) =
                crate::runtime::owner_projection::summary_public_name(&summary)
            else {
                continue;
            };
            if let Some(candidate) =
                candidate_from_federated_summary(&owner, &summary, bare_ability, scope)
            {
                rows.push(candidate);
            }
        }
    }

    // **Cross-hub device axis (PR-N3 N3-4 surface)**. Only `Public`
    // scope consults the federated directory; see
    // [`federated_directory_candidates`] for the projection contract
    // and why other scopes are excluded.
    if matches!(scope, Scope::Public) {
        // Routed through the feature-agnostic shim so this branch
        // compiles regardless of the `axon-pb` feature. With the
        // feature off the shim returns `Ok(vec![])`, which is
        // exactly the "no federated entries" case the helper
        // already handles.
        if let Ok(entries) =
            crate::support::federation_invoke_shim::invoke_federation_discover(None, None)
        {
            rows.extend(federated_directory_candidates(&entries));
        }
    }

    if let Some(q) = query {
        score_against_query(&mut rows, q);
        rows.retain(|c| c.score > 0.0);
    } else {
        for r in rows.iter_mut() {
            r.score = 1.0;
        }
    }
    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.qualified_name.cmp(&b.qualified_name))
    });
    rows.truncate(top_k);

    let candidates: Vec<Value> = rows.iter().map(Candidate::to_json).collect();
    Ok(json!({
        "candidates": candidates,
        "scope": scope.as_str(),
        "query": query,
    }))
}

/// Where the call wants to look. Mirrors the `[access].visibility`
/// tiers exposed in `core::ability_spec::Visibility` but is a
/// caller-side concept: "search at most this far".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Selfish,
    Device,
    /// All agents under the calling daemon's tenant. Includes
    /// other devices owned by the same user. This is the default
    /// federation query.
    User,
    /// Cross-tenant hub catalog. Returns every advertised agent
    /// regardless of tenant. Opt-in for explicit cross-user
    /// discovery; not the default.
    Public,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Scope::Selfish => "self",
            Scope::Device => "device",
            Scope::User => "user",
            Scope::Public => "public",
        }
    }

    /// True when the scope dispatches through the federation hub
    /// rather than the local agent registry. User and Public both
    /// fan out to the hub; Selfish and Device stay local.
    fn is_federated(self) -> bool {
        matches!(self, Scope::User | Scope::Public)
    }
}

/// Wire-accepted spellings for the `scope` argument, in the order
/// they should appear in the JSON-Schema `enum`. The list is the
/// single source of truth for both [`parse_scope`] (the runtime
/// acceptor) and [`input_schema`] (the contract the LLM sees) —
/// adding a new spelling means editing this array and the match in
/// `parse_scope`; the [`scope_enum_matches_parser_acceptance`] test
/// catches any drift between the two.
const ACCEPTED_SCOPE_LITERALS: &[&str] = &["self", "device", "user", "public"];

fn parse_scope(args: &Value) -> anyhow::Result<Scope> {
    let raw = args.get("scope").and_then(Value::as_str).unwrap_or("self");
    match raw {
        "self" => Ok(Scope::Selfish),
        "device" => Ok(Scope::Device),
        "user" => Ok(Scope::User),
        "public" => Ok(Scope::Public),
        other => anyhow::bail!(
            "discover: scope = {other:?} is not one of {:?}",
            ACCEPTED_SCOPE_LITERALS,
        ),
    }
}

fn parse_query(args: &Value) -> Option<String> {
    args.get("query")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn parse_top_k(args: &Value) -> anyhow::Result<usize> {
    let Some(v) = args.get("top_k") else {
        return Ok(DEFAULT_TOP_K);
    };
    let n = v.as_u64().ok_or_else(|| {
        anyhow::anyhow!("discover: top_k must be a non-negative integer; got {v}")
    })?;
    if n == 0 {
        anyhow::bail!("discover: top_k = 0 returns nothing; either omit it or pass a positive int");
    }
    Ok(n as usize)
}

const DEFAULT_TOP_K: usize = 20;

/// One discover result row. Held as a struct (not a JSON value) so
/// `score_against_query` can mutate the score in place without re-
/// parsing.
#[derive(Debug, Clone)]
struct Candidate {
    qualified_name: String,
    owner: String,
    ability: String,
    description: String,
    input_schema: Value,
    visibility: Visibility,
    scope_matched: Scope,
    score: f64,
    reason: String,
    fulfilled_by: Option<&'static str>,
}

#[derive(Debug, Clone, Default)]
struct LocalAgentAbilityOwners {
    local_agents: crate::persistence::local_agents::LocalAgentsFile,
}

impl LocalAgentAbilityOwners {
    fn load() -> Self {
        Self {
            local_agents: crate::persistence::local_agents::load().unwrap_or_default(),
        }
    }

    fn owner_ura_for(&self, agent_name: &str) -> Option<String> {
        crate::persistence::local_agents::lookup_hosted_ura(&self.local_agents, "llm", agent_name)
    }

    fn ability_ura_for(&self, agent_name: &str, public_name: &str) -> Option<String> {
        let owner_ura = self.owner_ura_for(agent_name)?;
        crate::ura::owner_ability_ura(&owner_ura, public_name)
    }
}

impl Candidate {
    fn to_json(&self) -> Value {
        json!({
            "qualified_name": self.qualified_name,
            "owner":          self.owner,
            "ability":        self.ability,
            "description":    self.description,
            "input_schema":   self.input_schema,
            "visibility":     self.visibility.as_wire_str(),
            "scope_matched":  self.scope_matched.as_str(),
            "score":          self.score,
            "reason":         self.reason,
            "fulfilled_by":   self.fulfilled_by.map(Value::from).unwrap_or(Value::Null),
        })
    }
}

fn candidate_from_federated_summary(
    owner: &str,
    summary: &crate::runtime::owner_projection::AbilityProjectionSummary,
    public_name: String,
    scope: Scope,
) -> Option<Candidate> {
    let ability_ura = summary.ability_ura.trim();
    if ability_ura.is_empty() {
        return None;
    }
    let input_schema = match (
        summary.schema_ref.as_deref(),
        summary.schema_hash.as_deref(),
    ) {
        (None, None) => Value::Null,
        (schema_ref, schema_hash) => json!({
            "schema_ref": schema_ref,
            "schema_hash": schema_hash,
        }),
    };
    Some(Candidate {
        qualified_name: ability_ura.to_string(),
        owner: owner.to_string(),
        ability: public_name,
        description: String::new(),
        input_schema,
        visibility: Visibility::Public,
        scope_matched: scope,
        score: 0.0,
        reason: String::new(),
        fulfilled_by: Some("federation"),
    })
}

/// Decide whether one peer ability satisfies the requested scope and
/// the ability's own `[access]` policy, then push a row.
fn push_candidate(
    out: &mut Vec<Candidate>,
    local_agent_uras: &LocalAgentAbilityOwners,
    self_agent: &str,
    peer_name: &str,
    _peer_entry: &AgentEntry,
    manifest: &crate::core::ability_spec::AbilityManifest,
    scope: Scope,
) {
    let access = manifest.access();
    let visibility = access.visibility;

    let is_self = peer_name == self_agent;
    let scope_matched = match (is_self, visibility) {
        (true, _) => Scope::Selfish,
        // A peer ability with `visibility = "self"` is invisible to
        // other agents regardless of caller scope — the access policy
        // strictly trumps the caller's reach.
        (false, Visibility::Selfish) => return,
        (false, Visibility::Device) => Scope::Device,
        (false, Visibility::Public) => Scope::Device,
    };

    // Skip candidates whose `scope_matched` exceeds the caller's
    // requested `scope`. e.g. a `scope = "self"` query must not see
    // peer entries even if their visibility allows it — the caller is
    // saying "only show me MY abilities".
    let allowed = match (scope, scope_matched) {
        (Scope::Selfish, Scope::Selfish) => true,
        (Scope::Selfish, _) => false,
        (Scope::Device, _) => true,
        // User and Public are federation-only; the local fan-in
        // path doesn't run for them but is_federated() guards make
        // sure we never reach this match arm under those scopes.
        // Keep the arms exhaustive for the compiler.
        (Scope::User, _) => true,
        (Scope::Public, _) => true,
    };
    if !allowed {
        return;
    }

    let Some(qualified_name) = local_agent_uras.ability_ura_for(peer_name, manifest.name()) else {
        return;
    };
    out.push(Candidate {
        qualified_name,
        owner: peer_name.to_string(),
        ability: manifest.name().to_string(),
        description: manifest.description().to_string(),
        input_schema: manifest.input_schema().clone(),
        visibility,
        scope_matched,
        score: 0.0,
        reason: String::new(),
        fulfilled_by: classify_fulfilled_by(manifest),
    });
}

/// Tag a manifest with how the call would actually run, so the LLM can
/// budget for latency. `shell` (sub-second deterministic) vs
/// `agent_chat` (LLM-driven, several seconds, non-deterministic) is
/// the load-bearing distinction; the field is `None` when the manifest
/// has no `[exec]` block and would route through the legacy chat
/// fallback (still effectively `agent_chat`, but the legacy path is
/// labelled separately for diagnostics).
fn classify_fulfilled_by(
    manifest: &crate::core::ability_spec::AbilityManifest,
) -> Option<&'static str> {
    use crate::core::ability_spec::AbilityExec;
    match manifest.exec() {
        Some(AbilityExec::Shell(_)) => Some("shell"),
        Some(AbilityExec::Http(_)) => Some("http"),
        Some(AbilityExec::Eal(_)) => Some("eal"),
        Some(AbilityExec::Mcp(_)) => Some("mcp"),
        None => Some("agent_chat_fallback"),
    }
}

/// Project a federated-directory entry list into discover candidates.
///
/// RFC-005 forbids synthesizing ability identities from presence
/// facts. A directory entry proves that an owner exists or is online;
/// it does not prove that `<owner>.forward_invoke` is a real public
/// Ability. Cross-hub routing still uses `federation.forward_invoke`
/// internally, but discover must only expose callable abilities when
/// a federated ability summary carries a canonical `ability_ura`.
///
/// The helper remains separate and tested because the call site still
/// receives directory entries from older federation surfaces. Keeping
/// the refusal local prevents future maintenance from reintroducing a
/// pseudo-Ability projection.
fn federated_directory_candidates(_entries: &[Value]) -> Vec<Candidate> {
    Vec::new()
}

/// Score every candidate against `query`. The scoring is intentionally
/// simple — a direct exact match on the ability name beats a substring
/// match, which beats a description keyword hit. The numbers carry no
/// absolute meaning; only the relative ordering matters, and the LLM
/// reads `reason` for the human story.
///
/// A future PR can replace this with a pluggable scorer (the SKILL.md
/// already teaches the LLM that `<owner>.semantic_discover` providers
/// may exist). The function is kept private and small so the swap is
/// a single line in `discover_handler`.
fn score_against_query(rows: &mut [Candidate], query: &str) {
    let q = query.to_lowercase();
    let q_terms: Vec<&str> = q.split_whitespace().collect();
    if q_terms.is_empty() {
        return;
    }

    for row in rows.iter_mut() {
        let name = row.ability.to_lowercase();
        let qualified = row.qualified_name.to_lowercase();
        let description = row.description.to_lowercase();

        let mut score: f64 = 0.0;
        let mut reasons: Vec<&str> = Vec::new();

        if name == q || qualified == q {
            score += 5.0;
            reasons.push("exact name match");
        }
        for term in &q_terms {
            if name.contains(term) {
                score += 3.0;
                reasons.push("term in ability name");
            }
            if description.contains(term) {
                score += 1.0;
                reasons.push("term in description");
            }
        }

        // Normalise so the JSON `score` stays in a documentable
        // range. Cap is heuristic — five name-hits would be a contrived
        // pathological query and clamping is more polite than letting
        // the field grow unbounded.
        row.score = (score / 10.0_f64).min(1.0);
        row.reason = dedup_reasons(reasons);
    }
}

fn dedup_reasons(rs: Vec<&str>) -> String {
    let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
    for r in rs {
        seen.insert(r, ());
    }
    seen.into_keys().collect::<Vec<&str>>().join(", ")
}

fn error_envelope(code: &str, message: &str, scope: Scope, query: Option<&str>) -> Value {
    json!({
        "candidates": [],
        "scope": scope.as_str(),
        "query": query,
        "error": {
            "code": code,
            "message": message,
            "retriable": false,
        }
    })
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn input_schema() -> Value {
    // Build the `enum` array from the single source of truth so the
    // schema the LLM sees can never drift from what `parse_scope`
    // actually accepts. The parity is pinned by the
    // `scope_enum_matches_parser_acceptance` test below.
    let scope_enum: Vec<Value> = ACCEPTED_SCOPE_LITERALS
        .iter()
        .map(|s| Value::String((*s).to_string()))
        .collect();
    json!({
        "type": "object",
        "properties": {
            "scope": {
                "type": "string",
                "enum": scope_enum,
                "default": "self",
                "description": "How far to search. \
                                `self` = my own abilities. \
                                `device` = abilities published by other agents \
                                on this device with visibility >= device. \
                                `user` = the calling tenant's federation view, \
                                across every device the user has joined to the \
                                same realm; calls federation.resolve against \
                                the realm hub. `public` = cross-tenant catalog \
                                — every agent the hub advertises, regardless \
                                of tenant; opt-in for explicit cross-user \
                                discovery. Federation tiers return federation_not_joined \
                                when the daemon hasn't run `device join`, or \
                                federation_unavailable when the hub call \
                                fails — both as typed envelopes, not Err."
            },
            "query": {
                "type": "string",
                "description": "Optional free-text query. Scored against \
                                ability name, qualified name, and description; \
                                empty/missing returns alphabetical."
            },
            "top_k": {
                "type": "integer",
                "minimum": 1,
                "default": 20,
                "description": "Cap on returned candidates after scoring."
            },
            "provider": {
                "type": "string",
                "description": "Optional discover-provider ability to delegate to \
                                (e.g. `userx.semantic_discover`). Provider must \
                                accept the same {scope, query, top_k} args and \
                                return the same {candidates, scope, query} \
                                envelope. Omit to use the builtin BM25-lite \
                                matcher."
            }
        },
        "additionalProperties": false
    })
}

pub fn manifest() -> AbilityManifest {
    AbilityManifest::new(ABILITY_VERB, description(), input_schema())
        .expect("discover ability manifest is static and validated by tests")
}

pub fn description() -> &'static str {
    "Walk the discovery ladder (self → device → user → public) and \
     return ranked candidates matching the optional query. The \
     federation tiers (`user` / `public`) dial the realm hub via \
     federation.resolve and project the receipt into the same Candidate \
     envelope as the local tiers; failures surface as typed envelopes \
     ({code: \"federation_not_joined\"} / \"federation_unavailable\") \
     so callers fall through gracefully. Use this BEFORE telling the \
     user you can't do something — another ability on the device or \
     in the realm may already cover it."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ability_spec::{AbilityManifest, AccessPolicy, Visibility};
    use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn obj_schema() -> Value {
        json!({"type": "object"})
    }

    #[test]
    fn register_publishes_discover_manifest_description() {
        let mut reg = AxonAbilityCatalog::new();
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );

        let manifest = reg
            .manifest_for("claude.discover")
            .expect("discover registration must publish its manifest");
        assert_eq!(manifest.description(), description());
        assert_eq!(manifest.input_schema(), &input_schema());
    }

    /// Build a temp workspace with `<root>/abilities/<verb>.ability.toml`
    /// for every (verb, manifest) pair, return the entry so the
    /// `manifests_for` reader can find it. The TempDir is returned so
    /// the caller keeps it alive for the duration of the test.
    ///
    /// Writes the minimal `agent.toml` `AgentDirectory::open` requires
    /// — without it the manifests reader hits "missing agent.toml" and
    /// silently falls back to an empty list, which would mask any
    /// scope/access regression behind a "no candidates" green.
    fn workspace_with_manifests(
        agent_name: &str,
        manifests: &[(&str, AbilityManifest)],
    ) -> (TempDir, PathBuf, AgentEntry) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let abilities_dir = root.join("abilities");
        std::fs::create_dir_all(&abilities_dir).unwrap();
        std::fs::write(
            root.join("agent.toml"),
            format!("name = \"{agent_name}\"\nruntime = \"claude-code\"\n"),
        )
        .unwrap();
        for (verb, m) in manifests {
            let toml = m.to_toml_string().unwrap();
            std::fs::write(
                abilities_dir.join(format!("{verb}.ability.toml")),
                toml.as_bytes(),
            )
            .unwrap();
        }
        let mut entry = AgentEntry::new(AgentType::ClaudeCode, None);
        entry.root_path = Some(root.clone());
        (dir, root, entry)
    }

    fn one_agent(name: &str, entry: AgentEntry) -> AgentRegistry {
        let mut reg = AgentRegistry::default();
        reg.agents.insert(name.to_string(), entry);
        reg
    }

    fn seed_local_agent_uras(
        entries: &[(&str, &str)],
    ) -> crate::facade::cli::test_support::HomeGuard {
        let guard = crate::facade::cli::test_support::HomeGuard::new();
        let mut local = crate::persistence::local_agents::LocalAgentsFile {
            host_device_agent_ura: "easynet:///r/acme/device/01DEV".into(),
            hosted_agents: Vec::new(),
        };
        for (agent_name, owner_ura) in entries {
            crate::persistence::local_agents::upsert_hosted_agent(
                &mut local, "llm", agent_name, owner_ura,
            );
        }
        crate::persistence::local_agents::save(&local).expect("seed local-agents.json");
        guard
    }

    fn ability_ura(owner_ura: &str, public_name: &str) -> String {
        crate::ura::owner_ability_ura(owner_ura, public_name).expect("test ability URA")
    }

    #[test]
    fn unknown_scope_is_rejected() {
        let mut reg = AxonAbilityCatalog::new();
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("claude.discover").unwrap();
        let err = h(json!({"scope": "galaxy"})).unwrap_err();
        assert!(format!("{err}").contains("scope"));
    }

    /// **Schema/parser parity pin**.
    ///
    /// The contract the LLM sees (`input_schema()["properties"]["scope"]
    /// ["enum"]`) must list exactly the spellings that `parse_scope`
    /// accepts. Any drift between the two is a P0 product bug — the
    /// LLM either thinks an accepted spelling is illegal (false
    /// rejection) or tries a spelling the runtime will reject (false
    /// confidence). The parity is sourced from
    /// [`ACCEPTED_SCOPE_LITERALS`]; this test pins both directions.
    #[test]
    fn scope_enum_matches_parser_acceptance() {
        // Direction 1: every literal the schema advertises must
        // round-trip through parse_scope without error. Catches a
        // schema that lists a spelling the parser does not handle.
        let schema = input_schema();
        let enum_arr = schema["properties"]["scope"]["enum"]
            .as_array()
            .expect("scope.enum must be an array");
        assert!(
            !enum_arr.is_empty(),
            "scope.enum must not be empty — the schema would forbid every value"
        );
        for v in enum_arr {
            let s = v.as_str().expect("scope.enum entries must be strings");
            parse_scope(&json!({"scope": s})).unwrap_or_else(|e| {
                panic!("schema advertises scope = {s:?} but parse_scope rejected it: {e}")
            });
        }

        // Direction 2: every spelling parse_scope accepts must
        // appear in the schema enum. The source-of-truth list is
        // ACCEPTED_SCOPE_LITERALS; comparing against it catches the
        // failure mode where someone adds a parse_scope match arm
        // but forgets to extend the literal list (so the LLM never
        // learns the new spelling exists).
        let enum_set: std::collections::BTreeSet<&str> =
            enum_arr.iter().map(|v| v.as_str().unwrap()).collect();
        let literal_set: std::collections::BTreeSet<&str> =
            ACCEPTED_SCOPE_LITERALS.iter().copied().collect();
        assert_eq!(
            enum_set, literal_set,
            "input_schema scope.enum must equal ACCEPTED_SCOPE_LITERALS — any \
             new scope literal must be added to both the parse_scope match \
             arms and the constant"
        );
    }

    /// Pin the user-visible description so a future schema edit that
    /// drops `user` / `public` from the description text (but keeps
    /// them in the enum) is caught — the LLM reads the description
    /// to pick a scope, not just the enum array.
    #[test]
    fn scope_description_names_user_and_public_tiers() {
        let schema = input_schema();
        let desc = schema["properties"]["scope"]["description"]
            .as_str()
            .expect("scope.description must be a string");
        for required_token in ["`self`", "`device`", "`user`", "`public`"] {
            assert!(
                desc.contains(required_token),
                "scope.description must mention {required_token}; got: {desc}"
            );
        }
        assert!(
            !desc.contains("`easynet`"),
            "scope.description must not advertise retired scope alias: {desc}"
        );
    }

    #[test]
    fn parse_scope_recognises_current_scope_literals() {
        let s = parse_scope(&json!({"scope": "user"})).unwrap();
        assert_eq!(s.as_str(), "user");
        assert!(s.is_federated());
        let s = parse_scope(&json!({"scope": "public"})).unwrap();
        assert_eq!(s.as_str(), "public");
        assert!(s.is_federated());
        // Self / device unchanged.
        assert!(!parse_scope(&json!({"scope": "self"}))
            .unwrap()
            .is_federated());
        assert!(!parse_scope(&json!({"scope": "device"}))
            .unwrap()
            .is_federated());
        // Unknown still rejected.
        assert!(parse_scope(&json!({"scope": "unknown-scope"})).is_err());
        assert!(
            parse_scope(&json!({"scope": "easynet"})).is_err(),
            "retired easynet scope alias must not parse"
        );
    }

    #[test]
    fn user_scope_falls_through_when_not_joined() {
        // The user tier is the canonical same-realm federation scope.
        // Under HomeGuard it should fail softly with a typed envelope.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("claude.discover").unwrap();
        let resp = h(json!({"scope": "user"})).unwrap();
        let code = resp["error"]["code"].as_str().unwrap_or("");
        assert!(
            code == "federation_not_joined" || code == "federation_unavailable",
            "expected federation_* typed code, got {code:?}; full resp: {resp:#?}"
        );
        assert_eq!(resp["scope"], "user");
    }

    #[test]
    fn public_scope_falls_through_when_not_joined() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("claude.discover").unwrap();
        let resp = h(json!({"scope": "public"})).unwrap();
        let code = resp["error"]["code"].as_str().unwrap_or("");
        assert!(
            code == "federation_not_joined" || code == "federation_unavailable",
            "expected federation_* typed code, got {code:?}; full resp: {resp:#?}"
        );
        assert_eq!(resp["scope"], "public");
    }

    #[test]
    fn user_scope_unjoined_returns_typed_error_envelope() {
        // No ~/.easynet/credentials.json under HomeGuard tmp HOME →
        // resolve_via_federation sees the unjoined state and returns
        // a typed envelope so the LLM falls through gracefully.
        // Pin the wire-level code so a SKILL.md grep stays stable.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("claude.discover").unwrap();
        let resp = h(json!({"scope": "user"})).unwrap();
        let code = resp["error"]["code"].as_str().unwrap_or("");
        assert!(
            code == "federation_not_joined" || code == "federation_unavailable",
            "expected federation_* typed code, got {code:?}; full resp: {resp:#?}"
        );
        assert_eq!(resp["candidates"].as_array().unwrap().len(), 0);
        assert_eq!(resp["scope"], "user");
    }

    #[test]
    fn self_scope_returns_only_calling_agents_abilities() {
        let _home = seed_local_agent_uras(&[
            ("claude", "easynet:///r/acme/agent/user-1.claude"),
            ("codex", "easynet:///r/acme/agent/user-1.codex"),
        ]);
        let weather = AbilityManifest::new("weather", "Fetch weather", obj_schema()).unwrap();
        let (_dir_a, _, entry_a) = workspace_with_manifests("claude", &[("weather", weather)]);
        let summary = AbilityManifest::new("summarize", "Summarise text", obj_schema()).unwrap();
        let (_dir_b, _, entry_b) = workspace_with_manifests("codex", &[("summarize", summary)]);

        let mut agents = AgentRegistry::default();
        agents.agents.insert("claude".into(), entry_a);
        agents.agents.insert("codex".into(), entry_b);
        let agents_clone = agents.clone();

        let mut reg = AxonAbilityCatalog::new();
        register_for_agent(
            &mut reg,
            "claude".into(),
            move || agents_clone.clone(),
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("claude.discover").unwrap();
        let resp = h(json!({"scope": "self"})).unwrap();
        let cands = resp["candidates"].as_array().unwrap();
        let names: Vec<&str> = cands
            .iter()
            .filter_map(|c| c["qualified_name"].as_str())
            .collect();
        // claude's seeded chat manifest plus weather; codex's
        // summarize must NOT appear.
        let claude_owner = crate::ura::agent_ura("acme", "user-1", "claude");
        assert!(names.iter().all(|n| {
            crate::ura::AbilitySelector::parse(n)
                .map(|selector| selector.owner_ura() == claude_owner)
                .unwrap_or(false)
        }));
        let claude_weather = crate::ura::ability_ura("acme", "user-1", "claude", "weather");
        let codex_summarize = crate::ura::ability_ura("acme", "user-1", "codex", "summarize");
        assert!(names.contains(&claude_weather.as_str()));
        assert!(!names.contains(&codex_summarize.as_str()));
    }

    #[test]
    fn device_scope_includes_peers_with_device_visibility() {
        let _home = seed_local_agent_uras(&[
            ("claude", "easynet:///r/acme/agent/user-1.claude"),
            ("codex", "easynet:///r/acme/agent/user-1.codex"),
        ]);
        let weather = AbilityManifest::new("weather", "Fetch weather", obj_schema())
            .unwrap()
            .with_access(AccessPolicy {
                visibility: Visibility::Device,
                ..Default::default()
            })
            .unwrap();
        let (_dir, _, entry) = workspace_with_manifests("claude", &[("weather", weather)]);

        let mut agents = AgentRegistry::default();
        agents.agents.insert("claude".into(), entry);
        // The caller (codex) has nothing of its own. We inject a
        // bare entry so the agent registry has both names.
        let mut codex_entry = AgentEntry::new(AgentType::Codex, None);
        codex_entry.root_path = Some(PathBuf::from("/nonexistent"));
        agents.agents.insert("codex".into(), codex_entry);
        let agents_clone = agents.clone();

        let mut reg = AxonAbilityCatalog::new();
        register_for_agent(
            &mut reg,
            "codex".into(),
            move || agents_clone.clone(),
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("codex.discover").unwrap();
        let resp = h(json!({"scope": "device", "query": "weather"})).unwrap();
        let cands = resp["candidates"].as_array().unwrap();
        let weather_ura = ability_ura("easynet:///r/acme/agent/user-1.claude", "weather");
        assert!(cands
            .iter()
            .any(|c| c["qualified_name"] == weather_ura.as_str()));
        // Each peer entry must report which tier it matched.
        let weather_entry = cands
            .iter()
            .find(|c| c["qualified_name"] == weather_ura.as_str())
            .unwrap();
        assert_eq!(weather_entry["scope_matched"], "device");
        assert_eq!(weather_entry["visibility"], "device");
    }

    #[test]
    fn device_scope_hides_peer_abilities_marked_self_visibility() {
        let _home = seed_local_agent_uras(&[
            ("claude", "easynet:///r/acme/agent/user-1.claude"),
            ("codex", "easynet:///r/acme/agent/user-1.codex"),
        ]);
        // An author who marked an ability as `[access] visibility = "self"`
        // is opting out of peer discovery. The discover handler must
        // honour that even when the caller asks for scope=device.
        let private = AbilityManifest::new("internal", "private helper", obj_schema())
            .unwrap()
            .with_access(AccessPolicy {
                visibility: Visibility::Selfish,
                ..Default::default()
            })
            .unwrap();
        let (_dir, _, entry) = workspace_with_manifests("claude", &[("internal", private)]);

        let mut agents = one_agent("claude", entry);
        let mut codex_entry = AgentEntry::new(AgentType::Codex, None);
        codex_entry.root_path = Some(PathBuf::from("/nonexistent"));
        agents.agents.insert("codex".into(), codex_entry);
        let agents_clone = agents.clone();

        let mut reg = AxonAbilityCatalog::new();
        register_for_agent(
            &mut reg,
            "codex".into(),
            move || agents_clone.clone(),
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("codex.discover").unwrap();
        let resp = h(json!({"scope": "device"})).unwrap();
        let cands = resp["candidates"].as_array().unwrap();
        assert!(
            cands
                .iter()
                .all(|c| c["qualified_name"] != "easynet:///r/acme/ability/user-1.claude.internal"),
            "self-visibility ability leaked to peer: {cands:#?}"
        );
    }

    #[test]
    fn query_scoring_orders_exact_name_match_first() {
        let _home = seed_local_agent_uras(&[("claude", "easynet:///r/acme/agent/user-1.claude")]);
        let weather =
            AbilityManifest::new("weather", "Fetches weather data via wttr.in", obj_schema())
                .unwrap();
        let news =
            AbilityManifest::new("news", "Daily weather and news digest", obj_schema()).unwrap();
        let (_dir, _, entry) =
            workspace_with_manifests("claude", &[("weather", weather), ("news", news)]);

        let agents = one_agent("claude", entry);
        let agents_clone = agents.clone();

        let mut reg = AxonAbilityCatalog::new();
        register_for_agent(
            &mut reg,
            "claude".into(),
            move || agents_clone.clone(),
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("claude.discover").unwrap();
        let resp = h(json!({"scope": "self", "query": "weather"})).unwrap();
        let cands = resp["candidates"].as_array().unwrap();
        assert_eq!(cands[0]["ability"], "weather");
    }

    #[test]
    fn top_k_zero_is_rejected() {
        let mut reg = AxonAbilityCatalog::new();
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("claude.discover").unwrap();
        let err = h(json!({"top_k": 0})).unwrap_err();
        assert!(format!("{err}").contains("top_k"));
    }

    #[test]
    fn provider_arg_delegates_to_named_handler() {
        // The discover handler routes to the named provider via the
        // dispatch registry. Pin both halves: (a) the provider IS
        // called, (b) the `provider` field is stripped before the
        // forwarded args reach it (so the provider doesn't see a
        // recursion-trigger).
        use std::sync::Mutex;
        let captured: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let captured_for_provider = Arc::clone(&captured);
        let provider_handler: crate::runtime::ability_dispatch::LocalRpcHandler =
            Arc::new(move |args: Value| {
                *captured_for_provider.lock().unwrap() = Some(args);
                Ok(json!({
                    "candidates": [],
                    "scope": "self",
                    "query": null,
                    "provider": "userx.semantic_discover (mock)"
                }))
            });

        // Build a registry with the provider handler + the per-agent
        // discover. Wire the OnceLock to the same registry so the
        // builtin discover can resolve the provider.
        let mut reg = AxonAbilityCatalog::new();
        reg.register_rpc("userx.semantic_discover", provider_handler);
        let handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>> =
            Arc::new(std::sync::OnceLock::new());
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::clone(&handle),
        );
        let arc_reg = Arc::new(reg);
        handle.set(Arc::clone(&arc_reg)).expect("handle set once");

        let h = arc_reg.resolve_rpc("claude.discover").unwrap();
        let resp = h(json!({
            "scope": "self",
            "query": "weather",
            "provider": "userx.semantic_discover"
        }))
        .unwrap();
        // Provider's response is returned verbatim.
        assert_eq!(resp["provider"], "userx.semantic_discover (mock)");
        // Provider received args without `provider` field.
        let captured_args = captured.lock().unwrap().clone().unwrap();
        assert!(captured_args.get("provider").is_none());
        assert_eq!(captured_args["query"], "weather");
    }

    #[test]
    fn provider_without_dot_is_rejected() {
        let mut reg = AxonAbilityCatalog::new();
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("claude.discover").unwrap();
        let err = h(json!({"provider": "bogus"})).unwrap_err();
        assert!(format!("{err}").contains("<owner>.<verb>"));
    }

    #[test]
    fn provider_not_registered_returns_typed_error() {
        let mut reg = AxonAbilityCatalog::new();
        let handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>> =
            Arc::new(std::sync::OnceLock::new());
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::clone(&handle),
        );
        let arc_reg = Arc::new(reg);
        handle.set(Arc::clone(&arc_reg)).expect("set");
        let h = arc_reg.resolve_rpc("claude.discover").unwrap();
        let err = h(json!({"provider": "ghost.discover"})).unwrap_err();
        assert!(format!("{err}").contains("not registered"));
    }

    // -----------------------------------------------------------------
    // federated_directory_candidates — RFC-005 negative projection
    // pins. Directory entries are presence facts, not Ability
    // descriptors, so this helper must never synthesize candidates.
    // -----------------------------------------------------------------

    fn active_entry(agent_ura: &str, node_id: &str, display: Option<&str>) -> Value {
        let mut v = json!({
            "agent_ura": agent_ura,
            "node_id":   node_id,
            "status":    "active",
            "origin_realm": "peer-realm",
        });
        if let Some(d) = display {
            v["display_name"] = Value::String(d.to_string());
        }
        v
    }

    fn federated_summary(
        ability_ura: &str,
    ) -> crate::runtime::owner_projection::AbilityProjectionSummary {
        crate::runtime::owner_projection::AbilityProjectionSummary {
            ability_ura: ability_ura.to_string(),
            owner_ura: "easynet:///r/acme/agent/alice.bot".to_string(),
            namespace: String::new(),
            local_name: "chat".to_string(),
            descriptor_revision: "sha256:test".to_string(),
            schema_ref: None,
            schema_hash: Some("sha256:schema".to_string()),
            policy_ref: "visibility:PUBLIC".to_string(),
            route_summary_ref: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn federated_summary_candidate_requires_ability_ura() {
        let summary = federated_summary("");
        let candidate = candidate_from_federated_summary(
            "easynet:///r/acme/agent/alice.bot",
            &summary,
            "chat".to_string(),
            Scope::Public,
        );

        assert!(
            candidate.is_none(),
            "federated discover must not synthesize owner.public_name identities"
        );
    }

    #[test]
    fn federated_summary_candidate_uses_ability_ura_as_qualified_name() {
        let ability_ura = "easynet:///r/acme/ability/alice.bot.chat";
        let summary = federated_summary(ability_ura);
        let candidate = candidate_from_federated_summary(
            "easynet:///r/acme/agent/alice.bot",
            &summary,
            "chat".to_string(),
            Scope::Public,
        )
        .expect("complete federated summary should project");

        assert_eq!(candidate.qualified_name, ability_ura);
        assert_eq!(candidate.owner, "easynet:///r/acme/agent/alice.bot");
        assert_eq!(candidate.ability, "chat");
        assert_eq!(candidate.visibility, Visibility::Public);
        assert_eq!(candidate.fulfilled_by, Some("federation"));
    }

    #[test]
    fn federated_directory_helper_does_not_project_active_entry() {
        let entries = vec![active_entry(
            "easynet:///r/peer-realm/device/d1",
            "d1",
            Some("Peer Workstation"),
        )];
        let out = federated_directory_candidates(&entries);

        assert!(
            out.is_empty(),
            "directory presence must not synthesize a forward_invoke Ability candidate"
        );
    }

    #[test]
    fn federated_directory_helper_does_not_project_malformed_entry() {
        let mut e = active_entry("placeholder", "d3", None);
        e.as_object_mut().unwrap().remove("agent_ura");
        let out = federated_directory_candidates(&[e]);
        assert!(
            out.is_empty(),
            "malformed directory rows also must not be repaired into pseudo abilities"
        );
    }

    #[test]
    fn federated_directory_helper_does_not_project_multiple_entries() {
        let entries = vec![
            active_entry("easynet:///r/p/device/a", "a", Some("A")),
            active_entry("easynet:///r/p/device/b", "b", Some("B")),
            active_entry("easynet:///r/p/device/c", "c", Some("C")),
        ];
        let out = federated_directory_candidates(&entries);
        assert!(
            out.is_empty(),
            "presence-only directory snapshots must not expand into Ability candidates"
        );
    }
}

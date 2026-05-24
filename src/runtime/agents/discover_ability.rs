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
//   Tier 3  scope = "easynet"  — abilities published to the EasyNet
//                                federation. Calls `federation.resolve`
//                                against the realm's hub (Hub-profile
//                                ability, RFC-001 §A14) and projects
//                                each `ResolvedAgent` into the same
//                                `Candidate` envelope as the local
//                                tiers. Failures (no realm joined /
//                                hub call dropped) surface as typed
//                                `federation_not_joined` /
//                                `federation_unavailable` envelopes
//                                so the LLM falls through gracefully.
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
// `easynet.discover` and `meta.list_abilities` continue to exist as
// thin compat aliases (see `meta_ability::register`); they return the
// flat `{abilities: [...]}` catalogue without scope filtering. The
// `delegate` skill teaches the new owner-namespaced form so a fresh
// install lands on the canonical name.
//
// Output shape
// ------------
//   { "candidates": [
//       {
//         "qualified_name": "claude.weather",
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
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

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
    reg: &mut LocalAbilityRegistry,
    agent_name: String,
    agent_registry_provider: F,
    dispatch_registry_handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
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
/// register_for_agent (which requires `&mut LocalAbilityRegistry`).
pub fn dispatch(
    self_agent: &str,
    agent_registry_provider: &Arc<dyn Fn() -> AgentRegistry + Send + Sync>,
    dispatch_registry_handle: &Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
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
    let mut rows: Vec<Candidate> = Vec::new();
    for (peer_name, peer_entry) in agents.agents.iter() {
        let manifests = crate::runtime::abilities::manifests_for(peer_name, peer_entry);
        for m in manifests {
            push_candidate(&mut rows, self_agent, peer_name, peer_entry, &m, scope);
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
    dispatch_registry_handle: &Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
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
    let handler = registry.resolve_rpc(provider_name).ok_or_else(|| {
        anyhow::anyhow!(
            "discover: provider {provider_name:?} is not registered. Pick from \
             the abilities your `<self>.discover()` lists with names ending \
             in `.discover` (or `.semantic_discover` etc), or omit the \
             `provider` argument to use the builtin BM25 matcher."
        )
    })?;
    handler(args)
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

/// Tier 3 — `scope: "easynet"` resolution. Dials the realm's hub via
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
/// outweighs the perf win. If discover-on-easynet ever becomes
/// hot, swap to a stashed `BridgePool` here without touching
/// callers.
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
                     a realm before scope=\"easynet\""
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
                 with a hub before scope=\"easynet\"",
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
    let realm = creds.tenant_id.clone();
    let tenant = creds.tenant_id.as_str();
    if realm.is_empty() {
        return Ok(error_envelope(
            "federation_not_joined",
            "credentials.json carries an empty tenant; rejoin via \
             `easynet device join` before scope=\"easynet\"",
            scope,
            query,
        ));
    }

    // Pin the caller URI to the daemon's own device-profile Agent
    // URA. Without this the bridge synthesises a hub-literal caller
    // (`agents/easynet:prv:hub:<realm>`) and the hub's membership
    // gate rejects with AXON_MEMBERSHIP_REQUIRED — same caller-URI
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
        let owner = agent.uri.clone();
        for desc in agent.abilities {
            let bare_ability = desc
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if bare_ability.is_empty() {
                continue;
            }
            let description = desc
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let input_schema = desc.get("input_schema").cloned().unwrap_or(Value::Null);
            let qualified_name = if bare_ability.contains('.') {
                // Hub-side descriptors sometimes carry the
                // owner-prefixed name verbatim; preserve it.
                bare_ability.clone()
            } else {
                format!("{owner}.{bare_ability}")
            };
            rows.push(Candidate {
                qualified_name,
                owner: owner.clone(),
                ability: bare_ability,
                description,
                input_schema,
                visibility: Visibility::Public,
                scope_matched: scope,
                score: 0.0,
                reason: String::new(),
                fulfilled_by: Some("federation"),
            });
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
    /// other devices owned by the same user. The default federation
    /// query — what "scope: easynet" actually means in practice.
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

fn parse_scope(args: &Value) -> anyhow::Result<Scope> {
    let raw = args.get("scope").and_then(Value::as_str).unwrap_or("self");
    match raw {
        "self" => Ok(Scope::Selfish),
        "device" => Ok(Scope::Device),
        // RFC-002 §5 update: `easynet` is the historical name for
        // the federation tier. We retain it as an alias for `user`
        // — the scope users actually want when they ask "what's on
        // my account" — so existing callers keep working. New
        // callers should prefer `user` for self-tenant queries and
        // `public` for cross-tenant.
        "easynet" | "user" => Ok(Scope::User),
        "public" => Ok(Scope::Public),
        other => anyhow::bail!(
            "discover: scope = {other:?} is not one of \"self\", \"device\", \
             \"user\" / \"easynet\", or \"public\""
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

/// Decide whether one peer ability satisfies the requested scope and
/// the ability's own `[access]` policy, then push a row.
fn push_candidate(
    out: &mut Vec<Candidate>,
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

    let qualified_name = format!("{peer_name}.{}", manifest.name());
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
    json!({
        "type": "object",
        "properties": {
            "scope": {
                "type": "string",
                "enum": ["self", "device", "easynet"],
                "default": "self",
                "description": "How far to search. self = my own abilities. \
                                device = abilities published by other agents \
                                on this device with visibility >= device. \
                                easynet = published to the EasyNet federation \
                                (calls federation.resolve against the realm's \
                                hub; returns federation_not_joined when the \
                                daemon hasn't run `device join`, or \
                                federation_unavailable when the hub call \
                                fails — both as typed envelopes, not Err)."
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
    "Walk the discovery ladder (self → device → easynet) and return \
     ranked candidates matching the optional query. Tier 3 \
     (scope=\"easynet\") dials the realm hub via federation.resolve \
     and projects the receipt into the same Candidate envelope as \
     the local tiers; failures surface as typed envelopes \
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
        let mut reg = LocalAbilityRegistry::new();
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

    #[test]
    fn unknown_scope_is_rejected() {
        let mut reg = LocalAbilityRegistry::new();
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

    #[test]
    fn parse_scope_recognises_user_and_public_aliases() {
        // RFC-002 §5: "user" is the new canonical name for what
        // "easynet" used to mean. "public" is opt-in cross-tenant.
        // Both must be accepted; "easynet" stays as a back-compat
        // alias mapping to User.
        let s = parse_scope(&json!({"scope": "user"})).unwrap();
        assert_eq!(s.as_str(), "user");
        assert!(s.is_federated());
        let s = parse_scope(&json!({"scope": "easynet"})).unwrap();
        assert_eq!(
            s.as_str(),
            "user",
            "easynet alias must canonicalise to user"
        );
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
    }

    #[test]
    fn user_scope_falls_through_when_not_joined() {
        // Same shape as easynet_scope_unjoined_returns_typed_error_envelope
        // but exercising the new "user" name explicitly so a future
        // refactor that drops the alias still has direct coverage.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = LocalAbilityRegistry::new();
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
        let mut reg = LocalAbilityRegistry::new();
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
    fn easynet_scope_unjoined_returns_typed_error_envelope() {
        // No ~/.easynet/credentials.json under HomeGuard tmp HOME →
        // resolve_via_federation sees the unjoined state and returns
        // a typed envelope so the LLM falls through gracefully.
        // Pin the wire-level code so a SKILL.md grep stays stable.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = LocalAbilityRegistry::new();
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("claude.discover").unwrap();
        let resp = h(json!({"scope": "easynet"})).unwrap();
        let code = resp["error"]["code"].as_str().unwrap_or("");
        assert!(
            code == "federation_not_joined" || code == "federation_unavailable",
            "expected federation_* typed code, got {code:?}; full resp: {resp:#?}"
        );
        assert_eq!(resp["candidates"].as_array().unwrap().len(), 0);
        // RFC-002 §5 update: scope: "easynet" is the alias; it
        // canonicalises to "user" in the echo so callers can grep
        // the new name. Both scope values reach the same federation
        // path; only the echoed string differs.
        assert_eq!(resp["scope"], "user");
    }

    #[test]
    fn self_scope_returns_only_calling_agents_abilities() {
        let weather = AbilityManifest::new("weather", "Fetch weather", obj_schema()).unwrap();
        let (_dir_a, _, entry_a) = workspace_with_manifests("claude", &[("weather", weather)]);
        let summary = AbilityManifest::new("summarize", "Summarise text", obj_schema()).unwrap();
        let (_dir_b, _, entry_b) = workspace_with_manifests("codex", &[("summarize", summary)]);

        let mut agents = AgentRegistry::default();
        agents.agents.insert("claude".into(), entry_a);
        agents.agents.insert("codex".into(), entry_b);
        let agents_clone = agents.clone();

        let mut reg = LocalAbilityRegistry::new();
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
        assert!(names.iter().all(|n| n.starts_with("claude.")));
        assert!(names.iter().any(|n| *n == "claude.weather"));
        assert!(!names.iter().any(|n| *n == "codex.summarize"));
    }

    #[test]
    fn device_scope_includes_peers_with_device_visibility() {
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

        let mut reg = LocalAbilityRegistry::new();
        register_for_agent(
            &mut reg,
            "codex".into(),
            move || agents_clone.clone(),
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("codex.discover").unwrap();
        let resp = h(json!({"scope": "device", "query": "weather"})).unwrap();
        let cands = resp["candidates"].as_array().unwrap();
        assert!(cands
            .iter()
            .any(|c| c["qualified_name"] == "claude.weather"));
        // Each peer entry must report which tier it matched.
        let weather_entry = cands
            .iter()
            .find(|c| c["qualified_name"] == "claude.weather")
            .unwrap();
        assert_eq!(weather_entry["scope_matched"], "device");
        assert_eq!(weather_entry["visibility"], "device");
    }

    #[test]
    fn device_scope_hides_peer_abilities_marked_self_visibility() {
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

        let mut reg = LocalAbilityRegistry::new();
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
                .all(|c| c["qualified_name"] != "claude.internal"),
            "self-visibility ability leaked to peer: {cands:#?}"
        );
    }

    #[test]
    fn query_scoring_orders_exact_name_match_first() {
        let weather =
            AbilityManifest::new("weather", "Fetches weather data via wttr.in", obj_schema())
                .unwrap();
        let news =
            AbilityManifest::new("news", "Daily weather and news digest", obj_schema()).unwrap();
        let (_dir, _, entry) =
            workspace_with_manifests("claude", &[("weather", weather), ("news", news)]);

        let agents = one_agent("claude", entry);
        let agents_clone = agents.clone();

        let mut reg = LocalAbilityRegistry::new();
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
        let mut reg = LocalAbilityRegistry::new();
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
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc("userx.semantic_discover", provider_handler);
        let handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>> =
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
        let mut reg = LocalAbilityRegistry::new();
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
        let mut reg = LocalAbilityRegistry::new();
        let handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>> =
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
}

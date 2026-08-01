// EasyNet CLI — <agent>.discover ability handler
// =================================================================
//
// File: src/daemon/ability/builtins/agents/discover.rs
//
// Per-agent ability discovery walking the three-tier ladder taught
// by the `delegate` SKILL.md:
//
//   Tier 1  scope = "self"     — abilities advertised by the calling agent
//   Tier 2  scope = "device"   — abilities advertised by other agents
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
// Why "<agent>.discover" and not the legacy "easynet.discover"
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
//         "call_modes":     ["rpc"],
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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::ability::manifest::{AbilityManifest, ManifestAccessScope};
use crate::daemon::invocation::routing::target::{
    CallMode, InvocationTarget, PublicInvocationTargetIssuer,
};
use crate::daemon::persistence::agent_aggregate::AgentAggregateSnapshot;
use crate::daemon::persistence::agent_registry::AgentRegistry;

/// Verb portion of the per-agent discover ability. Combined with the
/// owning agent's name to form the wire-level `<agent>.discover`.
pub const ABILITY_VERB: &str = crate::daemon::ability::names::agents::DISCOVER_VERB;
/// Daemon-owned `agent.discover` aggregate used by top-level
/// `easynet discover` when the caller does not select a self agent.
/// The owner is already `OwnerKind::Device`, so the dispatch key must
/// stay owner-local instead of duplicating a `device.` prefix.
pub const DEVICE_DISCOVER_ABILITY: &str = crate::daemon::ability::names::agents::DISCOVER;

/// Shared resolver object for federation-backed discover tiers.
///
/// The discover handler owns the ladder/ranking projection; it does
/// not own how a daemon reaches the realm directory. Production daemon
/// boot injects a local read-model resolver so `<agent>.discover`
/// does not re-enter the daemon over its own UDS. Bridge-only harnesses
/// use [`DetachedDiscoverFederationResolver`] to make the missing dependency
/// Axon runtime path.
pub type SharedDiscoverFederationResolver = Arc<dyn DiscoverFederationResolver>;

/// Error classes for the federation tier of `<agent>.discover`.
#[derive(Debug, thiserror::Error)]
pub enum DiscoverFederationResolveError {
    /// The daemon has no usable realm-directory resolver for this
    /// process. Caller surfaces this as a typed degradation envelope,
    /// not as a failed discover command.
    #[error("{0}")]
    NotJoined(String),
    /// A configured resolver exists but the lookup failed.
    #[error("{0}")]
    Unavailable(String),
}

/// Dependency boundary between the discover ladder and the realm
/// directory implementation.
pub trait DiscoverFederationResolver: Send + Sync {
    /// Resolve active agents for the supplied tenant/realm scope.
    fn resolve_agents(
        &self,
        tenant: &str,
        realm: &str,
        caller_ura: String,
        tenant_filter: Option<String>,
    ) -> Result<
        Vec<crate::daemon::federation::client::ability_contract::ResolvedAgent>,
        DiscoverFederationResolveError,
    >;
}

/// Explicit unresolved directory dependency used by deterministic catalogue
/// tests. Production daemon assembly replaces it with the late-bound local
/// directory resolver before any public-tier discovery can succeed.
#[derive(Debug, Default)]
pub struct DetachedDiscoverFederationResolver;

impl DiscoverFederationResolver for DetachedDiscoverFederationResolver {
    fn resolve_agents(
        &self,
        _tenant: &str,
        _realm: &str,
        _caller_ura: String,
        _tenant_filter: Option<String>,
    ) -> Result<
        Vec<crate::daemon::federation::client::ability_contract::ResolvedAgent>,
        DiscoverFederationResolveError,
    > {
        Err(DiscoverFederationResolveError::NotJoined(
            "daemon directory resolver is not attached".to_string(),
        ))
    }
}

/// Late-bound resolver cell for boot paths where the agent registry is
/// built before the daemon Invocation transport constructs its
/// directory stores.
#[derive(Default)]
pub struct DeferredDiscoverFederationResolver {
    resolver: OnceLock<SharedDiscoverFederationResolver>,
}

impl DeferredDiscoverFederationResolver {
    /// Create an empty late-bound resolver.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach the concrete resolver exactly once.
    pub fn set(
        &self,
        resolver: SharedDiscoverFederationResolver,
    ) -> Result<(), SharedDiscoverFederationResolver> {
        self.resolver.set(resolver)
    }
}

impl DiscoverFederationResolver for DeferredDiscoverFederationResolver {
    fn resolve_agents(
        &self,
        tenant: &str,
        realm: &str,
        caller_ura: String,
        tenant_filter: Option<String>,
    ) -> Result<
        Vec<crate::daemon::federation::client::ability_contract::ResolvedAgent>,
        DiscoverFederationResolveError,
    > {
        let Some(resolver) = self.resolver.get() else {
            return Err(DiscoverFederationResolveError::NotJoined(
                "daemon directory resolver is not attached yet; retry after Invocation transport boot"
                    .to_string(),
            ));
        };
        resolver.resolve_agents(tenant, realm, caller_ura, tenant_filter)
    }
}

/// Daemon-local federation resolver backed by the same read models
/// that `federation.advertise_agent` and `federation.advertise_abilities`
/// update.
#[cfg(feature = "axon-pb")]
pub struct LocalDirectoryDiscoverFederationResolver {
    presence: Arc<crate::daemon::invocation::bidi::state::presence::PresenceRegistry>,
    advertised_agents:
        Arc<crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore>,
    ability_catalog:
        Arc<crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore>,
    local_ability_catalog: Arc<OnceLock<Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog>>>,
}

#[cfg(feature = "axon-pb")]
impl LocalDirectoryDiscoverFederationResolver {
    /// Construct a resolver over daemon-owned directory stores.
    #[must_use]
    pub fn new(
        presence: Arc<crate::daemon::invocation::bidi::state::presence::PresenceRegistry>,
        advertised_agents: Arc<
            crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore,
        >,
        ability_catalog: Arc<
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore,
        >,
        local_ability_catalog: Arc<
            OnceLock<Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog>>,
        >,
    ) -> Self {
        Self {
            presence,
            advertised_agents,
            ability_catalog,
            local_ability_catalog,
        }
    }
}

#[cfg(feature = "axon-pb")]
impl DiscoverFederationResolver for LocalDirectoryDiscoverFederationResolver {
    fn resolve_agents(
        &self,
        tenant: &str,
        realm: &str,
        _caller_ura: String,
        tenant_filter: Option<String>,
    ) -> Result<
        Vec<crate::daemon::federation::client::ability_contract::ResolvedAgent>,
        DiscoverFederationResolveError,
    > {
        let ura_prefix = local_resolve_prefix(tenant, realm, tenant_filter.as_deref())?;
        let request =
            crate::daemon::invocation::dispatch::federation_wrappers::ResolveRequest::with_filter(
                ura_prefix, true,
            );
        let response = crate::daemon::invocation::dispatch::federation_wrappers::handle_resolve(
            &request,
            &self.presence,
            Some(self.advertised_agents.as_ref()),
            self.ability_catalog.as_ref(),
            self.local_ability_catalog.get().map(Arc::as_ref),
        )
        .map_err(|error| {
            DiscoverFederationResolveError::Unavailable(format!(
                "federation.resolve ability projection: {error}"
            ))
        })?;
        let value = serde_json::to_value(response)
            .map_err(|e| DiscoverFederationResolveError::Unavailable(e.to_string()))?;
        let receipt: crate::daemon::federation::client::ability_contract::ResolveReceipt =
            crate::daemon::federation::client::ability_contract::parse_receipt_value(&value)
                .map_err(|e| {
                    DiscoverFederationResolveError::Unavailable(format!(
                        "parse local federation.resolve response: {e}"
                    ))
                })?;
        Ok(receipt.agents)
    }
}

#[cfg(feature = "axon-pb")]
fn local_resolve_prefix(
    tenant: &str,
    realm: &str,
    tenant_filter: Option<&str>,
) -> Result<Option<String>, DiscoverFederationResolveError> {
    let filter = tenant_filter
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let realm_segment = match filter {
        Some("*") => return Ok(None),
        Some(explicit_tenant) => explicit_tenant,
        None => {
            if !realm.trim().is_empty() {
                realm.trim()
            } else {
                tenant.trim()
            }
        }
    };
    if realm_segment.is_empty() {
        return Err(DiscoverFederationResolveError::Unavailable(
            "local federation.resolve cannot derive a tenant/realm prefix".to_string(),
        ));
    }
    crate::core::ura::realm_prefix_ura(realm_segment)
        .map(Some)
        .map_err(|err| DiscoverFederationResolveError::Unavailable(err.to_string()))
}

/// Register `<agent_name>.discover` on the registry. Each agent gets
/// its own copy of this self-bundle ability — the handler closes over
/// the agent's name so calls from MCP / EAL never need to pass an
/// explicit caller identity.
///
/// `agent_registry_provider` is invoked at handler-call time so that
/// hot-added or hot-removed peer agents are reflected on the next
/// discover call without re-registration.
pub(crate) type AgentDirectoryProvider =
    Arc<dyn Fn() -> anyhow::Result<AgentAggregateSnapshot> + Send + Sync>;

/// Adapter for infallible in-memory fixtures and fallible durable providers.
/// Production providers return `anyhow::Result`; the infallible implementation
/// keeps deterministic unit fixtures concise without introducing a runtime
/// fallback.
pub(crate) trait IntoAgentDirectoryLoadResult {
    fn into_agent_directory_load_result(self) -> anyhow::Result<AgentAggregateSnapshot>;
}

#[cfg(test)]
impl IntoAgentDirectoryLoadResult for AgentRegistry {
    fn into_agent_directory_load_result(self) -> anyhow::Result<AgentAggregateSnapshot> {
        Ok(AgentAggregateSnapshot::new(
            self,
            crate::daemon::persistence::local_agents::load_for_fresh_host_projection()?,
        ))
    }
}

#[cfg(test)]
impl IntoAgentDirectoryLoadResult for anyhow::Result<AgentRegistry> {
    fn into_agent_directory_load_result(self) -> anyhow::Result<AgentAggregateSnapshot> {
        self?.into_agent_directory_load_result()
    }
}

impl IntoAgentDirectoryLoadResult for AgentAggregateSnapshot {
    fn into_agent_directory_load_result(self) -> anyhow::Result<AgentAggregateSnapshot> {
        Ok(self)
    }
}

impl IntoAgentDirectoryLoadResult for anyhow::Result<AgentAggregateSnapshot> {
    fn into_agent_directory_load_result(self) -> anyhow::Result<AgentAggregateSnapshot> {
        self
    }
}

#[cfg(test)]
pub(crate) fn register_for_agent<F, R>(
    reg: &mut AxonAbilityCatalog,
    agent_name: String,
    agent_registry_provider: F,
    dispatch_registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) where
    F: Fn() -> R + Send + Sync + 'static,
    R: IntoAgentDirectoryLoadResult,
{
    register_for_agent_with_resolver(
        reg,
        agent_name,
        agent_registry_provider,
        dispatch_registry_handle,
        Arc::new(DetachedDiscoverFederationResolver),
    );
}

/// Same as [`register_for_agent`] with an explicit federation
/// resolver dependency.
#[cfg(test)]
pub(crate) fn register_for_agent_with_resolver<F, R>(
    reg: &mut AxonAbilityCatalog,
    agent_name: String,
    agent_registry_provider: F,
    dispatch_registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    federation_resolver: SharedDiscoverFederationResolver,
) where
    F: Fn() -> R + Send + Sync + 'static,
    R: IntoAgentDirectoryLoadResult,
{
    use crate::daemon::ability::dispatch::OwnerKind;
    let provider: AgentDirectoryProvider =
        Arc::new(move || agent_registry_provider().into_agent_directory_load_result());
    let qualified = format!("{agent_name}.{ABILITY_VERB}");
    let agent = agent_name.clone();
    reg.register_rpc_with_spec_and_action(
        &qualified,
        OwnerKind::Agent(agent_name),
        crate::daemon::ability::descriptors::AdmissionAction::Read,
        manifest(),
        Arc::new(move |args: Value| {
            dispatch(
                &agent,
                &provider,
                &dispatch_registry_handle,
                federation_resolver.as_ref(),
                args,
            )
        }),
    );
}

/// Register the daemon-owned `agent.discover` aggregate entry.
///
/// This is intentionally a thin owner wrapper over [`dispatch`], not a
/// second discovery implementation. Passing an empty `self_agent` means local
/// fan-in has no self tier: `visibility = self` helpers stay hidden and the
/// top-level CLI sees the device aggregate without choosing an arbitrary
/// first agent as caller identity.
pub(crate) fn register_device_aggregate_with_resolver<F, R>(
    reg: &mut AxonAbilityCatalog,
    agent_registry_provider: F,
    dispatch_registry_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    federation_resolver: SharedDiscoverFederationResolver,
) where
    F: Fn() -> R + Send + Sync + 'static,
    R: IntoAgentDirectoryLoadResult,
{
    use crate::daemon::ability::dispatch::OwnerKind;
    let provider: AgentDirectoryProvider =
        Arc::new(move || agent_registry_provider().into_agent_directory_load_result());
    reg.register_rpc_with_spec_and_action(
        DEVICE_DISCOVER_ABILITY,
        OwnerKind::Device,
        crate::daemon::ability::descriptors::AdmissionAction::Read,
        manifest(),
        Arc::new(move |args: Value| {
            dispatch(
                "",
                &provider,
                &dispatch_registry_handle,
                federation_resolver.as_ref(),
                args,
            )
        }),
    );
}

/// Public per-call entry point. Validates `scope`, applies `query`
/// filtering, returns the standardised `{candidates, scope, query}`
/// envelope.
///
/// Provider routing
/// ----------------
/// When the call passes `provider = "<agent>.discover"`, dispatch
/// hands off to that hosted agent's discover ability instead of running
/// the builtin BM25-lite scorer. The provider must satisfy the same
/// input/output contract (accepts `{scope, query, top_k, source_window}`,
/// returns `{candidates, scope, query}`). Builtin is the default.
///
/// Exposed so HotAgentRegistrar can build the same handler for a
/// hot-added agent and materialise it in LocalRuntime without
/// re-running this module's `register_for_agent` (which requires
/// `&mut AxonAbilityCatalog`).
pub(crate) fn dispatch(
    self_agent: &str,
    agent_registry_provider: &AgentDirectoryProvider,
    dispatch_registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    federation_resolver: &dyn DiscoverFederationResolver,
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
                agent_registry_provider,
                dispatch_registry_handle,
                strip_provider_field(&args),
            );
        }
    }

    let scope = parse_scope(&args)?;
    let query = parse_query(&args);
    let source_window = parse_source_window(&args)?;

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
        return resolve_via_federation(federation_resolver, scope, query.as_deref(), source_window);
    }

    let catalog = dispatch_registry_handle.get().ok_or_else(|| {
        anyhow::anyhow!(
            "discover: canonical local Ability catalog is not attached; retry after daemon boot"
        )
    })?;
    let directory = agent_registry_provider()
        .map_err(|error| anyhow::anyhow!("discover: load Agent directory: {error:#}"))?;
    let mut rows = local_catalog_candidates(catalog, &directory, self_agent, scope)?;

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
    let source = source_window.apply(&mut rows);

    let candidates: Vec<Value> = rows.iter().map(Candidate::to_json).collect();
    Ok(json!({
        "candidates": candidates,
        "scope": scope.as_str(),
        "query": query,
        "source": {
            "available": source.available,
            "returned": rows.len(),
            "limit": source.limit,
            "truncated": source.truncated,
        },
    }))
}

/// Forward a discover call to one hosted agent's discover provider.
/// The provider is named in `<agent>.discover` form and must map to an
/// agent registered on this daemon. We strip the `provider` field so the
/// downstream handler sees the args it declared in its own input_schema, not a
/// recursion-trigger.
fn delegate_to_provider(
    provider_name: &str,
    agent_registry_provider: &AgentDirectoryProvider,
    dispatch_registry_handle: &Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
    args: Value,
) -> anyhow::Result<Value> {
    DiscoverProviderName::parse(provider_name)?;
    let directory = agent_registry_provider()
        .map_err(|error| anyhow::anyhow!("discover: load Agent directory: {error:#}"))?;
    let registry = dispatch_registry_handle.get().ok_or_else(|| {
        anyhow::anyhow!(
            "internal_error: dispatch registry handle not yet set; \
             discover provider routing requires the daemon's live registry"
        )
    })?;
    let provider = DiscoverProviderTarget::resolve(provider_name, &directory.registry, registry)?;
    let provider_registry_name = provider.registry_name().to_string();
    let provider_target = provider.into_invocation_target(args)?;
    registry
        .invoke_rpc_target_json(provider_target)
        .map_err(|err| {
            anyhow::anyhow!(
                "discover: provider {provider_registry_name:?} is not registered or failed. Pick a registered \
                 `<agent>.discover` provider, or omit provider to use the builtin matcher. ({err})"
            )
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscoverProviderTarget {
    registry_name: String,
    ability_ura: String,
    subject_ura: String,
}

impl DiscoverProviderTarget {
    fn resolve(
        raw: &str,
        agents: &AgentRegistry,
        registry: &AxonAbilityCatalog,
    ) -> anyhow::Result<Self> {
        let provider = DiscoverProviderName::parse(raw)?;
        if !agents.agents.contains_key(provider.agent.as_str()) {
            anyhow::bail!(
                "discover: provider agent {:?} is not registered on this daemon; choose an \
                 agent from `agent.list` or omit provider",
                provider.agent
            );
        }
        let registry_name = provider.as_registry_name();
        let record = registry
            .control_plane_record_for_mode(&registry_name, crate::daemon::ability::CallMode::Rpc)
            .map_err(|err| anyhow::anyhow!("discover provider control-plane lookup: {err}"))?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "discover: provider {registry_name:?} is not registered in the control plane"
                )
            })?;
        Ok(Self {
            registry_name,
            ability_ura: record.descriptor().canonical_ability_ura().ok_or_else(|| {
                anyhow::anyhow!(
                    "discover: provider {:?} has no canonical Ability URA",
                    record.ability()
                )
            })?,
            subject_ura: record.authority().scope().authority_root().to_string(),
        })
    }

    fn registry_name(&self) -> &str {
        &self.registry_name
    }

    fn into_invocation_target(self, args: Value) -> anyhow::Result<InvocationTarget> {
        PublicInvocationTargetIssuer::local_explicit_tuple(
            self.ability_ura,
            args,
            CallMode::Rpc,
            self.subject_ura,
            axon_sdk::invocation::CausalContext::None,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscoverProviderName {
    agent: String,
}

impl DiscoverProviderName {
    fn parse(raw: &str) -> anyhow::Result<Self> {
        let raw = raw.trim();
        let owner_local = crate::core::ura::OwnerLocalAbilityName::parse(raw).map_err(|_| {
            anyhow::anyhow!("discover: provider {raw:?} must use `<agent>.discover`")
        })?;
        if owner_local.public_name() != ABILITY_VERB {
            anyhow::bail!(
                "discover: provider {raw:?} is not a discover provider; expected `<agent>.{ABILITY_VERB}`"
            );
        }
        Ok(Self {
            agent: owner_local.owner().to_string(),
        })
    }

    fn as_registry_name(&self) -> String {
        format!("{}.{}", self.agent, ABILITY_VERB)
    }
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
    federation_resolver: &dyn DiscoverFederationResolver,
    scope: Scope,
    query: Option<&str>,
    source_window: SourceWindow,
) -> anyhow::Result<Value> {
    let creds = match crate::daemon::persistence::config::load_credentials() {
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
    // `build_bootstrap_plan` in cli::start). A future config
    // split separates them; until then the same string flows into
    // both fields and `federation.resolve` accepts it as the realm
    // segment.
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

    // Pin the caller URA to the daemon's own Device URA. The session
    // membership gate requires this exact joined identity.
    let device_caller_ura = crate::core::ura::device_ura(tenant, &creds.node_id);
    // Tenant_filter wire shape mirrors RFC-002 §5 update:
    //   * User scope → None: hub auto-fills caller_tenant.
    //   * Public scope → "*": cross-tenant catalog listing.
    let tenant_filter = match scope {
        Scope::Public => Some("*".to_string()),
        _ => None,
    };
    let resolved = match federation_resolver.resolve_agents(
        tenant,
        &realm,
        device_caller_ura,
        tenant_filter,
    ) {
        Ok(r) => r,
        Err(DiscoverFederationResolveError::NotJoined(e)) => {
            return Ok(error_envelope("federation_not_joined", &e, scope, query));
        }
        Err(DiscoverFederationResolveError::Unavailable(e)) => {
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
            let Some(summary) =
                crate::daemon::federation::read_model::owner_projection::summary_from_value(&desc)
            else {
                continue;
            };
            let Some(bare_ability) =
                crate::daemon::federation::read_model::owner_projection::summary_public_name(
                    &summary,
                )
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
        // Routed through the federated-directory reader so this branch
        // compiles regardless of the `axon-pb` feature. With the
        // feature off, the reader returns an explicit capability error
        // rather than fabricating an empty directory.
        let local_user_id = match creds.user_id() {
            Ok(user_id) => user_id,
            Err(error) => {
                return Ok(error_envelope(
                    "federation_not_joined",
                    &format!("credentials cannot identify the calling user: {error}"),
                    scope,
                    query,
                ));
            }
        };
        match crate::daemon::federation::directory_reader::read_federated_directory_for_user(
            None,
            local_user_id,
        ) {
            Ok(entries) => rows.extend(federated_directory_candidates(&entries)),
            Err(error) if rows.is_empty() => {
                return Ok(error_envelope(
                    "federation_unavailable",
                    &format!("federation.discover directory read failed: {error}"),
                    scope,
                    query,
                ));
            }
            Err(_) => {}
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
    let source = source_window.apply(&mut rows);

    let candidates: Vec<Value> = rows.iter().map(Candidate::to_json).collect();
    Ok(json!({
        "candidates": candidates,
        "scope": scope.as_str(),
        "query": query,
        "source": {
            "available": source.available,
            "returned": rows.len(),
            "limit": source.limit,
            "truncated": source.truncated,
        },
    }))
}

/// Where the call wants to look. Mirrors the `[access].visibility`
/// tiers exposed in `daemon::ability::manifest::ManifestAccessScope` but is a
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceWindow {
    Bounded(usize),
    All,
}

#[derive(Debug, Clone)]
struct AppliedSourceWindow {
    available: usize,
    limit: Value,
    truncated: bool,
}

impl SourceWindow {
    fn apply(self, rows: &mut Vec<Candidate>) -> AppliedSourceWindow {
        let available = rows.len();
        match self {
            SourceWindow::Bounded(limit) => {
                let truncated = available > limit;
                rows.truncate(limit);
                AppliedSourceWindow {
                    available,
                    limit: json!(limit),
                    truncated,
                }
            }
            SourceWindow::All => AppliedSourceWindow {
                available,
                limit: Value::Null,
                truncated: false,
            },
        }
    }
}

fn parse_source_window(args: &Value) -> anyhow::Result<SourceWindow> {
    let Some(mode) = args.get("source_window") else {
        return Ok(SourceWindow::Bounded(parse_top_k(args)?));
    };
    let mode = mode.as_str().ok_or_else(|| {
        anyhow::anyhow!("discover: source_window must be \"bounded\" or \"all\"; got {mode}")
    })?;
    match mode {
        "bounded" => Ok(SourceWindow::Bounded(parse_top_k(args)?)),
        "all" => {
            if args.get("top_k").is_some() {
                anyhow::bail!("discover: source_window=\"all\" must omit top_k");
            }
            Ok(SourceWindow::All)
        }
        other => anyhow::bail!(
            "discover: unsupported source_window {other:?}; expected \"bounded\" or \"all\""
        ),
    }
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
    call_modes: BTreeSet<String>,
    visibility: ManifestAccessScope,
    scope_matched: Scope,
    score: f64,
    reason: String,
    fulfilled_by: Option<&'static str>,
    identity_state: &'static str,
    diagnostic: Option<String>,
}

impl Candidate {
    fn is_callable(&self) -> bool {
        self.identity_state == "minted" && self.fulfilled_by != Some("unbound_manifest")
    }

    fn to_json(&self) -> Value {
        json!({
            "qualified_name": self.qualified_name,
            "owner":          self.owner,
            "ability":        self.ability,
            "description":    self.description,
            "input_schema":   self.input_schema,
            "call_modes":     self.call_modes,
            "visibility":     self.visibility.as_wire_str(),
            "scope_matched":  self.scope_matched.as_str(),
            "score":          self.score,
            "reason":         self.reason,
            "fulfilled_by":   self.fulfilled_by.map(Value::from).unwrap_or(Value::Null),
            "identity_state": self.identity_state,
            "callable":       self.is_callable(),
            "diagnostic":     self.diagnostic.as_deref().map(Value::from).unwrap_or(Value::Null),
        })
    }
}

fn candidate_from_federated_summary(
    owner: &str,
    summary: &crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary,
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
        description: summary.callable_summary.description.clone(),
        input_schema,
        call_modes: BTreeSet::from([summary.callable_summary.call_mode.as_str().to_string()]),
        visibility: ManifestAccessScope::Public,
        scope_matched: scope,
        score: 0.0,
        reason: String::new(),
        fulfilled_by: Some("federation"),
        identity_state: "minted",
        diagnostic: None,
    })
}

/// Canonical local discovery projection. Descriptor identity, schema, access
/// policy, and callability come from committed control-plane records only.
/// Agent manifests are authoring inputs and never participate in this read
/// model after registration.
fn local_catalog_candidates(
    catalog: &AxonAbilityCatalog,
    directory: &AgentAggregateSnapshot,
    self_agent: &str,
    scope: Scope,
) -> anyhow::Result<Vec<Candidate>> {
    use crate::daemon::ability::descriptors::Visibility;
    use crate::daemon::ability::dispatch::OwnerKind;

    let snapshot = catalog.authority_ability_catalog_snapshot();
    let caller_ura = if self_agent.is_empty() {
        catalog.hosted_device_authority_root().map(str::to_string)
    } else {
        snapshot.iter().find_map(|row| match &row.owner {
            OwnerKind::Agent(agent) if agent == self_agent => {
                Some(row.descriptor.owner_ura.clone())
            }
            _ => None,
        })
    }
    .ok_or_else(|| {
        anyhow::anyhow!(
            "discover: caller {self_agent:?} has no authority in the canonical local Ability catalog"
        )
    })?;

    let mut candidates = BTreeMap::<String, Candidate>::new();
    for row in snapshot {
        let is_self = matches!(&row.owner, OwnerKind::Agent(agent) if agent == self_agent);
        if matches!(scope, Scope::Selfish) && !is_self {
            continue;
        }
        if !row.descriptor.is_visible_to(&caller_ura, &caller_ura) {
            continue;
        }
        let Some(qualified_name) = row.descriptor.canonical_ability_ura() else {
            continue;
        };
        let visibility = match row.descriptor.visibility {
            Visibility::Private => ManifestAccessScope::Selfish,
            Visibility::Scoped => ManifestAccessScope::Device,
            Visibility::Public => ManifestAccessScope::Public,
        };
        let owner = match &row.owner {
            OwnerKind::Agent(peer_name) => require_hosted_llm_agent_ura(directory, peer_name)?,
            _ => row.descriptor.owner_ura.clone(),
        };
        let call_mode = row.descriptor.call_mode().as_str().to_string();
        match candidates.entry(qualified_name.clone()) {
            std::collections::btree_map::Entry::Occupied(mut existing) => {
                existing.get_mut().call_modes.insert(call_mode);
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Candidate {
                    qualified_name,
                    owner,
                    ability: row.descriptor.public_name(),
                    description: row.descriptor.description.clone(),
                    input_schema: row.descriptor.input_schema().clone(),
                    call_modes: BTreeSet::from([call_mode]),
                    visibility,
                    scope_matched: if is_self {
                        Scope::Selfish
                    } else {
                        Scope::Device
                    },
                    score: 0.0,
                    reason: String::new(),
                    fulfilled_by: Some("local_catalog"),
                    identity_state: "minted",
                    diagnostic: None,
                });
            }
        }
    }
    Ok(candidates.into_values().collect())
}

fn require_hosted_llm_agent_ura(
    directory: &AgentAggregateSnapshot,
    peer_name: &str,
) -> anyhow::Result<String> {
    let identity = directory.hosted_llm_agent_ura(peer_name);
    identity.map(str::to_string).ok_or_else(|| {
        anyhow::anyhow!(
            "discover: canonical Ability owner {peer_name:?} has no unique hosted LLM Agent identity"
        )
    })
}

/// Project a federated-directory entry list into discover candidates.
///
/// RFC-005 forbids synthesizing ability identities from presence
/// facts. A directory entry proves that an owner exists or is online;
/// it does not prove that `<owner>.canonical_invoke` is a real public
/// ability. Cross-hub routing still uses the canonical `Invocation::Invoke` RPC
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
/// match, which beats a description or owner keyword hit. The numbers
/// carry no absolute meaning; only the relative ordering matters, and the
/// LLM reads `reason` for the human story.
///
/// The scored dimensions (name, description, owner) are a superset of the
/// caller-owned ranker's signals, so this reduction never drops a row the
/// final ranker would have ranked.
///
/// A future PR can replace this with a richer scorer inside a hosted
/// agent's `<agent>.discover` implementation. The function is kept
/// private and small so the swap stays behind the provider boundary.
fn score_against_query(rows: &mut [Candidate], query: &str) {
    let q = query.trim().to_lowercase();
    let q_terms = tokenize_search_terms(query);
    if q_terms.is_empty() {
        return;
    }

    for row in rows.iter_mut() {
        let name = row.ability.to_lowercase();
        let qualified = row.qualified_name.to_lowercase();
        let description = row.description.to_lowercase();
        let owner = row.owner.to_lowercase();

        let mut score: f64 = 0.0;
        let mut reasons: Vec<&str> = Vec::new();

        if name == q || qualified == q {
            score += 5.0;
            reasons.push("exact name match");
        }
        for term in &q_terms {
            if name.contains(term.as_str()) {
                score += 3.0;
                reasons.push("term in ability name");
            }
            if description.contains(term.as_str()) {
                score += 1.0;
                reasons.push("term in description");
            }
            // Owner is a first-class ranking signal: a query that names
            // the owning agent/device must produce a non-zero score so the
            // row survives the `score > 0` reduction and reaches the
            // caller-owned ranker (which also scores the owner segment).
            // Scoring fewer dimensions here than the final ranker would
            // silently drop owner-only matches before they are ever ranked.
            if !owner.is_empty() && owner.contains(term.as_str()) {
                score += 1.0;
                reasons.push("term in owner");
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

fn tokenize_search_terms(input: &str) -> Vec<String> {
    input
        .split(|c: char| !c.is_alphanumeric())
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect()
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
                "description": "Cap on returned candidates after scoring when source_window is bounded."
            },
            "source_window": {
                "type": "string",
                "enum": ["bounded", "all"],
                "default": "bounded",
                "description": "`bounded` applies top_k in the runtime source. `all` returns the complete source set so a caller-owned ranker can score before truncating."
            },
            "provider": {
                "type": "string",
                "description": "Optional hosted-agent discover provider to delegate to \
                                in `<agent>.discover` form. Provider must accept \
                                the same {scope, query, top_k, source_window} args and \
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
    use crate::daemon::ability::manifest::{AbilityManifest, AccessPolicy, ManifestAccessScope};
    use crate::daemon::persistence::agent_registry::{AgentEntry, AgentRegistry, AgentType};

    fn obj_schema() -> Value {
        json!({"type": "object"})
    }

    #[test]
    fn register_publishes_discover_manifest_description() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = runtime_test_catalog();
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );

        let record = reg
            .control_plane_record_for_mode("claude.discover", crate::daemon::ability::CallMode::Rpc)
            .expect("discover descriptor lookup is unambiguous")
            .expect("discover registration must publish its canonical descriptor");
        assert_eq!(record.descriptor().description, description());
        assert_eq!(record.descriptor().input_schema(), &input_schema());
    }

    const TEST_DEVICE_URA: &str = "easynet:///r/test/device/agent-discover";

    fn metadata_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(TEST_DEVICE_URA)
    }

    fn runtime_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_runtime_for_device_authority(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            TEST_DEVICE_URA,
        )
    }

    fn local_discovery_catalog(
        self_agent: &str,
        abilities: Vec<(
            String,
            crate::daemon::ability::dispatch::OwnerKind,
            AbilityManifest,
        )>,
    ) -> Arc<AxonAbilityCatalog> {
        let mut hosted_agents = std::collections::BTreeSet::from([self_agent.to_string()]);
        for (_, owner, _) in &abilities {
            if let crate::daemon::ability::dispatch::OwnerKind::Agent(agent) = owner {
                hosted_agents.insert(agent.clone());
            }
        }
        let authority = crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            TEST_DEVICE_URA,
            hosted_agents
                .into_iter()
                .map(|agent| crate::core::ura::agent_ura("test", "local", &agent)),
        )
        .expect("test hosted Agent authority roots");
        let mut catalog = AxonAbilityCatalog::new_metadata_only_with_authority_context(authority);
        for (registry_name, owner, manifest) in abilities {
            catalog.register_rpc_with_spec_and_action(
                &registry_name,
                owner,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
                manifest,
                Arc::new(|_| Ok(Value::Null)),
            );
        }
        let handle = Arc::new(std::sync::OnceLock::new());
        register_for_agent(
            &mut catalog,
            self_agent.to_string(),
            AgentRegistry::default,
            Arc::clone(&handle),
        );
        let catalog = Arc::new(catalog);
        handle
            .set(Arc::clone(&catalog))
            .expect("local discovery catalog attached once");
        catalog
    }

    #[test]
    fn unknown_scope_is_rejected() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = runtime_test_catalog();
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
    fn local_discovery_does_not_depend_on_agent_manifest_registry() {
        let catalog = local_discovery_catalog("claude", Vec::new());
        let handler = catalog.get_rpc("claude.discover").unwrap();
        let response = handler(json!({"scope": "self"})).unwrap();
        assert!(response["candidates"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["ability"] == "discover")));
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

    #[cfg(feature = "axon-pb")]
    #[test]
    fn local_resolver_maps_user_scope_to_realm_prefix_and_public_to_wildcard() {
        assert_eq!(
            local_resolve_prefix("acme", "acme", None).unwrap(),
            Some("easynet:///r/acme/".to_string())
        );
        assert_eq!(
            local_resolve_prefix("acme", "acme", Some("*")).unwrap(),
            None
        );
        assert_eq!(
            local_resolve_prefix("acme", "acme", Some("other")).unwrap(),
            Some("easynet:///r/other/".to_string())
        );
    }

    #[test]
    fn user_scope_falls_through_when_not_joined() {
        // The user tier is the canonical same-realm federation scope.
        // Under HomeGuard it should fail softly with a typed envelope.
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = metadata_test_catalog();
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
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = metadata_test_catalog();
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
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = metadata_test_catalog();
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
        let weather = AbilityManifest::new("weather", "Fetch weather", obj_schema()).unwrap();
        let summary = AbilityManifest::new("summarize", "Summarise text", obj_schema()).unwrap();
        let catalog = local_discovery_catalog(
            "claude",
            vec![
                (
                    "claude.weather".into(),
                    crate::daemon::ability::dispatch::OwnerKind::Agent("claude".into()),
                    weather,
                ),
                (
                    "codex.summarize".into(),
                    crate::daemon::ability::dispatch::OwnerKind::Agent("codex".into()),
                    summary,
                ),
            ],
        );
        let h = catalog.get_rpc("claude.discover").unwrap();
        let resp = h(json!({"scope": "self"})).unwrap();
        let cands = resp["candidates"].as_array().unwrap();
        assert!(cands.iter().any(|row| row["ability"] == "weather"));
        assert!(!cands.iter().any(|row| row["ability"] == "summarize"));
    }

    #[test]
    fn authoring_manifest_without_live_binding_is_not_discoverable() {
        let catalog = local_discovery_catalog("claude", Vec::new());
        let h = catalog.get_rpc("claude.discover").unwrap();
        let resp = h(json!({"scope": "self", "query": "weather"})).unwrap();
        assert!(resp["candidates"].as_array().unwrap().is_empty());
    }

    #[test]
    fn device_scope_includes_peers_with_device_visibility() {
        let weather = AbilityManifest::new("weather", "Fetch weather", obj_schema())
            .unwrap()
            .with_access(AccessPolicy {
                visibility: ManifestAccessScope::Device,
                ..Default::default()
            })
            .unwrap();
        let catalog = local_discovery_catalog(
            "codex",
            vec![(
                "claude.weather".into(),
                crate::daemon::ability::dispatch::OwnerKind::Agent("claude".into()),
                weather,
            )],
        );
        let h = catalog.get_rpc("codex.discover").unwrap();
        let resp = h(json!({"scope": "device", "query": "weather"})).unwrap();
        let cands = resp["candidates"].as_array().unwrap();
        let weather_entry = cands.iter().find(|c| c["ability"] == "weather").unwrap();
        assert_eq!(weather_entry["scope_matched"], "device");
        assert_eq!(weather_entry["visibility"], "device");
    }

    #[test]
    fn device_scope_includes_live_device_owned_easyremote_ability() {
        let inference =
            AbilityManifest::new("ai_inference", "Run local AI inference", obj_schema())
                .unwrap()
                .with_admission_action("stream")
                .unwrap()
                .with_access(AccessPolicy {
                    visibility: ManifestAccessScope::Device,
                    ..Default::default()
                })
                .unwrap();
        let authority = crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
            TEST_DEVICE_URA,
            [crate::core::ura::agent_ura("test", "local", "codex-smoke")],
        )
        .expect("test hosted Agent authority root");
        let mut catalog = AxonAbilityCatalog::new_metadata_only_with_authority_context(authority);
        catalog.register_stream_with_spec(
            "er.ai_inference",
            crate::daemon::ability::dispatch::OwnerKind::Device,
            inference,
            Arc::new(|_| anyhow::bail!("stream execution is outside discovery test scope")),
        );
        let handle = Arc::new(std::sync::OnceLock::new());
        register_for_agent(
            &mut catalog,
            "codex-smoke".to_string(),
            AgentRegistry::default,
            Arc::clone(&handle),
        );
        let catalog = Arc::new(catalog);
        handle
            .set(Arc::clone(&catalog))
            .expect("local discovery catalog attached once");

        let response = catalog.get_rpc("codex-smoke.discover").unwrap()(
            json!({"scope": "device", "query": "ai inference"}),
        )
        .unwrap();
        let candidate = response["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["ability"] == "er.ai_inference")
            .expect("live device-owned EasyRemote ability");
        assert_eq!(candidate["fulfilled_by"], "local_catalog");
        assert_eq!(candidate["callable"], true);
        assert_eq!(candidate["call_modes"], json!(["stream"]));
        assert_eq!(candidate["scope_matched"], "device");
    }

    #[test]
    fn device_scope_hides_peer_abilities_marked_self_visibility() {
        // An author who marked an ability as `[access] visibility = "self"`
        // is opting out of peer discovery. The discover handler must
        // honour that even when the caller asks for scope=device.
        let private = AbilityManifest::new("internal", "private helper", obj_schema())
            .unwrap()
            .with_access(AccessPolicy {
                visibility: ManifestAccessScope::Selfish,
                ..Default::default()
            })
            .unwrap();
        let catalog = local_discovery_catalog(
            "codex",
            vec![(
                "claude.internal".into(),
                crate::daemon::ability::dispatch::OwnerKind::Agent("claude".into()),
                private,
            )],
        );
        let h = catalog.get_rpc("codex.discover").unwrap();
        let resp = h(json!({"scope": "device"})).unwrap();
        let cands = resp["candidates"].as_array().unwrap();
        assert!(
            cands.iter().all(|c| c["ability"] != "internal"),
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
        let catalog = local_discovery_catalog(
            "claude",
            vec![
                (
                    "claude.weather".into(),
                    crate::daemon::ability::dispatch::OwnerKind::Agent("claude".into()),
                    weather,
                ),
                (
                    "claude.news".into(),
                    crate::daemon::ability::dispatch::OwnerKind::Agent("claude".into()),
                    news,
                ),
            ],
        );
        let h = catalog.get_rpc("claude.discover").unwrap();
        let resp = h(json!({"scope": "self", "query": "weather"})).unwrap();
        let cands = resp["candidates"].as_array().unwrap();
        assert_eq!(cands[0]["ability"], "weather");
    }

    #[test]
    fn query_tokenization_splits_symbol_separated_terms() {
        let screen =
            AbilityManifest::new("screen_snapshot", "Capture pixels", obj_schema()).unwrap();
        let catalog = local_discovery_catalog(
            "claude",
            vec![(
                "claude.screen_snapshot".into(),
                crate::daemon::ability::dispatch::OwnerKind::Agent("claude".into()),
                screen,
            )],
        );
        let h = catalog.get_rpc("claude.discover").unwrap();
        let resp = h(json!({"scope": "self", "query": "screen-snapshot"})).unwrap();
        let cands = resp["candidates"].as_array().unwrap();

        assert!(
            cands.iter().any(|c| c["ability"] == "screen_snapshot"),
            "symbol-separated query should match underscore-separated ability name: {cands:#?}"
        );
    }

    #[test]
    fn top_k_zero_is_rejected() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = metadata_test_catalog();
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
    fn source_window_all_returns_complete_source() {
        let abilities = (0..25)
            .map(|idx| {
                let name = format!("ability_{idx:02}");
                let manifest =
                    AbilityManifest::new(&name, format!("Ability {idx}"), obj_schema()).unwrap();
                (
                    format!("claude.{name}"),
                    crate::daemon::ability::dispatch::OwnerKind::Agent("claude".into()),
                    manifest,
                )
            })
            .collect();
        let catalog = local_discovery_catalog("claude", abilities);
        let h = catalog.get_rpc("claude.discover").unwrap();
        let resp = h(json!({"scope": "self", "source_window": "all"})).unwrap();

        assert_eq!(resp["candidates"].as_array().unwrap().len(), 26);
        assert_eq!(resp["source"]["available"], json!(26));
        assert_eq!(resp["source"]["returned"], json!(26));
        assert!(resp["source"]["limit"].is_null());
        assert_eq!(resp["source"]["truncated"], json!(false));
    }

    #[test]
    fn source_window_all_rejects_top_k() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = metadata_test_catalog();
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("claude.discover").unwrap();
        let err = h(json!({"source_window": "all", "top_k": 1})).unwrap_err();
        assert!(format!("{err}").contains("must omit top_k"));
    }

    #[test]
    fn provider_arg_delegates_to_named_handler() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        // The discover handler routes to the named provider via the
        // dispatch registry. Pin both halves: (a) the provider IS
        // called, (b) the `provider` field is stripped before the
        // forwarded args reach it (so the provider doesn't see a
        // recursion-trigger).
        use std::sync::Mutex;
        let captured: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let captured_for_provider = Arc::clone(&captured);
        let provider_handler: crate::daemon::ability::dispatch::LocalRpcHandler =
            Arc::new(move |args: Value| {
                *captured_for_provider.lock().unwrap() = Some(args);
                Ok(json!({
                    "candidates": [],
                    "scope": "self",
                    "query": null,
                    "provider": "userx.discover (mock)"
                }))
            });

        // Build a registry with the provider handler + the per-agent
        // discover. Wire the OnceLock to the same runtime-backed registry so
        // the provider delegation exercises the same LocalRuntime boundary as
        // daemon dispatch.
        let mut reg = runtime_test_catalog();
        reg.register_rpc_with_owner_and_action(
            "userx.discover",
            crate::daemon::ability::dispatch::OwnerKind::Device,
            crate::daemon::ability::descriptors::AdmissionAction::Read,
            provider_handler,
        );
        let handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>> =
            Arc::new(std::sync::OnceLock::new());
        let mut agents = AgentRegistry::default();
        agents.agents.insert(
            "claude".to_string(),
            AgentEntry::new(AgentType::ClaudeCode, None),
        );
        agents
            .agents
            .insert("userx".to_string(), AgentEntry::new(AgentType::Codex, None));
        let agents_clone = agents.clone();
        register_for_agent(
            &mut reg,
            "claude".into(),
            move || agents_clone.clone(),
            Arc::clone(&handle),
        );
        let arc_reg = Arc::new(reg);
        handle.set(Arc::clone(&arc_reg)).expect("handle set once");

        let h = arc_reg.resolve_rpc("claude.discover").unwrap();
        let resp = h(json!({
            "scope": "self",
            "query": "weather",
            "provider": "userx.discover"
        }))
        .unwrap();
        // Provider's response is returned verbatim.
        assert_eq!(resp["provider"], "userx.discover (mock)");
        // Provider received args without `provider` field.
        let captured_args = captured.lock().unwrap().clone().unwrap();
        assert!(captured_args.get("provider").is_none());
        assert_eq!(captured_args["query"], "weather");
    }

    #[test]
    fn provider_target_is_descriptor_bound_with_explicit_subject() {
        let mut reg = metadata_test_catalog();
        reg.register_rpc_with_owner_and_action(
            "userx.discover",
            crate::daemon::ability::dispatch::OwnerKind::Device,
            crate::daemon::ability::descriptors::AdmissionAction::Read,
            Arc::new(|_args| Ok(json!({"candidates": []}))),
        );
        let mut agents = AgentRegistry::default();
        agents
            .agents
            .insert("userx".to_string(), AgentEntry::new(AgentType::Codex, None));

        let target =
            DiscoverProviderTarget::resolve("userx.discover", &agents, &reg).expect("provider");
        let invocation_target = target
            .into_invocation_target(json!({"query": "weather"}))
            .expect("valid provider invocation target");

        assert!(
            crate::core::ura::AbilitySelector::parse(&invocation_target.ability).is_ok(),
            "provider delegation must dispatch a descriptor-bound Ability URA"
        );
        assert!(
            invocation_target
                .subject
                .as_deref()
                .is_some_and(|subject| { crate::core::ura::parse_ura(subject).is_ok() }),
            "provider delegation must not rely on a missing subject default"
        );
        assert!(matches!(
            invocation_target.causal_context,
            crate::daemon::invocation::routing::target::InvocationCausalContext::Explicit(
                axon_sdk::invocation::CausalContext::None
            )
        ));
    }

    #[test]
    fn provider_without_dot_is_rejected() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = metadata_test_catalog();
        register_for_agent(
            &mut reg,
            "claude".into(),
            AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );
        let h = reg.get_rpc("claude.discover").unwrap();
        let err = h(json!({"provider": "bogus"})).unwrap_err();
        assert!(format!("{err}").contains("<agent>.discover"));
    }

    #[test]
    fn provider_not_registered_returns_typed_error() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = metadata_test_catalog();
        let handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>> =
            Arc::new(std::sync::OnceLock::new());
        let mut agents = AgentRegistry::default();
        agents
            .agents
            .insert("userx".to_string(), AgentEntry::new(AgentType::Codex, None));
        let agents_clone = agents.clone();
        register_for_agent(
            &mut reg,
            "claude".into(),
            move || agents_clone.clone(),
            Arc::clone(&handle),
        );
        let arc_reg = Arc::new(reg);
        handle.set(Arc::clone(&arc_reg)).expect("set");
        let h = arc_reg.resolve_rpc("claude.discover").unwrap();
        let err = h(json!({"provider": "userx.discover"})).unwrap_err();
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
    ) -> crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
        crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary {
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
            callable_summary: crate::daemon::federation::read_model::owner_projection::AbilityCallableSummary::minimal(
                "chat",
            ),
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
        assert_eq!(candidate.visibility, ManifestAccessScope::Public);
        assert_eq!(candidate.fulfilled_by, Some("federation"));
    }

    #[test]
    fn federated_summary_candidate_preserves_description() {
        let mut summary = federated_summary("easynet:///r/acme/ability/alice.bot.chat");
        summary.callable_summary.description = "Chat with Alice's assistant".to_string();

        let candidate = candidate_from_federated_summary(
            "easynet:///r/acme/agent/alice.bot",
            &summary,
            "chat".to_string(),
            Scope::Public,
        )
        .expect("complete federated summary should project");

        assert_eq!(candidate.description, "Chat with Alice's assistant");
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
            "directory presence must not synthesize a canonical_invoke Ability candidate"
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

    /// Build a minimal `Candidate` whose only query-relevant signal is the
    /// owner segment — name and description deliberately share no token
    /// with the query so the row scores zero unless the owner is ranked.
    fn owner_only_candidate(owner: &str) -> Candidate {
        let owner_ura = crate::core::ura::agent_ura("acme", "u", owner);
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "fs.read").expect("agent ability URA");
        Candidate {
            qualified_name: ability_ura,
            owner: owner.to_string(),
            ability: "fs.read".to_string(),
            description: "read a file from disk".to_string(),
            input_schema: json!({"type": "object"}),
            call_modes: BTreeSet::from(["rpc".to_string()]),
            visibility: ManifestAccessScope::Device,
            scope_matched: Scope::Device,
            score: 0.0,
            reason: String::new(),
            fulfilled_by: None,
            identity_state: "minted",
            diagnostic: None,
        }
    }

    /// Regression: the runtime reducer scores the owner segment, so a
    /// query that names the owning agent survives the `score > 0`
    /// reduction and reaches the caller-owned ranker. Before this fix the
    /// owner dimension was scored only by the CLI ranker, which never saw
    /// the row because the reducer dropped it first.
    #[test]
    fn owner_name_query_survives_score_reduction() {
        let mut rows = vec![owner_only_candidate("codex")];
        score_against_query(&mut rows, "codex");
        rows.retain(|c| c.score > 0.0);
        assert_eq!(rows.len(), 1, "owner-only match must not be dropped");
        assert!(rows[0].reason.contains("owner"));
    }

    #[test]
    fn unrelated_query_still_drops_the_row() {
        let mut rows = vec![owner_only_candidate("codex")];
        score_against_query(&mut rows, "weather");
        rows.retain(|c| c.score > 0.0);
        assert!(
            rows.is_empty(),
            "a genuinely unrelated query must score zero"
        );
    }
}

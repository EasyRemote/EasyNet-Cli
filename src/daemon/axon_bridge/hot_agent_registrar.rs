//! Hot-add agent registration into `LocalRuntime`.
//!
//! Phase 5c blocker: `agent.start` / `agent.stop`
//! used to mutate only `agents.json` on disk. The new agent's
//! `<agent>.chat / discover / invoke` handlers were not registered
//! in the Axon runtime; an older catalog lookup-miss fallback
//! synthesized them on demand. RFC-005 route selection cannot rely
//! on such hidden executable routes, so hot additions must now
//! materialise runtime handlers before the owner projection is
//! advertised.
//!
//! This registrar closes the gap. `agent.start` calls
//! `register_agent(name, entry)` which builds the same handler set
//! the boot path needs, then commits every row through
//! [`AxonAbilityCatalog`]'s dynamic registration transaction. That
//! transaction is the single writer for descriptor facts, authority
//! binding, implementation binding, dynamic catalogue metadata, and
//! the [`LocalRuntime`] executable row. Product-facing
//! `<agent>.<verb>` names stay at the catalog/manifest boundary only.
//! `agent.stop` calls `unregister_agent(name)`, which decodes runtime
//! keys back to owner-local public names and removes the matching
//! dynamic rows through the same catalogue transaction.
//!
//! ## Boot-order rationale
//!
//! The `AxonAbilityCatalog` is constructed *before* the Axon
//! `LocalRuntime` (registry comes from
//! `daemon::ability::catalog::build_registry_with_services` in the daemon's
//! Stage 2; runtime comes later in
//! `invocation_transport::start_daemon_invocation_transport`).
//! But `agent.start`'s handler closure has to be installed at
//! registry-build time. We bridge that by parking the
//! [`LocalRuntime`] handle in an internal [`OnceLock`]: the
//! registrar is constructed *pending* at registry-build time, the
//! handler closure captures `Arc<Self>`, and boot calls
//! [`HotAgentRegistrar::set_runtime`] once the runtime is ready.
//! Any `register_agent` call that lands before the runtime is wired
//! returns `runtime_not_ready` and emits a diagnostic op_event —
//! the disk-side `agents.json` write still succeeds, so the next
//! daemon restart replays the current hosted-agent registry through
//! this same dynamic registrar after the catalogue OnceLock is set.

use std::sync::{Arc, OnceLock};

use easynet_axon::invocation::LocalRuntime;

use crate::daemon::ability::builtins::agents::chat::{
    build_agent_ability_handler, build_chat_handler_for, build_chat_stream_handler_for,
    build_discover_handler_for, build_host_stream_handler, build_invoke_handler_for, ContextLoader,
};
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::persistence::agent_registry::AgentEntry;

pub(crate) fn block_on_hot_registrar<F, T>(future: F) -> Option<T>
where
    F: std::future::Future<Output = T> + Send,
    T: Send,
{
    crate::support::async_bridge::try_run_blocking_in_tokio(future)
}

#[derive(Debug, Clone)]
struct HostedAgentRuntimeBinding {
    agent_ura: String,
}

struct HotAgentRuntimeSyncContext<'a> {
    runtime: &'a Arc<LocalRuntime>,
    catalog: &'a Arc<AxonAbilityCatalog>,
    binding: &'a HostedAgentRuntimeBinding,
    outcome: &'a mut HotAgentRuntimeSyncOutcome,
}

impl HostedAgentRuntimeBinding {
    fn load(name: &str) -> anyhow::Result<Self> {
        let local_agents = crate::daemon::persistence::local_agents::load()
            .map_err(|err| anyhow::anyhow!("load local hosted agents: {err}"))?;
        let entry = crate::daemon::persistence::local_agents::lookup_hosted_agent_by_name(
            &local_agents,
            name,
        )?
        .ok_or_else(|| {
            anyhow::anyhow!("hosted agent {name:?} is missing from local-agents.json")
        })?;
        let parsed = crate::core::ura::parse_ura(&entry.agent_ura).map_err(|err| {
            anyhow::anyhow!("invalid hosted agent URA {:?}: {err}", entry.agent_ura)
        })?;
        if parsed.kind != crate::core::ura::URAKind::Agent {
            anyhow::bail!(
                "hosted agent {name:?} resolved to non-Agent URA {:?}",
                entry.agent_ura
            );
        }
        let Some((_, agent_id)) = parsed.agent_ids().or_else(|| parsed.device_agent_ids()) else {
            anyhow::bail!(
                "hosted agent {name:?} URA {:?} does not expose an agent id",
                entry.agent_ura
            );
        };
        if agent_id != name {
            anyhow::bail!(
                "hosted agent {name:?} resolved to URA {:?} with mismatched agent id {agent_id:?}",
                entry.agent_ura
            );
        }
        Ok(Self {
            agent_ura: entry.agent_ura.clone(),
        })
    }

    fn runtime_ability_ura(&self, registry_ability: &str) -> Option<String> {
        let public_name =
            crate::core::ura::owner_local_ability_name(&self.agent_ura, registry_ability);
        crate::core::ura::owner_ability_ura(&self.agent_ura, &public_name)
    }
}

fn dispatch_key_for_hosted_agent_runtime_key(
    runtime_key: &str,
    expected_agent: &str,
) -> Option<String> {
    let selector = crate::core::ura::AbilitySelector::parse(runtime_key).ok()?;
    let parsed_owner = crate::core::ura::parse_ura(selector.owner_ura()).ok()?;
    if parsed_owner.kind != crate::core::ura::URAKind::Agent {
        return None;
    }
    let (_, agent_id) = parsed_owner
        .agent_ids()
        .or_else(|| parsed_owner.device_agent_ids())?;
    if agent_id != expected_agent {
        return None;
    }
    Some(crate::core::ura::local_dispatch_ability_key(
        selector.owner_ura(),
        selector.public_name(),
    ))
}

async fn hosted_agent_runtime_ability_uras_for_agent(
    runtime: &Arc<LocalRuntime>,
    agent: &str,
) -> Vec<String> {
    let prefix = format!("{agent}.");
    runtime
        .list_abilities()
        .await
        .into_iter()
        .filter_map(|descriptor| {
            let dispatch_key = dispatch_key_for_hosted_agent_runtime_key(&descriptor.name, agent)?;
            dispatch_key.starts_with(&prefix).then_some(descriptor.name)
        })
        .collect()
}

/// Captures every dependency a hot-add path needs to synthesise an
/// agent's handler set + register it into the ability catalogue.
///
/// Constructed *pending* (without a runtime) at registry-build time
/// via [`HotAgentRegistrar::new_pending`]. Boot calls
/// [`HotAgentRegistrar::set_runtime`] once the Axon `LocalRuntime`
/// exists, at which point `register_agent` / `unregister_agent`
/// start landing rows into the runtime.
pub struct HotAgentRegistrar {
    /// Populated by [`HotAgentRegistrar::set_runtime`] after the
    /// Axon `LocalRuntime` is constructed in boot. Reads before that
    /// point return `None` and the registrar no-ops.
    runtime: OnceLock<Arc<LocalRuntime>>,
    loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
    /// The discover + invoke handlers re-enter local dispatch through
    /// this handle to resolve peer-agent ability descriptors.
    dispatch_handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
    /// Federation resolver used by hot-added `<agent>.discover`
    /// handlers. Must match the boot-time handler dependency so hot
    /// agents do not observe a different user/public tier.
    discover_federation_resolver:
        crate::daemon::ability::builtins::agents::discover::SharedDiscoverFederationResolver,
    /// Optional hub-advertise bridge for hot-added hosted agents.
    ///
    /// Runtime registration is local; hub visibility is separate.
    /// Device-mode boot wires this after the long-lived
    /// `session.open` escalation channel exists. Tests and
    /// non-device modes leave it empty, in which case
    /// `agent.start` still succeeds locally and the next
    /// reconnect/boot advertise sweep repairs hub visibility.
    hot_advertiser: OnceLock<Arc<dyn HotAgentAdvertiser>>,
}

/// True when `name` claims the reserved `device.` owner token — the
/// grammar slot for device-sponsored System Agents
/// (`agent/device.<device-id>.<agent-id>`, RFC-005 §3.1.2 / DEC-F048).
/// Hosted user agents MUST NOT register under it: a hosted agent named
/// `device.<x>` would mint `device.<x>.*` runtime rows that read as
/// device-owned ability shapes downstream.
#[must_use]
pub fn name_claims_reserved_device_owner(name: &str) -> bool {
    name == "device" || name.starts_with("device.")
}

/// Outcome reported back to the caller of `register_agent` /
/// `unregister_agent` so the lifecycle handler's op_event line can
/// surface how many `<agent>.*` rows actually landed.
#[derive(Debug, Default, Clone, Copy)]
pub struct HotAgentRuntimeSyncOutcome {
    pub registered: usize,
    pub replaced: usize,
    pub failed: usize,
    /// True when the agent name claimed the reserved `device.` owner
    /// token and the whole registration was refused (RFC-005 §3.1.2:
    /// hosted user agent ≠ device-sponsored System Agent). No runtime
    /// rows were touched.
    pub rejected_reserved_owner: bool,
    /// Rows reconciled away: previously-registered `<agent>.*`
    /// abilities whose backing manifest is gone. The registrar owns
    /// the whole `<agent>.` LocalRuntime namespace (see
    /// `unregister_agent`'s prefix wipe), so anything it did not
    /// just (re-)register is stale by definition.
    pub removed: usize,
    /// True when the call landed before `set_runtime` was called.
    /// Distinguishes "deliberate no-op due to boot ordering" from
    /// "every register attempt errored". The disk side still wrote
    /// `agents.json`, so the agent comes up on daemon restart.
    pub runtime_not_ready: bool,
    /// True when the catalogue `OnceLock` has not been populated yet.
    /// Runtime-only registration is forbidden: a hosted-agent ability
    /// row without descriptor/authority/implementation facts is not a
    /// valid EasyNet ability.
    pub catalog_not_ready: bool,
}

/// Input for a hot hosted-agent advertise pass.
///
/// `agent_ura` drives `federation.advertise_agent` (identity). When
/// `abilities_payload` + `abilities_resource_ura` are present, the
/// advertiser ALSO fires `federation.advertise_abilities` on the same
/// transport so a hot ability add/remove reaches the hub immediately
/// instead of waiting for the next heartbeat. ISS-002.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotAgentAdvertiseRequest {
    pub agent_ura: String,
    /// Pre-encoded `federation.advertise_abilities` args (built from the
    /// just-persisted owner projection via
    /// `advertise::advertise_abilities_payload`). `None` skips the
    /// abilities advertise (identity-only). The advertiser targets the
    /// hub federation surface by ability name, so no resource URA is
    /// carried here.
    pub abilities_payload: Option<Vec<u8>>,
}

/// Outcome for best-effort hub advertisement after hot agent add.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HotAgentAdvertiseOutcome {
    pub advertised: bool,
    pub error: Option<String>,
}

/// Input for a hot hosted-agent revoke pass (`agent.stop`). Drives
/// `federation.revoke` so the agent identity is removed from the hub
/// directory immediately, symmetric to `advertise_hosted_agent`.
/// ISS-002.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotAgentRevokeRequest {
    pub agent_ura: String,
    pub reason: String,
}

/// Narrow abstraction over the transport used to notify the hub
/// about a hot-added hosted agent.
///
/// The registrar owns the trait object so runtime lifecycle code
/// does not depend on `daemon::invocation` concrete
/// session types. Device-mode boot supplies an implementation backed
/// by the current `session.open` bidi; tests can supply a recorder.
pub trait HotAgentAdvertiser: Send + Sync {
    fn advertise_hosted_agent(&self, request: HotAgentAdvertiseRequest)
        -> HotAgentAdvertiseOutcome;

    /// Revoke a hot-removed hosted agent's identity from the hub
    /// directory (`federation.revoke`). Default is a no-op outcome so
    /// recorders/tests that only care about advertise need not
    /// implement it; the device-mode session advertiser overrides it.
    /// ISS-002.
    fn revoke_hosted_agent(&self, request: HotAgentRevokeRequest) -> HotAgentAdvertiseOutcome {
        let _ = request;
        HotAgentAdvertiseOutcome {
            advertised: false,
            error: None,
        }
    }
}

impl HotAgentRegistrar {
    /// Build a *pending* registrar — runtime not yet attached.
    /// Construct at registry-build time so the lifecycle ability
    /// closure can capture a stable `Arc<Self>` before
    /// `LocalRuntime` is built.
    #[must_use]
    pub fn new_pending(
        loaders: Arc<Vec<Arc<dyn ContextLoader>>>,
        dispatch_handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
        discover_federation_resolver: crate::daemon::ability::builtins::agents::discover::SharedDiscoverFederationResolver,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime: OnceLock::new(),
            loaders,
            dispatch_handle,
            discover_federation_resolver,
            hot_advertiser: OnceLock::new(),
        })
    }

    /// Attach the live `LocalRuntime`. Idempotent on the first
    /// successful set; subsequent calls are ignored (`OnceLock`
    /// semantics). Called from `boot.rs` exactly once, after
    /// `build_local_runtime`.
    pub fn set_runtime(&self, runtime: Arc<LocalRuntime>) {
        let _ = self.runtime.set(runtime);
    }

    /// Attach the hot advertise bridge. Idempotent first-writer-wins
    /// to mirror [`Self::set_runtime`]; boot should call this at most
    /// once after the device-mode session escalation handle exists.
    pub fn set_hot_agent_advertiser(&self, advertiser: Arc<dyn HotAgentAdvertiser>) {
        let _ = self.hot_advertiser.set(advertiser);
    }

    /// Clone the current hot advertise bridge if boot wired one.
    #[must_use]
    pub fn hot_agent_advertiser(&self) -> Option<Arc<dyn HotAgentAdvertiser>> {
        self.hot_advertiser.get().cloned()
    }

    /// Register the canonical `<agent>.chat / discover / invoke`
    /// triple plus every executable TOML-declared `<agent>.<verb>`
    /// for `name` through the dynamic catalogue transaction.
    ///
    /// **Replace-capable.** Dynamic registration is idempotent at
    /// the product ability name: existing catalogue/control-plane/
    /// runtime rows are replaced as one unit. This is required for
    /// `agent set`, `agent.refresh`, and `meta.acquire`/`forget`,
    /// which update durable state first and then refresh live rows
    /// for names that may already be present.
    pub async fn register_agent(
        &self,
        name: &str,
        entry: &AgentEntry,
    ) -> HotAgentRuntimeSyncOutcome {
        // DEC-F048 enforcement gate: the registrar owns hosted
        // agents' owner-local public namespace before mapping it to
        // LocalRuntime Ability URA keys. It refuses to mint rows
        // under the reserved `device.` owner token regardless of
        // caller — the lifecycle surface rejects earlier with a
        // user-facing error; this is the invariant's home.
        if name_claims_reserved_device_owner(name) {
            crate::op_event!(
                component = axon_bridge,
                kind = hot_agent_register_reserved_owner_rejected,
                agent = name,
                message = "`device.` is the reserved owner token for \
                          device-sponsored System Agents (RFC-005 §3.1.2); \
                          hosted user agents cannot register under it",
            );
            return HotAgentRuntimeSyncOutcome {
                rejected_reserved_owner: true,
                ..Default::default()
            };
        }

        let Some(runtime) = self.runtime.get() else {
            crate::op_event!(
                component = axon_bridge,
                kind = hot_agent_register_runtime_not_ready,
                agent = name,
                message = "agent.start landed before LocalRuntime was wired; \
                          agents.json still written, agent comes up on daemon restart",
            );
            return HotAgentRuntimeSyncOutcome {
                runtime_not_ready: true,
                ..Default::default()
            };
        };
        let Some(catalog) = self.dispatch_handle.get().cloned() else {
            crate::op_event!(
                component = axon_bridge,
                kind = hot_agent_register_catalog_not_ready,
                agent = name,
                message = "agent runtime registration refused because the live ability \
                          catalogue is not wired; runtime-only rows are invalid",
            );
            return HotAgentRuntimeSyncOutcome {
                failed: 1,
                catalog_not_ready: true,
                ..Default::default()
            };
        };

        let binding = match HostedAgentRuntimeBinding::load(name) {
            Ok(binding) => binding,
            Err(err) => {
                let err_msg = err.to_string();
                crate::op_event!(
                    component = axon_bridge,
                    kind = hot_agent_register_identity_missing,
                    agent = name,
                    error = err_msg.as_str(),
                    message = "hosted agent runtime registration requires canonical local-agents identity",
                );
                return HotAgentRuntimeSyncOutcome {
                    failed: 1,
                    ..Default::default()
                };
            }
        };

        let mut outcome = HotAgentRuntimeSyncOutcome::default();
        let owner = OwnerKind::Agent(name.to_string());
        // Every catalogue row this sync (re-)registers; the reconcile pass at
        // the end removes any other row whose decoded public ability name is
        // `<name>.*` — its backing manifest is gone.
        let mut synced: std::collections::HashSet<String> = std::collections::HashSet::new();
        {
            let mut sync_ctx = HotAgentRuntimeSyncContext {
                runtime,
                catalog: &catalog,
                binding: &binding,
                outcome: &mut outcome,
            };

            // ── chat
            let chat_ability = format!("{name}.chat");
            let chat_handler =
                build_chat_handler_for(name.to_string(), entry.clone(), Arc::clone(&self.loaders));
            if Self::register_rpc_with_spec(
                &mut sync_ctx,
                &chat_ability,
                owner.clone(),
                crate::core::ability::spec::default_chat_manifest(),
                chat_handler,
            )
            .await
            {
                synced.insert(chat_ability.clone());
            }

            let chat_stream_handler = build_chat_stream_handler_for(
                name.to_string(),
                entry.clone(),
                Arc::clone(&self.loaders),
            );
            if Self::register_stream_with_spec(
                &mut sync_ctx,
                &chat_ability,
                owner.clone(),
                crate::core::ability::spec::default_chat_manifest(),
                chat_stream_handler,
            )
            .await
            {
                synced.insert(chat_ability.clone());
            }

            // ── discover
            let discover_handler = build_discover_handler_for(
                name.to_string(),
                Arc::clone(&self.dispatch_handle),
                Arc::clone(&self.discover_federation_resolver),
            );
            let discover_ability = format!("{name}.discover");
            if Self::register_rpc_with_spec(
                &mut sync_ctx,
                &discover_ability,
                owner.clone(),
                crate::daemon::ability::builtins::agents::discover::manifest(),
                discover_handler,
            )
            .await
            {
                synced.insert(discover_ability);
            }

            // ── invoke
            let invoke_handler =
                build_invoke_handler_for(name.to_string(), Arc::clone(&self.dispatch_handle));
            let invoke_ability = format!("{name}.invoke");
            if Self::register_rpc_with_spec(
                &mut sync_ctx,
                &invoke_ability,
                owner.clone(),
                crate::daemon::ability::builtins::agents::invoke::manifest(),
                invoke_handler,
            )
            .await
            {
                synced.insert(invoke_ability);
            }

            // ── TOML-declared executor-bound abilities. Manifests without
            // `[exec]` are discoverable declarations, not invocable runtime
            // handlers.
            let chat_name = format!("{name}.chat");
            let manifests =
                crate::daemon::execution::mission::agent_ability_specs::manifests_for(name, entry);
            for spec in
                crate::daemon::execution::mission::agent_ability_specs::abilities_for(name, entry)
            {
                let ability_name = spec.name().to_string();
                if ability_name == chat_name {
                    continue;
                }
                let bare = ability_name
                    .strip_prefix(&format!("{name}."))
                    .unwrap_or(&ability_name)
                    .to_string();

                let Some(manifest) = manifests.iter().find(|m| m.name() == bare) else {
                    continue;
                };
                let Some(exec) = manifest.exec() else {
                    continue;
                };
                match exec {
                    crate::core::ability::spec::AbilityExec::HostStream(stream_spec) => {
                        let h = build_host_stream_handler(stream_spec.clone());
                        if Self::register_stream_with_envelope_and_spec(
                            &mut sync_ctx,
                            &ability_name,
                            owner.clone(),
                            manifest.clone(),
                            h,
                        )
                        .await
                        {
                            synced.insert(ability_name);
                        }
                    }
                    _ => {
                        let h = build_agent_ability_handler(
                            name.to_string(),
                            entry.clone(),
                            Arc::clone(&self.loaders),
                            bare,
                        );
                        if Self::register_rpc_with_spec(
                            &mut sync_ctx,
                            &ability_name,
                            owner.clone(),
                            manifest.clone(),
                            h,
                        )
                        .await
                        {
                            synced.insert(ability_name);
                        }
                    }
                }
            }
        }

        // ── reconcile: a provider withdraws an ability by deleting
        // its TOML; the row must leave the live runtime on the next
        // sync, not on the next daemon restart.
        for stale in hosted_agent_runtime_ability_uras_for_agent(runtime, name).await {
            let Some(dispatch_key) = dispatch_key_for_hosted_agent_runtime_key(&stale, name) else {
                continue;
            };
            if synced.contains(&dispatch_key) {
                continue;
            }
            match catalog.hot_unregister(&dispatch_key) {
                Ok(true) => {
                    outcome.removed += 1;
                    crate::op_event!(
                        component = axon_bridge,
                        kind = hot_agent_ability_reconciled_removed,
                        agent = name,
                        ability = dispatch_key.as_str(),
                        message = "ability manifest gone; dynamic catalogue row removed",
                    );
                }
                Ok(false) => {}
                Err(err) => {
                    outcome.failed += 1;
                    let err_msg = err.to_string();
                    crate::op_event!(
                        component = axon_bridge,
                        kind = hot_agent_ability_reconcile_failed,
                        agent = name,
                        ability = dispatch_key.as_str(),
                        error = err_msg.as_str(),
                    );
                }
            }
        }

        outcome
    }

    /// Unregister every dynamic `<name>.*` hosted-agent ability.
    /// Returns the count of catalogue rows actually removed.
    ///
    /// Runtime rows are keyed by hosted-agent Ability URAs. Removal
    /// decodes each row back to its owner-local public name before
    /// matching the `<name>.*` product namespace, then removes it
    /// through `AxonAbilityCatalog::hot_unregister` so control-plane
    /// and dynamic side tables cannot drift from the executable row.
    ///
    /// No-op (returns 0) when the runtime is not yet wired —
    /// the disk-side `agents.json` row is already gone, so the
    /// next daemon restart won't bring the agent back.
    pub async fn unregister_agent(&self, name: &str) -> usize {
        let Some(runtime) = self.runtime.get() else {
            return 0;
        };
        let Some(catalog) = self.dispatch_handle.get().cloned() else {
            crate::op_event!(
                component = axon_bridge,
                kind = hot_agent_unregister_catalog_not_ready,
                agent = name,
                message = "agent runtime unregister refused because the live ability \
                          catalogue is not wired",
            );
            return 0;
        };
        let mut removed = 0;
        for runtime_key in hosted_agent_runtime_ability_uras_for_agent(runtime, name).await {
            let Some(dispatch_key) = dispatch_key_for_hosted_agent_runtime_key(&runtime_key, name)
            else {
                continue;
            };
            match catalog.hot_unregister(&dispatch_key) {
                Ok(true) => removed += 1,
                Ok(false) => {}
                Err(err) => {
                    let err_msg = err.to_string();
                    crate::op_event!(
                        component = axon_bridge,
                        kind = hot_agent_unregister_failed,
                        agent = name,
                        ability = dispatch_key.as_str(),
                        error = err_msg.as_str(),
                    );
                }
            }
        }
        removed
    }

    async fn register_rpc_with_spec(
        ctx: &mut HotAgentRuntimeSyncContext<'_>,
        ability_name: &str,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: crate::daemon::ability::dispatch::LocalRpcHandler,
    ) -> bool {
        let was_present = match ctx.binding.runtime_ability_ura(ability_name) {
            Some(runtime_key) => {
                Self::runtime_has_mode(
                    ctx.runtime,
                    &runtime_key,
                    crate::daemon::ability::CallMode::Rpc,
                )
                .await
            }
            None => {
                Self::record_bad_runtime_key(ctx.binding, ability_name, ctx.outcome);
                return false;
            }
        };
        match ctx
            .catalog
            .hot_register_rpc_with_spec(ability_name, owner, manifest, handler)
        {
            Ok(()) if was_present => {
                ctx.outcome.replaced += 1;
                true
            }
            Ok(()) => {
                ctx.outcome.registered += 1;
                true
            }
            Err(err) => {
                Self::record_registration_error(ability_name, err, ctx.outcome);
                false
            }
        }
    }

    async fn register_stream_with_spec(
        ctx: &mut HotAgentRuntimeSyncContext<'_>,
        ability_name: &str,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: crate::daemon::ability::dispatch::LocalStreamHandler,
    ) -> bool {
        let was_present = match ctx.binding.runtime_ability_ura(ability_name) {
            Some(runtime_key) => {
                Self::runtime_has_mode(
                    ctx.runtime,
                    &runtime_key,
                    crate::daemon::ability::CallMode::Stream,
                )
                .await
            }
            None => {
                Self::record_bad_runtime_key(ctx.binding, ability_name, ctx.outcome);
                return false;
            }
        };
        match ctx
            .catalog
            .hot_register_stream_with_spec(ability_name, owner, manifest, handler)
        {
            Ok(()) if was_present => {
                ctx.outcome.replaced += 1;
                true
            }
            Ok(()) => {
                ctx.outcome.registered += 1;
                true
            }
            Err(err) => {
                Self::record_registration_error(ability_name, err, ctx.outcome);
                false
            }
        }
    }

    async fn register_stream_with_envelope_and_spec(
        ctx: &mut HotAgentRuntimeSyncContext<'_>,
        ability_name: &str,
        owner: OwnerKind,
        manifest: crate::core::ability::spec::AbilityManifest,
        handler: crate::daemon::ability::dispatch::LocalStreamHandlerWithEnvelope,
    ) -> bool {
        let was_present = match ctx.binding.runtime_ability_ura(ability_name) {
            Some(runtime_key) => {
                Self::runtime_has_mode(
                    ctx.runtime,
                    &runtime_key,
                    crate::daemon::ability::CallMode::Stream,
                )
                .await
            }
            None => {
                Self::record_bad_runtime_key(ctx.binding, ability_name, ctx.outcome);
                return false;
            }
        };
        match ctx.catalog.hot_register_stream_with_envelope_and_spec(
            ability_name,
            owner,
            manifest,
            handler,
        ) {
            Ok(()) if was_present => {
                ctx.outcome.replaced += 1;
                true
            }
            Ok(()) => {
                ctx.outcome.registered += 1;
                true
            }
            Err(err) => {
                Self::record_registration_error(ability_name, err, ctx.outcome);
                false
            }
        }
    }

    fn record_bad_runtime_key(
        binding: &HostedAgentRuntimeBinding,
        ability_name: &str,
        outcome: &mut HotAgentRuntimeSyncOutcome,
    ) {
        outcome.failed += 1;
        crate::op_event!(
            component = axon_bridge,
            kind = hot_agent_register_failed,
            ability = ability_name,
            agent_ura = binding.agent_ura.as_str(),
            error = "derive hosted agent ability URA failed",
        );
    }

    async fn runtime_has_mode(
        runtime: &Arc<LocalRuntime>,
        runtime_key: &str,
        call_mode: crate::daemon::ability::CallMode,
    ) -> bool {
        let Some(descriptor) = runtime.ability_descriptor(runtime_key).await else {
            return false;
        };
        match call_mode {
            crate::daemon::ability::CallMode::Rpc => descriptor.options.modes.rpc,
            crate::daemon::ability::CallMode::Stream => descriptor.options.modes.stream,
            crate::daemon::ability::CallMode::Bidi => descriptor.options.modes.bidi,
        }
    }

    fn record_registration_error(
        ability_name: &str,
        err: anyhow::Error,
        outcome: &mut HotAgentRuntimeSyncOutcome,
    ) {
        outcome.failed += 1;
        let err_msg = format!("{err}");
        crate::op_event!(
            component = axon_bridge,
            kind = hot_agent_register_failed,
            ability = ability_name,
            error = err_msg.as_str(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::daemon::persistence::agent_registry::{AgentEntry, AgentType};

    fn seed_hosted_agent(name: &str) -> String {
        let host_device_ura = crate::core::ura::device_ura("localhost", "dev");
        let agent_ura = crate::core::ura::agent_ura("localhost", "dev", name);
        crate::daemon::persistence::local_agents::save(
            &crate::daemon::persistence::local_agents::LocalAgentsFile {
                host_device_agent_ura: host_device_ura.clone(),
                hosted_agents: vec![crate::daemon::persistence::local_agents::HostedAgentEntry {
                    profile: "llm".to_string(),
                    name: name.to_string(),
                    agent_ura: agent_ura.clone(),
                    signing_authority: format!("hosted_by:{host_device_ura}"),
                    first_seen_at: "2026-06-24T00:00:00Z".to_string(),
                }],
            },
        )
        .expect("seed local-agents.json");
        agent_ura
    }

    fn runtime_key(agent: &str, registry_ability: &str) -> String {
        let agent_ura = crate::core::ura::agent_ura("localhost", "dev", agent);
        let public_name = crate::core::ura::owner_local_ability_name(&agent_ura, registry_ability);
        crate::core::ura::owner_ability_ura(&agent_ura, &public_name).expect("runtime key")
    }

    fn build_pending() -> Arc<HotAgentRegistrar> {
        HotAgentRegistrar::new_pending(
            Arc::new(Vec::new()),
            Arc::new(OnceLock::new()),
            Arc::new(crate::daemon::ability::builtins::agents::discover::BridgeDiscoverFederationResolver),
        )
    }

    fn wire_runtime_and_catalog(
        registrar: &Arc<HotAgentRegistrar>,
        runtime: Arc<LocalRuntime>,
    ) -> Arc<AxonAbilityCatalog> {
        registrar.set_runtime(Arc::clone(&runtime));
        let catalog = Arc::new(AxonAbilityCatalog::new_with_runtime(runtime));
        registrar
            .dispatch_handle
            .set(Arc::clone(&catalog))
            .expect("test catalog wired once");
        catalog
    }

    /// Reconcile pin: a `<agent>.*` row whose backing manifest is
    /// gone (registered by an earlier sync) must leave the runtime on
    /// the next `register_agent`, while the rows this sync owns stay.
    /// This is what lets a provider WITHDRAW an ability via
    /// `agent refresh` instead of a daemon restart.
    #[tokio::test]
    async fn register_agent_rejects_reserved_device_owner_token() {
        let registrar = build_pending();
        let rt = LocalRuntime::new();
        let _catalog = wire_runtime_and_catalog(&registrar, Arc::clone(&rt));
        let entry = AgentEntry::new(AgentType::ClaudeCode, None);

        // Dotted form — would mint rows that read as device-owned
        // ability shapes (`device.dev-1.sys.chat`).
        let outcome = registrar.register_agent("device.dev-1.sys", &entry).await;
        assert!(outcome.rejected_reserved_owner);
        assert_eq!(outcome.registered, 0);
        assert!(
            rt.list_abilities().await.is_empty(),
            "no device-owned-shaped rows may reach the runtime"
        );

        // Bare reserved token — would collide with the `device.*`
        // system ability namespace.
        let outcome = registrar.register_agent("device", &entry).await;
        assert!(outcome.rejected_reserved_owner);
        assert!(rt.list_abilities().await.is_empty());

        // User-owned shape passes the same gate untouched.
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_hosted_agent("liangbing");
        let outcome = registrar.register_agent("liangbing", &entry).await;
        assert!(!outcome.rejected_reserved_owner);
        assert!(
            rt.has_ability(&runtime_key("liangbing", "liangbing.chat"))
                .await
        );
    }

    #[tokio::test]
    async fn register_agent_reconciles_rows_without_backing_manifests() {
        let registrar = build_pending();
        let rt = LocalRuntime::new();
        let catalog = wire_runtime_and_catalog(&registrar, Arc::clone(&rt));
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_hosted_agent("liangbing");

        // Simulate an earlier sync's TOML ability whose manifest has
        // since been deleted: the row exists in the runtime but no
        // current source will re-register it.
        let ghost_key = runtime_key("liangbing", "liangbing.ghost_op");
        catalog
            .hot_register_rpc_with_spec(
                "liangbing.ghost_op",
                OwnerKind::Agent("liangbing".to_string()),
                crate::core::ability::spec::default_chat_manifest(),
                Arc::new(|_args| Ok(serde_json::Value::Null)),
            )
            .expect("seed dynamic ghost ability");
        assert!(rt.has_ability(&ghost_key).await);

        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        let outcome = registrar.register_agent("liangbing", &entry).await;

        assert_eq!(outcome.removed, 1, "stale row must be reconciled away");
        assert!(
            !rt.has_ability(&ghost_key).await,
            "withdrawn ability must leave the live runtime"
        );
        assert!(
            rt.has_ability(&runtime_key("liangbing", "liangbing.chat"))
                .await,
            "rows owned by this sync must survive the reconcile"
        );
    }

    #[tokio::test]
    async fn register_agent_lands_chat_discover_invoke_into_runtime_after_set_runtime() {
        // **Phase 5c invariant pin.**
        //
        // After `set_runtime`, calling `register_agent("liangbing", entry)`
        // MUST make `runtime.has_ability("liangbing.chat") == true` —
        // the load-bearing property the dispatcher's Phase-4 arm
        // (`runtime.invoke_remote`) and the host's session-receive
        // Axon arm (`LocalAxonSessionDispatcher`) both gate on.
        //
        // Pre-this-PR, `agent.start` only wrote `agents.json`
        // and the hot-added agent surfaced ONLY through the retired
        // lookup-miss catalog path. Chat worked, but every call went
        // through that path, never reaching the wired `LedgerSink` —
        // so `invocations.redb` stayed empty even on successful
        // chats. This test pins the fix at the
        // registrar layer; the boot-side wiring + lifecycle handler
        // wiring are tested separately.
        let registrar = build_pending();
        let rt = LocalRuntime::new();
        let _catalog = wire_runtime_and_catalog(&registrar, Arc::clone(&rt));
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_hosted_agent("liangbing");

        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        let outcome = registrar.register_agent("liangbing", &entry).await;

        assert!(!outcome.runtime_not_ready, "runtime IS ready post-set");
        assert!(
            outcome.registered >= 3,
            "chat/discover/invoke triple must land (plus any TOML), got {outcome:?}"
        );
        assert_eq!(outcome.failed, 0);
        assert_eq!(outcome.replaced, 0);

        // The load-bearing checks — these are what the Phase-4 arm
        // and the Phase-5d session-receive arm gate on.
        assert!(
            rt.has_ability(&runtime_key("liangbing", "liangbing.chat"))
                .await,
            "liangbing.chat MUST be in LocalRuntime after register_agent"
        );
        assert!(
            rt.has_ability(&runtime_key("liangbing", "liangbing.discover"))
                .await
        );
        assert!(
            rt.has_ability(&runtime_key("liangbing", "liangbing.invoke"))
                .await
        );
    }

    #[tokio::test]
    async fn register_agent_replaces_existing_runtime_rows_without_duplicate_failures() {
        // `agent set` and `agent.refresh` both call
        // `register_agent` for an agent that may already be live.
        // The runtime sync must replace those rows instead of
        // reporting duplicate-name failures and leaving old handler
        // closures in place.
        use easynet_axon::invocation::AbilityChangeEvent;

        let registrar = build_pending();
        let rt = LocalRuntime::new();
        let _catalog = wire_runtime_and_catalog(&registrar, Arc::clone(&rt));
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_hosted_agent("liangbing");

        let first_entry = AgentEntry::new(AgentType::ClaudeCode, Some("sonnet".to_string()));
        let first = registrar.register_agent("liangbing", &first_entry).await;
        assert!(first.registered >= 3, "initial register must land rows");
        assert_eq!(first.replaced, 0);
        assert_eq!(first.failed, 0);

        let mut changes = rt.subscribe_ability_changes();
        let second_entry = AgentEntry::new(AgentType::ClaudeCode, Some("opus".to_string()));
        let second = registrar.register_agent("liangbing", &second_entry).await;
        assert_eq!(
            second.registered, 0,
            "refreshing an existing agent should not create duplicate rows"
        );
        assert!(
            second.replaced >= 3,
            "chat/discover/invoke triple must be replaced, got {second:?}"
        );
        assert_eq!(
            second.failed, 0,
            "duplicate-name failures would leave stale handler closures live"
        );

        let expected_chat = runtime_key("liangbing", "liangbing.chat");
        let expected_discover = runtime_key("liangbing", "liangbing.discover");
        let expected_invoke = runtime_key("liangbing", "liangbing.invoke");
        let mut replaced = std::collections::BTreeSet::new();
        for _ in 0..16 {
            let Ok(event) =
                tokio::time::timeout(std::time::Duration::from_millis(100), changes.recv()).await
            else {
                break;
            };
            if let Ok(AbilityChangeEvent::Replaced { name, .. }) = event {
                replaced.insert(name);
            }
            if replaced.contains(&expected_chat)
                && replaced.contains(&expected_discover)
                && replaced.contains(&expected_invoke)
            {
                break;
            }
        }
        assert!(
            replaced.contains(&expected_chat)
                && replaced.contains(&expected_discover)
                && replaced.contains(&expected_invoke),
            "runtime must broadcast replacement for the canonical agent triple, got {replaced:?}"
        );
    }

    #[tokio::test]
    async fn register_agent_before_set_runtime_no_ops_with_runtime_not_ready_flag() {
        // Pre-`set_runtime` (i.e. during the brief boot window
        // between `build_registry_with_services` and
        // `start_daemon_invocation_transport`'s `set_runtime` call), the
        // registrar must NOT panic — it logs an op_event and
        // returns `runtime_not_ready: true`. The agent still lands
        // on disk via the lifecycle handler's prior `save_agents`,
        // so daemon-restart picks it up via the static-boot path.
        let registrar = build_pending();
        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        let outcome = registrar.register_agent("liangbing", &entry).await;
        assert!(outcome.runtime_not_ready);
        assert_eq!(outcome.registered, 0);
        assert_eq!(outcome.replaced, 0);
        assert_eq!(outcome.failed, 0);
    }

    #[tokio::test]
    async fn unregister_agent_removes_every_matching_hosted_agent_runtime_key() {
        // The reverse runtime-sync invariant: `agent.stop`
        // must wipe the `<name>.*` public set after decoding
        // LocalRuntime Ability URA keys back to owner-local public
        // names, so `runtime.has_ability` flips back to `false`.
        let registrar = build_pending();
        let rt = LocalRuntime::new();
        let _catalog = wire_runtime_and_catalog(&registrar, Arc::clone(&rt));
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_hosted_agent("liangbing");

        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        registrar.register_agent("liangbing", &entry).await;
        assert!(
            rt.has_ability(&runtime_key("liangbing", "liangbing.chat"))
                .await
        );

        let removed = registrar.unregister_agent("liangbing").await;
        assert!(
            removed >= 3,
            "chat/discover/invoke triple must be removed, got {removed}"
        );
        assert!(
            !rt.has_ability(&runtime_key("liangbing", "liangbing.chat"))
                .await
        );
        assert!(
            !rt.has_ability(&runtime_key("liangbing", "liangbing.discover"))
                .await
        );
        assert!(
            !rt.has_ability(&runtime_key("liangbing", "liangbing.invoke"))
                .await
        );
    }
}

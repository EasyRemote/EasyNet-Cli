//! Hot-add agent registration into `LocalRuntime`.
//!
//! Phase 5c blocker: `device.agent.start` / `device.agent.stop`
//! currently only mutate `agents.json` on disk. The new agent's
//! `<agent>.chat / discover / invoke` handlers are NOT registered
//! anywhere — they only resolve via `AxonAbilityCatalog.rpc_fallback`
//! at lookup time. Once Phase 5c retires `AxonAbilityCatalog`, the
//! fallback resolver is gone too and every hot-added agent
//! immediately fails to dispatch.
//!
//! This registrar closes the gap. `device.agent.start` calls
//! `register_agent(name, entry)` which builds the same three
//! handler closures `chat_ability::register_dynamic_agent_fallback`
//! would have synthesised lazily, wraps each through
//! [`crate::runtime::ability_dispatch::rpc_handler_to_ability_fn`],
//! and inserts them into [`LocalRuntime`] under the canonical
//! `<agent>.<verb>` names. `device.agent.stop` calls
//! `unregister_agent(name)` which uses `unregister_ability_by_prefix`
//! to wipe the `<agent>.*` set in one atomic call.
//!
//! ## Boot-order rationale
//!
//! The `AxonAbilityCatalog` is constructed *before* the Axon
//! `LocalRuntime` (registry comes from
//! `runtime::agents::build_registry_with_services` in the daemon's
//! Stage 2; runtime comes later in `axon_serve::start_..._sidecar`).
//! But `device.agent.start`'s handler closure has to be installed at
//! registry-build time. We bridge that by parking the
//! [`LocalRuntime`] handle in an internal [`OnceLock`]: the
//! registrar is constructed *pending* at registry-build time, the
//! handler closure captures `Arc<Self>`, and boot calls
//! [`HotAgentRegistrar::set_runtime`] once the runtime is ready.
//! Any `register_agent` call that lands before the runtime is wired
//! returns `runtime_not_ready` and emits a diagnostic op_event —
//! the disk-side `agents.json` write still succeeds, so the next
//! daemon restart picks up the agent through the boot-time
//! registration loop (`for agent_name in agents.agents.keys()` in
//! `build_registry_with_services`).

use std::sync::{Arc, OnceLock};

use easynet_axon::invocation::{AbilityOptions, LocalRuntime};

use crate::registry::agents::AgentEntry;
use crate::runtime::ability_dispatch::{rpc_handler_to_ability_fn, AxonAbilityCatalog};
use crate::runtime::agents::chat_ability::{
    build_agent_ability_handler, build_chat_handler_for, build_discover_handler_for,
    build_invoke_handler_for, ContextLoader,
};

/// Captures every dependency a hot-add path needs to synthesise an
/// agent's static handler set + register it into `LocalRuntime`.
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
    /// Same `dispatch_handle` the chat-fallback resolver was given;
    /// the discover + invoke handlers re-enter local dispatch
    /// through it to resolve peer-agent ability descriptors.
    dispatch_handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>>,
}

/// Outcome reported back to the caller of `register_agent` /
/// `unregister_agent` so the lifecycle handler's op_event line can
/// surface how many `<agent>.*` rows actually landed.
#[derive(Debug, Default, Clone, Copy)]
pub struct HotAgentRuntimeSyncOutcome {
    pub registered: usize,
    pub replaced: usize,
    pub failed: usize,
    /// True when the call landed before `set_runtime` was called.
    /// Distinguishes "deliberate no-op due to boot ordering" from
    /// "every register attempt errored". The disk side still wrote
    /// `agents.json`, so the agent comes up on daemon restart.
    pub runtime_not_ready: bool,
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
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime: OnceLock::new(),
            loaders,
            dispatch_handle,
        })
    }

    /// Attach the live `LocalRuntime`. Idempotent on the first
    /// successful set; subsequent calls are ignored (`OnceLock`
    /// semantics). Called from `boot.rs` exactly once, after
    /// `build_local_runtime`.
    pub fn set_runtime(&self, runtime: Arc<LocalRuntime>) {
        let _ = self.runtime.set(runtime);
    }

    /// Register the canonical `<agent>.chat / discover / invoke`
    /// triple plus every TOML-declared `<agent>.<verb>` for `name`
    /// into [`LocalRuntime`].
    ///
    /// **Replace-capable.** Existing rows are overwritten through
    /// `LocalRuntime::replace_ability`, not ignored as duplicates.
    /// This is required for `agent set` and `device.agent.refresh`:
    /// both update `agents.json` first and then call this registrar
    /// against names that may already be live in the runtime. The
    /// runtime must therefore swap the handler closure atomically so
    /// subsequent invokes observe the updated `AgentEntry` and TOML
    /// ability set without requiring a daemon restart.
    pub async fn register_agent(
        &self,
        name: &str,
        entry: &AgentEntry,
    ) -> HotAgentRuntimeSyncOutcome {
        let Some(runtime) = self.runtime.get() else {
            crate::op_event!(
                component = axon_bridge,
                kind = hot_agent_register_runtime_not_ready,
                agent = name,
                message = "device.agent.start landed before LocalRuntime was wired; \
                          agents.json still written, agent comes up on daemon restart",
            );
            return HotAgentRuntimeSyncOutcome {
                runtime_not_ready: true,
                ..Default::default()
            };
        };

        let mut outcome = HotAgentRuntimeSyncOutcome::default();

        // ── chat
        let chat_handler =
            build_chat_handler_for(name.to_string(), entry.clone(), Arc::clone(&self.loaders));
        Self::try_replace(runtime, &format!("{name}.chat"), chat_handler, &mut outcome).await;

        // ── discover
        let discover_handler =
            build_discover_handler_for(name.to_string(), Arc::clone(&self.dispatch_handle));
        Self::try_replace(
            runtime,
            &format!("{name}.discover"),
            discover_handler,
            &mut outcome,
        )
        .await;

        // ── invoke
        let invoke_handler =
            build_invoke_handler_for(name.to_string(), Arc::clone(&self.dispatch_handle));
        Self::try_replace(
            runtime,
            &format!("{name}.invoke"),
            invoke_handler,
            &mut outcome,
        )
        .await;

        // ── TOML-declared abilities (same chat-translation path
        // the boot-time `register_for_agent` loop uses).
        let chat_name = format!("{name}.chat");
        for spec in crate::runtime::abilities::abilities_for(name, entry) {
            let ability_name = spec.name().to_string();
            if ability_name == chat_name {
                continue;
            }
            let bare = ability_name
                .strip_prefix(&format!("{name}."))
                .unwrap_or(&ability_name)
                .to_string();
            let h = build_agent_ability_handler(
                name.to_string(),
                entry.clone(),
                Arc::clone(&self.loaders),
                bare,
            );
            Self::try_replace(runtime, &ability_name, h, &mut outcome).await;
        }

        outcome
    }

    /// Unregister every `<name>.*` ability from `LocalRuntime`.
    /// Returns the count of rows actually removed.
    ///
    /// Uses `unregister_ability_by_prefix` so a single lock-cycle
    /// drops the whole set atomically + fires one
    /// `AbilityChangeEvent::Unregistered` per removed row, in
    /// sorted order. The trailing dot in the prefix prevents
    /// accidentally taking out a sibling agent whose name happens
    /// to share a non-dot prefix (e.g. "alice" vs "alice-2").
    ///
    /// No-op (returns 0) when the runtime is not yet wired —
    /// the disk-side `agents.json` row is already gone, so the
    /// next daemon restart won't bring the agent back.
    pub async fn unregister_agent(&self, name: &str) -> usize {
        let Some(runtime) = self.runtime.get() else {
            return 0;
        };
        match runtime
            .unregister_ability_by_prefix(&format!("{name}."))
            .await
        {
            Ok(removed) => removed.len(),
            // Empty-prefix is the only error shape the runtime can
            // return; we always pass `"<name>."` which is non-empty.
            Err(_) => 0,
        }
    }

    async fn try_replace(
        runtime: &Arc<LocalRuntime>,
        ability_name: &str,
        handler: crate::runtime::ability_dispatch::LocalRpcHandler,
        outcome: &mut HotAgentRuntimeSyncOutcome,
    ) {
        let ability_fn = rpc_handler_to_ability_fn(handler);
        match runtime
            .replace_ability(
                ability_name.to_string(),
                ability_fn,
                AbilityOptions::default(),
            )
            .await
        {
            Ok(Some(_)) => outcome.replaced += 1,
            Ok(None) => outcome.registered += 1,
            Err(err) => {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::registry::agents::{AgentEntry, AgentType};

    fn build_pending() -> Arc<HotAgentRegistrar> {
        HotAgentRegistrar::new_pending(Arc::new(Vec::new()), Arc::new(OnceLock::new()))
    }

    #[tokio::test]
    async fn register_agent_lands_chat_discover_invoke_into_runtime_after_set_runtime() {
        // **Phase 5c invariant pin.**
        //
        // After `set_runtime`, calling `register_agent("liangbing", entry)`
        // MUST make `runtime.has_ability("liangbing.chat") == true` —
        // the load-bearing property the dispatcher's Phase-4 arm
        // (`<self>.invoke_remote`) and the host's session-receive
        // Axon arm (`LocalAxonSessionDispatcher`) both gate on.
        //
        // Pre-this-PR, `device.agent.start` only wrote `agents.json`
        // and the hot-added agent surfaced ONLY through the legacy
        // `rpc_fallback` resolver. Chat worked, but every call went
        // through the legacy path, never reaching the wired
        // `LedgerSink` — so `invocations.redb` stayed empty even on
        // successful chats. This test pins the fix at the
        // registrar layer; the boot-side wiring + lifecycle handler
        // wiring are tested separately.
        let registrar = build_pending();
        let rt = LocalRuntime::new();
        registrar.set_runtime(Arc::clone(&rt));

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
            rt.has_ability("liangbing.chat").await,
            "liangbing.chat MUST be in LocalRuntime after register_agent"
        );
        assert!(rt.has_ability("liangbing.discover").await);
        assert!(rt.has_ability("liangbing.invoke").await);
    }

    #[tokio::test]
    async fn register_agent_replaces_existing_runtime_rows_without_duplicate_failures() {
        // `agent set` and `device.agent.refresh` both call
        // `register_agent` for an agent that may already be live.
        // The runtime sync must replace those rows instead of
        // reporting duplicate-name failures and leaving old handler
        // closures in place.
        use easynet_axon::invocation::AbilityChangeEvent;

        let registrar = build_pending();
        let rt = LocalRuntime::new();
        registrar.set_runtime(Arc::clone(&rt));

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
            if replaced.contains("liangbing.chat")
                && replaced.contains("liangbing.discover")
                && replaced.contains("liangbing.invoke")
            {
                break;
            }
        }
        assert!(
            replaced.contains("liangbing.chat")
                && replaced.contains("liangbing.discover")
                && replaced.contains("liangbing.invoke"),
            "runtime must broadcast replacement for the canonical agent triple, got {replaced:?}"
        );
    }

    #[tokio::test]
    async fn register_agent_before_set_runtime_no_ops_with_runtime_not_ready_flag() {
        // Pre-`set_runtime` (i.e. during the brief boot window
        // between `build_registry_with_services` and
        // `start_axon_serve_sidecar`'s `set_runtime` call), the
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
    async fn unregister_agent_removes_every_prefix_match_atomically() {
        // The reverse runtime-sync invariant: `device.agent.stop`
        // must wipe the `<name>.*` set so `runtime.has_ability`
        // flips back to `false`. Uses `unregister_ability_by_prefix`
        // so the whole set drops in one atomic lock cycle.
        let registrar = build_pending();
        let rt = LocalRuntime::new();
        registrar.set_runtime(Arc::clone(&rt));

        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        registrar.register_agent("liangbing", &entry).await;
        assert!(rt.has_ability("liangbing.chat").await);

        let removed = registrar.unregister_agent("liangbing").await;
        assert!(
            removed >= 3,
            "chat/discover/invoke triple must be removed, got {removed}"
        );
        assert!(!rt.has_ability("liangbing.chat").await);
        assert!(!rt.has_ability("liangbing.discover").await);
        assert!(!rt.has_ability("liangbing.invoke").await);
    }
}

// EasyNet CLI — Runtime-dispatch Ability Adapter
// ==============================================
//
// File: src/daemon/control/runtime_dispatch_adapter.rs
// Description: Daemon-internal adapter from the runtime-dispatch UDS
//              protocol to the daemon-hosted Axon `LocalRuntime`.
//
// Boundary
// --------
// This module is NOT a public `control.sock` ability surface. Public
// product calls use daemon `Invocation` over `daemon.sock`; the JSON
// control socket is boot/status only. This adapter exists because
// Axon local-tool dispatch still needs a compact newline-delimited
// bridge that can invoke handlers registered inside the daemon's
// embedded `LocalRuntime`.
//
// Invariants
// ----------
// 1. No `IncomingFrame` / `OutgoingFrame` JSON-control product frame is
//    constructed or interpreted here.
// 2. Invocation canonicalization, admission, receipts, and protocol
//    stream/bidi semantics remain Axon-owned. This adapter only builds
//    a daemon-local `InvocationPlan` and delegates execution. When
//    Axon supplies envelope context, the adapter carries it into the
//    plan; it never recovers tuple fields from JSON args.
// 3. The adapter holds only the runtime and resolver it needs. Kernel,
//    receipt-header, subscription, and bidi-session state are not part
//    of runtime-dispatch ownership.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use easynet_axon::invocation::{LocalRuntime, StreamingInvocationHandle};

use crate::core::domain::NodeId;
use crate::daemon::invocation::dispatch::local_runtime_invoker::{
    invoke_local_rpc_sync, open_local_stream,
};
#[cfg(test)]
use crate::daemon::invocation::routing::target::LocalNodeResolver;
use crate::daemon::invocation::routing::target::{CallMode, InvocationPlan, TargetResolver};
use crate::support::async_bridge::{run_blocking, NoRuntimeFallback};

/// Daemon-internal runtime-dispatch adapter.
///
/// It is deliberately small: runtime-dispatch speaks a separate
/// newline-delimited JSON protocol, so routing through the retired
/// control-frame schema would reintroduce the product JSON surface
/// Step 6 is removing.
#[derive(Clone)]
pub struct RuntimeDispatchAdapter {
    local_runtime: Arc<LocalRuntime>,
    resolver: Arc<dyn TargetResolver>,
}

impl RuntimeDispatchAdapter {
    /// Construct an adapter over the daemon's already-built
    /// `LocalRuntime`.
    ///
    /// Production daemon boot should use this constructor so the
    /// runtime-dispatch path observes the exact same handlers as the
    /// daemon Invocation transport.
    pub fn new_with_runtime(
        local_runtime: Arc<LocalRuntime>,
        resolver: Arc<dyn TargetResolver>,
    ) -> Self {
        Self {
            local_runtime,
            resolver,
        }
    }

    /// Test/helper constructor with the live system ability registry.
    ///
    /// This is not used by daemon boot. It exists for runtime-dispatch
    /// wire tests that need an always-available local ability such as
    /// `observe.health` without constructing the whole daemon.
    #[cfg(test)]
    pub fn new_for_test() -> Self {
        let agents = crate::daemon::persistence::agent_registry::AgentRegistry::default();
        let local_runtime = LocalRuntime::new();
        let mut config = crate::daemon::ability::catalog::RegistryBuildConfig::new(
            crate::daemon::ability::catalog::RegistryBuildServices::fresh(),
            &agents,
        );
        config.local_runtime = Some(Arc::clone(&local_runtime));
        config.authority_context = Some(
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
                crate::daemon::identity::local_invocation::local_device_ura(),
            )
            .expect("local device URA is a valid device authority root"),
        );
        let _registry = crate::daemon::ability::catalog::build_registry_with_services(config);
        let resolver: Arc<dyn TargetResolver> =
            Arc::new(LocalNodeResolver::new(node_id_from_env_or_default()));
        Self::new_with_runtime(local_runtime, resolver)
    }

    /// Execute one runtime-dispatch RPC request.
    ///
    /// The caller provides the already-parsed tool name and JSON
    /// arguments. The returned value is the raw ability result used by
    /// `runtime_dispatch.rs` to build its newline-delimited response.
    pub fn execute_runtime_dispatch(
        &self,
        ability: &str,
        args: serde_json::Value,
        subject: Option<String>,
    ) -> Result<serde_json::Value, String> {
        let target = self.resolve(ability, args, CallMode::Rpc, subject)?;
        invoke_local_rpc_sync(Arc::clone(&self.local_runtime), target)
    }

    /// Execute one runtime-dispatch stream request.
    ///
    /// The live Axon streaming handle is returned to
    /// `runtime_dispatch.rs`, which owns wire-level backpressure and
    /// line framing for this internal protocol.
    pub fn execute_runtime_dispatch_stream(
        &self,
        ability: &str,
        args: serde_json::Value,
        subject: Option<String>,
    ) -> Result<StreamingInvocationHandle, String> {
        let target = self.resolve(ability, args, CallMode::Stream, subject)?;
        run_blocking(
            open_local_stream(Arc::clone(&self.local_runtime), target),
            NoRuntimeFallback::BuildCurrentThreadTokio,
        )
    }

    fn resolve(
        &self,
        ability: &str,
        args: serde_json::Value,
        call_mode: CallMode,
        subject: Option<String>,
    ) -> Result<crate::daemon::invocation::routing::target::InvocationTarget, String> {
        let plan = InvocationPlan {
            ability: ability.to_string(),
            target_node_hint: extract_node_hint(&args),
            args,
            call_mode,
            subject,
        };
        self.resolver
            .resolve(plan)
            .map_err(|e| format!("resolver: {e}"))
    }
}

/// Read a `node` field out of the runtime-dispatch args object if
/// present. The field is a daemon-local routing hint, not an Axon
/// Invocation tuple field.
fn extract_node_hint(args: &serde_json::Value) -> Option<NodeId> {
    args.get("node")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(NodeId::new)
}

#[cfg(test)]
fn node_id_from_env_or_default() -> NodeId {
    std::env::var("EASYNET_NODE_ID")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .map(NodeId::new)
        .unwrap_or_else(|| NodeId::new("self"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_dispatch_rpc_observe_health_returns_json_object() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let adapter = RuntimeDispatchAdapter::new_for_test();
        let value = adapter
            .execute_runtime_dispatch("observe.health", serde_json::json!({}), None)
            .expect("observe.health dispatch should succeed");
        assert!(value.is_object(), "observe.health returns a JSON object");
    }

    #[test]
    fn runtime_dispatch_unknown_ability_returns_not_found_text() {
        let adapter = RuntimeDispatchAdapter::new_for_test();
        let err = adapter
            .execute_runtime_dispatch("nope.does_not_exist", serde_json::json!({}), None)
            .expect_err("unknown ability must fail");
        assert!(
            crate::daemon::invocation::dispatch::local_runtime_invoker::is_not_found_error(&err),
            "error should classify as not_found; got {err:?}"
        );
    }

    #[test]
    fn node_hint_ignores_missing_empty_and_non_string_values() {
        assert!(extract_node_hint(&serde_json::json!({})).is_none());
        assert!(extract_node_hint(&serde_json::json!({"node": ""})).is_none());
        assert!(extract_node_hint(&serde_json::json!({"node": 7})).is_none());
    }

    #[test]
    fn node_hint_extracts_string_node_id() {
        let node = extract_node_hint(&serde_json::json!({"node": "edge-a"}))
            .expect("node hint should parse");
        assert_eq!(node.as_str(), "edge-a");
    }

    struct EchoResolver;

    impl TargetResolver for EchoResolver {
        fn resolve(
            &self,
            plan: InvocationPlan,
        ) -> anyhow::Result<crate::daemon::invocation::routing::target::InvocationTarget> {
            Ok(
                crate::daemon::invocation::routing::target::InvocationTarget {
                    scope: crate::daemon::invocation::routing::target::TargetScope::Local,
                    ability: plan.ability,
                    normalized_args: plan.args,
                    call_mode: plan.call_mode,
                    subject: plan.subject,
                    causal_context: None,
                    request_metadata: std::collections::HashMap::new(),
                },
            )
        }
    }

    #[test]
    fn resolve_preserves_envelope_subject_from_runtime_dispatch() {
        let adapter =
            RuntimeDispatchAdapter::new_with_runtime(LocalRuntime::new(), Arc::new(EchoResolver));
        let target = adapter
            .resolve(
                "observe.health",
                serde_json::json!({}),
                CallMode::Rpc,
                Some("easynet:///r/test/resource/device".to_string()),
            )
            .expect("resolver should accept subject");
        assert_eq!(
            target.subject.as_deref(),
            Some("easynet:///r/test/resource/device")
        );
    }
}

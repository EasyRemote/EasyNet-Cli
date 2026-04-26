//! mcp profile — RFC-001 §1 [P6].
//!
//! Per restatement-mapping decision P6: a single mcp-profile Agent
//! owns BOTH inbound and outbound MCP — `mcp.bridge.*` (incoming MCP
//! tools/list + tools/call) and `mcp.client.*` (outgoing MCP calls
//! to external servers). They share one Agent identity rather than
//! splitting into two profiles.
//!
//! This is the ONLY place MCP awareness is permitted in the CLI per
//! RFC-001 §A3 (MCP only at edge adapters; everywhere else is
//! Invocation-only). The conformance script enforces this.
//!
//! Owned ability namespaces
//! ------------------------
//!   mcp.bridge.list_tools  (inbound MCP server: tools/list)
//!   mcp.bridge.call_tool   (inbound MCP server: tools/call)
//!   mcp.client.list        (outbound: list configured external MCP servers)
//!   mcp.client.call        (outbound: dispatch to external MCP server)
//!
//! What this file provides today
//! -----------------------------
//!   - `owns(ability_name)`            : prefix check
//!   - `descriptors_for(owner_uri)`    : §1.6 descriptors emitter
//!   - `InvokeMcpProvider`             : the McpToolProvider impl that
//!     translates every `tools/list` and `tools/call` into in-process
//!     Invoke against the AbilityProxy. Replaces the legacy
//!     `facade::mcp::HubMcpProvider` that owned a duplicate tool
//!     catalog and called deleted bridge methods directly.
//!   - `tool_specs_from_descriptors(...)` : projects AbilityDescriptors
//!     into the JSON-Schema shape MCP clients expect.
//!
//! Quarantine policy (P4.8d)
//! -------------------------
//! Every MCP client touchpoint MUST flow through this module. The
//! conformance script accepts `facade/mcp/` only when its files
//! import from `runtime::agents::profiles::mcp` and contain no
//! independent tool registry of their own. The legacy `specs.rs`
//! catalog and `handlers.rs` dispatchers are deleted; what remains
//! is the stdio MCP server scaffolding (provider trait wiring,
//! bound-node patching), which P4.9 absorbs into this file.

pub const MCP_PROFILE_ABILITY_PREFIXES: &[&str] = &["mcp.bridge.", "mcp.client."];

pub fn owns(ability_name: &str) -> bool {
    MCP_PROFILE_ABILITY_PREFIXES
        .iter()
        .any(|p| ability_name.starts_with(p))
}

/// AbilityDescriptors for every mcp.bridge.* + mcp.client.* in the
/// live registry, anchored to the mcp-profile's canonical URA. All
/// SCOPED per §18 — local MCP clients only for bridge.*; the daemon
/// itself + selected internal callers for client.*. P4.7 narrows.
pub fn descriptors_for(
    owner_agent_uri: &str,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
    crate::runtime::agents::published_abilities()
        .into_iter()
        .filter(|m| owns(&m.name))
        .map(|m| {
            AbilityDescriptor::new(m.name.clone(), owner_agent_uri, Visibility::Scoped)
                .expect("registry-derived names satisfy descriptor invariants")
                .with_input_schema(m.input_schema.clone())
                .with_source("kernel:built-in")
        })
        .collect()
}

/// Project an AbilityDescriptor into the JSON-Schema shape MCP
/// clients expect from `tools/list`. The MCP wire format requires
/// `{name, description, inputSchema}`; we map:
///
///   name         ← descriptor.name
///   description  ← first non-empty source / metadata hint, falling
///                  back to a generic "<namespace> ability" string
///   inputSchema  ← descriptor.schema_summary.input (JSON Schema)
pub fn tool_spec_from_descriptor(
    descriptor: &crate::runtime::ability_descriptor::AbilityDescriptor,
) -> serde_json::Value {
    let description = if !descriptor.source.is_empty() {
        format!("{} (source: {})", descriptor.name, descriptor.source)
    } else {
        descriptor.name.clone()
    };
    let input_schema = if descriptor.schema_summary.input.is_null() {
        serde_json::json!({"type": "object"})
    } else {
        descriptor.schema_summary.input.clone()
    };
    serde_json::json!({
        "name": descriptor.name,
        "description": description,
        "inputSchema": input_schema,
    })
}

pub fn tool_specs_from_descriptors(
    descriptors: &[crate::runtime::ability_descriptor::AbilityDescriptor],
) -> Vec<serde_json::Value> {
    descriptors.iter().map(tool_spec_from_descriptor).collect()
}

/// The trait we need from any AbilityProxy-shaped dispatcher. Lets
/// tests inject a fake without depending on the full proxy
/// construction graph (kernel, gateway, resolver, …).
pub trait LocalInvoker {
    /// Invoke the named ability synchronously and return the raw
    /// Result frame's value, or an error message.
    fn invoke_sync(
        &self,
        ability: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

/// Production adapter: drives `AbilityProxy::handle` and decodes
/// the resulting frame queue into `Result<Value, String>`. The MCP
/// provider doesn't see frames, only the value or an error string —
/// every legacy facade/mcp handler boiled down to the same shape.
pub struct ProxyLocalInvoker {
    proxy: std::sync::Arc<crate::services::control::ability_proxy::AbilityProxy>,
    next_request_id: std::sync::atomic::AtomicU64,
}

impl ProxyLocalInvoker {
    pub fn new(
        proxy: std::sync::Arc<crate::services::control::ability_proxy::AbilityProxy>,
    ) -> Self {
        Self {
            proxy,
            next_request_id: std::sync::atomic::AtomicU64::new(1),
        }
    }
}

impl LocalInvoker for ProxyLocalInvoker {
    fn invoke_sync(
        &self,
        ability: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        use crate::services::control::frames::{IncomingFrame, OutgoingFrame};
        let req_id = format!(
            "mcp-{}",
            self.next_request_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let frames = self.proxy.handle(IncomingFrame::Invoke {
            request_id: req_id,
            ability: ability.to_string(),
            args,
        });
        // Per the proxy contract, the first frame is either a
        // single Result or a single Error. We surface the Result's
        // value verbatim; an Error becomes the Err string. Receipt
        // headers (P4.8c) are dropped here because the MCP wire
        // shape has no header field; receivers that need them must
        // use the IPC path directly, not the MCP shim.
        match frames.into_iter().next() {
            Some(OutgoingFrame::Result { value, .. }) => Ok(value),
            Some(OutgoingFrame::Error { code, message, .. }) => {
                Err(format!("{code}: {message}"))
            }
            Some(other) => Err(format!("unexpected frame from proxy: {other:?}")),
            None => Err("proxy returned no frames".into()),
        }
    }
}

/// Production InvokeMcpProvider — what `easynet mcp_server` and
/// `easynet start --mcp` will use after P4.8d's quarantine. Every
/// `tools/list` returns the host's AbilityDescriptors projected to
/// MCP shape; every `tools/call` routes through the in-process
/// AbilityProxy via `LocalInvoker::invoke_sync`. Zero direct bridge
/// calls; zero hub-mediated MCP tool catalog.
pub struct InvokeMcpProvider<I: LocalInvoker> {
    invoker: I,
    /// Snapshot of the host's ability descriptors at construction.
    /// Refreshed on daemon restart; for now we keep a static list
    /// because the registry doesn't change at runtime.
    descriptors: Vec<crate::runtime::ability_descriptor::AbilityDescriptor>,
}

impl<I: LocalInvoker> InvokeMcpProvider<I> {
    pub fn new(
        invoker: I,
        descriptors: Vec<crate::runtime::ability_descriptor::AbilityDescriptor>,
    ) -> Self {
        Self {
            invoker,
            descriptors,
        }
    }

    /// Number of descriptors the provider will surface in tools/list.
    /// Used by tests + by `easynet mcp_server`'s startup banner.
    pub fn descriptor_count(&self) -> usize {
        self.descriptors.len()
    }
}

impl<I: LocalInvoker> easynet_axon::mcp::McpToolProvider for InvokeMcpProvider<I> {
    fn tool_specs(&self) -> Vec<serde_json::Value> {
        tool_specs_from_descriptors(&self.descriptors)
    }

    fn handle_tool_call(
        &self,
        name: &str,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> easynet_axon::mcp::ToolResult {
        // Reject calls for tools we don't advertise. The descriptor
        // list is the single source of truth — if a name isn't
        // there, this is a caller-side bug, not a transient.
        if !self.descriptors.iter().any(|d| d.name == name) {
            return easynet_axon::mcp::ToolResult {
                payload: serde_json::json!({
                    "error": format!("unknown tool: `{name}`")
                }),
                is_error: true,
            };
        }
        let args_value = serde_json::Value::Object(args.clone());
        match self.invoker.invoke_sync(name, args_value) {
            Ok(value) => easynet_axon::mcp::ToolResult {
                payload: value,
                is_error: false,
            },
            Err(msg) => easynet_axon::mcp::ToolResult {
                payload: serde_json::json!({"error": msg}),
                is_error: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
    use easynet_axon::mcp::McpToolProvider;
    use std::cell::RefCell;

    #[test]
    fn owns_recognizes_both_mcp_namespaces() {
        assert!(owns("mcp.bridge.list_tools"));
        assert!(owns("mcp.bridge.call_tool"));
        assert!(owns("mcp.client.list"));
        assert!(owns("mcp.client.call"));
    }

    #[test]
    fn owns_rejects_other_profiles_and_bare_mcp() {
        assert!(!owns("mcp.evaluate")); // not in either bridge/client subset
        assert!(!owns("fleet.list_abilities"));
        assert!(!owns("consent.subscribe"));
    }

    fn d(name: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(name, "easynet:///r/acme/agent/01DEV", Visibility::Scoped)
            .unwrap()
            .with_source("kernel:built-in")
            .with_input_schema(serde_json::json!({"type":"object"}))
    }

    #[test]
    fn tool_spec_from_descriptor_emits_mcp_shape() {
        let spec = tool_spec_from_descriptor(&d("fleet.list_agents"));
        assert_eq!(spec["name"], "fleet.list_agents");
        assert!(spec["description"].as_str().unwrap().contains("kernel:built-in"));
        assert_eq!(spec["inputSchema"]["type"], "object");
    }

    #[test]
    fn tool_spec_falls_back_to_object_schema_when_input_is_null() {
        let mut desc =
            AbilityDescriptor::new("a.b", "u", Visibility::Public).unwrap();
        desc.schema_summary.input = serde_json::Value::Null;
        let spec = tool_spec_from_descriptor(&desc);
        assert_eq!(spec["inputSchema"]["type"], "object");
    }

    /// Recording fake invoker that asserts the proxy contract: the
    /// MCP provider must hand the raw ability_name + args bag to the
    /// dispatcher and surface its result verbatim (or its error).
    struct RecordingInvoker {
        last_ability: RefCell<Option<String>>,
        last_args: RefCell<Option<serde_json::Value>>,
        reply: Result<serde_json::Value, String>,
    }
    impl RecordingInvoker {
        fn new(reply: Result<serde_json::Value, String>) -> Self {
            Self {
                last_ability: RefCell::new(None),
                last_args: RefCell::new(None),
                reply,
            }
        }
    }
    impl LocalInvoker for RecordingInvoker {
        fn invoke_sync(
            &self,
            ability: &str,
            args: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            *self.last_ability.borrow_mut() = Some(ability.to_string());
            *self.last_args.borrow_mut() = Some(args);
            self.reply.clone()
        }
    }

    #[test]
    fn tool_specs_lists_every_descriptor_passed_at_construction() {
        let descs = vec![d("observe.health"), d("fleet.list_agents")];
        let p = InvokeMcpProvider::new(
            RecordingInvoker::new(Ok(serde_json::json!({}))),
            descs,
        );
        let specs = p.tool_specs();
        assert_eq!(specs.len(), 2);
        let names: Vec<&str> = specs.iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"observe.health"));
        assert!(names.contains(&"fleet.list_agents"));
    }

    #[test]
    fn unknown_tool_call_returns_error_result_without_invoking() {
        let invoker = RecordingInvoker::new(Ok(serde_json::json!({})));
        let p = InvokeMcpProvider::new(invoker, vec![d("observe.health")]);
        let result = p.handle_tool_call("totally.unknown", &serde_json::Map::new());
        assert!(result.is_error, "unknown tool must surface as is_error=true");
        // Crucially: the invoker MUST NOT have been called.
        assert!(p.invoker.last_ability.borrow().is_none());
    }

    #[test]
    fn known_tool_call_is_dispatched_via_invoker() {
        let invoker =
            RecordingInvoker::new(Ok(serde_json::json!({"status": "healthy"})));
        let p = InvokeMcpProvider::new(invoker, vec![d("observe.health")]);
        let mut args = serde_json::Map::new();
        args.insert("foo".into(), serde_json::Value::Bool(true));
        let result = p.handle_tool_call("observe.health", &args);
        assert!(!result.is_error);
        assert_eq!(result.payload["status"], "healthy");
        assert_eq!(
            p.invoker.last_ability.borrow().as_deref(),
            Some("observe.health")
        );
        assert_eq!(
            p.invoker.last_args.borrow().as_ref().unwrap()["foo"],
            serde_json::Value::Bool(true)
        );
    }

    #[test]
    fn invoker_error_surfaces_as_error_payload() {
        let invoker = RecordingInvoker::new(Err("policy denied".into()));
        let p = InvokeMcpProvider::new(invoker, vec![d("observe.health")]);
        let result = p.handle_tool_call("observe.health", &serde_json::Map::new());
        assert!(result.is_error);
        assert!(result.payload["error"].as_str().unwrap().contains("policy denied"));
    }

    #[test]
    fn descriptor_count_matches_input() {
        let invoker = RecordingInvoker::new(Ok(serde_json::json!({})));
        let p =
            InvokeMcpProvider::new(invoker, vec![d("observe.health"), d("fleet.list_agents")]);
        assert_eq!(p.descriptor_count(), 2);
    }

    /// End-to-end: build a real ProxyLocalInvoker over the live
    /// AbilityProxy and call `observe.health` through it. Pins the
    /// quarantine contract — the MCP shim MUST go through the same
    /// AbilityProxy the IPC server uses, never around it.
    #[test]
    fn proxy_local_invoker_dispatches_observe_health_through_real_proxy() {
        use crate::runtime::ability_dispatch::AbilityDispatcher;
        use crate::runtime::gateway::NoopGateway;
        use crate::runtime::invocation_target::LocalNodeResolver;
        use crate::runtime::kernel::Kernel;
        use crate::services::control::ability_proxy::AbilityProxy;
        use std::sync::Arc;

        let gateway: Arc<dyn crate::runtime::gateway_api::GatewayApi> =
            Arc::new(NoopGateway::new());
        let kernel: Arc<dyn crate::runtime::kernel_api::KernelApi> =
            Arc::new(Kernel::new(Arc::clone(&gateway)));
        let registry = crate::runtime::agents::build_registry();
        let dispatcher = AbilityDispatcher::new(registry, gateway);
        let resolver: Arc<dyn crate::runtime::invocation_target::TargetResolver> =
            Arc::new(LocalNodeResolver::new(crate::runtime::domain::NodeId::new("self")));
        let proxy = Arc::new(AbilityProxy::new_with_dispatcher(
            kernel,
            dispatcher,
            resolver,
        ));
        let invoker = ProxyLocalInvoker::new(proxy);
        let result = invoker
            .invoke_sync("observe.health", serde_json::json!({}))
            .expect("observe.health must dispatch successfully");
        // observe.health returns a JSON object with at least
        // `status`. Exact shape is owned by ping.rs; we only assert
        // the dispatch happened (got a non-null Value).
        assert!(!result.is_null(), "observe.health must return a value, not null");
    }

    #[test]
    fn proxy_local_invoker_surfaces_unknown_ability_as_error() {
        use crate::runtime::ability_dispatch::AbilityDispatcher;
        use crate::runtime::gateway::NoopGateway;
        use crate::runtime::invocation_target::LocalNodeResolver;
        use crate::runtime::kernel::Kernel;
        use crate::services::control::ability_proxy::AbilityProxy;
        use std::sync::Arc;

        let gateway: Arc<dyn crate::runtime::gateway_api::GatewayApi> =
            Arc::new(NoopGateway::new());
        let kernel: Arc<dyn crate::runtime::kernel_api::KernelApi> =
            Arc::new(Kernel::new(Arc::clone(&gateway)));
        let registry = std::sync::Arc::new(
            crate::runtime::ability_dispatch::LocalAbilityRegistry::new(),
        );
        let dispatcher = AbilityDispatcher::new(registry, gateway);
        let resolver: Arc<dyn crate::runtime::invocation_target::TargetResolver> =
            Arc::new(LocalNodeResolver::new(crate::runtime::domain::NodeId::new("self")));
        let proxy = Arc::new(AbilityProxy::new_with_dispatcher(
            kernel,
            dispatcher,
            resolver,
        ));
        let invoker = ProxyLocalInvoker::new(proxy);
        let err = invoker
            .invoke_sync("totally.unknown", serde_json::json!({}))
            .expect_err("unknown ability must surface as Err");
        assert!(
            err.contains("not_found"),
            "expected NOT_FOUND code in error string; got {err}"
        );
    }

    #[test]
    fn invoke_provider_routes_observe_health_end_to_end_through_real_proxy() {
        use crate::runtime::ability_descriptor::Visibility;
        use crate::runtime::ability_dispatch::AbilityDispatcher;
        use crate::runtime::gateway::NoopGateway;
        use crate::runtime::invocation_target::LocalNodeResolver;
        use crate::runtime::kernel::Kernel;
        use crate::services::control::ability_proxy::AbilityProxy;
        use easynet_axon::mcp::McpToolProvider;
        use std::sync::Arc;

        let gateway: Arc<dyn crate::runtime::gateway_api::GatewayApi> =
            Arc::new(NoopGateway::new());
        let kernel: Arc<dyn crate::runtime::kernel_api::KernelApi> =
            Arc::new(Kernel::new(Arc::clone(&gateway)));
        let registry = crate::runtime::agents::build_registry();
        let dispatcher = AbilityDispatcher::new(registry, gateway);
        let resolver: Arc<dyn crate::runtime::invocation_target::TargetResolver> =
            Arc::new(LocalNodeResolver::new(crate::runtime::domain::NodeId::new("self")));
        let proxy = Arc::new(AbilityProxy::new_with_dispatcher(
            kernel,
            dispatcher,
            resolver,
        ));
        let invoker = ProxyLocalInvoker::new(proxy);

        let descs = vec![AbilityDescriptor::new(
            "observe.health",
            "easynet:///r/acme/agent/01DEV",
            Visibility::Public,
        )
        .unwrap()];
        let provider = InvokeMcpProvider::new(invoker, descs);

        // tools/list mirrors the descriptor.
        let specs = provider.tool_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0]["name"], "observe.health");

        // tools/call dispatches through the proxy.
        let result =
            provider.handle_tool_call("observe.health", &serde_json::Map::new());
        assert!(
            !result.is_error,
            "observe.health must succeed end-to-end through MCP shim"
        );
    }
}

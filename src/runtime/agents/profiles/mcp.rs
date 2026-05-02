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
                .with_description(m.description)
        })
        .collect()
}

/// Project an AbilityDescriptor into the JSON-Schema shape MCP
/// clients expect from `tools/list`. The MCP wire format requires
/// `{name, description, inputSchema}`; we map:
///
///   name         ← descriptor.name
///   description  ← descriptor.description (the real human blurb
///                  from the registry's metadata table); when empty,
///                  fall back to the qualified name. The pre-fix
///                  behaviour stuffed `"<name> (source: <source>)"`
///                  into description, which made every tool look
///                  like its description was its provenance string —
///                  the LLM had to infer purpose from the name
///                  alone. Surfaced in the audit conversation when
///                  the MCP probe showed every tool's description as
///                  `"fs.read (source: kernel:built-in)"`.
///   inputSchema  ← descriptor.schema_summary.input (JSON Schema)
pub fn tool_spec_from_descriptor(
    descriptor: &crate::runtime::ability_descriptor::AbilityDescriptor,
) -> serde_json::Value {
    let description = if !descriptor.description.is_empty() {
        descriptor.description.clone()
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
            subject: None,
        });
        // Per the proxy contract, the first frame is either a
        // single Result or a single Error. We surface the Result's
        // value verbatim; an Error becomes the Err string. Receipt
        // headers (P4.8c) are dropped here because the MCP wire
        // shape has no header field; receivers that need them must
        // use the IPC path directly, not the MCP shim.
        match frames.into_iter().next() {
            Some(OutgoingFrame::Result { value, .. }) => Ok(value),
            Some(OutgoingFrame::Error { code, message, .. }) => Err(format!("{code}: {message}")),
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

/// Configuration for `build_stdio_server` — the single entry point
/// that constructs an MCP server ready to drive stdin/stdout. Both
/// `easynet mcp_server` and `easynet start --mcp` go through here
/// so the construction logic lives in exactly one place.
#[derive(Debug, Clone)]
pub struct StdioServerConfig {
    /// Free-form server-name suffix; appears in the MCP handshake's
    /// `serverName` field. Convention: `easynet-<role>`.
    pub server_name: String,
    /// Tenant ID — informational; routed dispatch honours whatever
    /// the loaded credentials carry.
    pub tenant_id: String,
    /// Optional: when this MCP server is the workspace MCP for a
    /// specific agent (the daemon spawned it as
    /// `easynet mcp serve --agent <name>`), set this to that
    /// agent's name. The descriptor list will then include the
    /// agent's per-workspace ability TOMLs from
    /// `<agent_root>/abilities/*.toml` IN ADDITION to the
    /// host-wide profile descriptors. None = host-only catalog
    /// (the operator-installed `easynet mcp install` path).
    ///
    /// Why this matters: every claude.chat / codex.chat call
    /// spawns a workspace MCP server with --agent set. Without
    /// this field, agents could only see the device-profile
    /// abilities through MCP — never their own abilities, which
    /// is the whole point of letting an agent expose abilities
    /// per the EasyNet ontology.
    pub agent_name: Option<String>,
}

/// Pre-built provider + server name, ready to hand to
/// `easynet_axon::mcp::StdioMcpServer::new`. Returned by
/// `build_stdio_server` so callers can decide whether to run
/// foreground (mcp_server) or in a spawned thread (start --mcp).
pub struct ConfiguredStdioServer {
    pub provider: InvokeMcpProvider<ProxyLocalInvoker>,
    pub server_name: String,
}

impl ConfiguredStdioServer {
    /// Number of MCP tools the configured provider advertises.
    /// Convenience getter so callers don't reach into provider state.
    pub fn descriptor_count(&self) -> usize {
        self.provider.descriptor_count()
    }
}

/// One-stop builder: assemble a Kernel + AbilityProxy, derive the
/// host's AbilityDescriptors from local-agents.json, and produce
/// a configured InvokeMcpProvider ready for the stdio runner.
///
/// Both `easynet mcp_server` and `easynet start --mcp` call this
/// — they differ only in argument parsing and how they launch the
/// stdio server (foreground vs. spawned thread).
pub fn build_stdio_server(config: &StdioServerConfig) -> ConfiguredStdioServer {
    use crate::services::control::ability_proxy::AbilityProxy;
    use std::sync::Arc;

    let gateway: Arc<dyn crate::runtime::gateway_api::GatewayApi> =
        Arc::new(crate::runtime::gateway::NoopGateway::new());
    let kernel: Arc<dyn crate::runtime::kernel_api::KernelApi> =
        Arc::new(crate::runtime::kernel::Kernel::new(gateway));
    let proxy = Arc::new(AbilityProxy::new(kernel));

    let mut descriptors = crate::runtime::agents::profiles::load_host_descriptors();

    // Workspace MCP: when --agent <name> is set, append that
    // agent's per-workspace ability descriptors. Read straight
    // from the agent's on-disk manifests (the same path
    // `easynet agent abilities <name>` walks). This is what makes
    // an agent's own abilities (e.g. `claude.audit-test-ability`
    // declared at <workspace>/abilities/...) visible to the
    // spawned LLM CLI as MCP tools — without this, the EasyNet
    // ontology's "agent exposes abilities" promise was just
    // metadata: agents could declare abilities but the LLM
    // running inside them couldn't call them.
    if let Some(agent_name) = config.agent_name.as_deref() {
        descriptors.extend(per_agent_workspace_descriptors(agent_name));
    }

    let invoker = ProxyLocalInvoker::new(proxy);
    let provider = InvokeMcpProvider::new(invoker, descriptors);
    let _ = config.tenant_id; // tenant is informational for now
    ConfiguredStdioServer {
        provider,
        server_name: config.server_name.clone(),
    }
}

/// Build AbilityDescriptors for one agent's per-workspace
/// ability TOMLs. Reads from the canonical disk path that
/// `easynet agent abilities <name>` reads from. Returns an
/// empty Vec when the agent isn't registered or its workspace
/// has no ability manifests; the MCP server proceeds with the
/// host-wide catalog only.
fn per_agent_workspace_descriptors(
    agent_name: &str,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};

    // Resolve the agent entry. If unregistered, no per-agent
    // catalog to add — the workspace MCP server is still useful
    // for the host catalog alone.
    let registry = match crate::registry::agents::load_agents() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let entry = match registry.agents.get(agent_name) {
        Some(e) => e,
        None => return Vec::new(),
    };

    let mut out: Vec<AbilityDescriptor> = Vec::new();

    let to_descriptor = |s: crate::runtime::abilities::AgentAbilitySpec,
                         owner_uri: &str,
                         source: String|
     -> Option<AbilityDescriptor> {
        AbilityDescriptor::new(s.name().to_string(), owner_uri, Visibility::Scoped)
            .ok()
            .map(|d| {
                // AgentAbilitySpec calls its JSON-Schema field
                // `parameters()` (carrying the input schema in
                // the chat-style "parameters" shape) — that
                // IS the input schema for the descriptor.
                d.with_input_schema(s.parameters().clone())
                    .with_source(source)
                    .with_description(s.description())
            })
    };

    // Phase 1: this agent's own abilities. Owner URI uses the
    // agent's own name. The agent's `<agent_name>.chat` ability is
    // filtered out — it is the outgoing surface, not something to
    // expose AS a tool to the LLM running INSIDE it (that would
    // invite infinite recursion).
    let own_specs = crate::runtime::abilities::abilities_for(agent_name, entry);
    let own_owner_uri = format!("agent://{agent_name}");
    let self_chat = format!("{agent_name}.chat");
    for s in own_specs.into_iter().filter(|s| s.name() != self_chat) {
        if let Some(d) = to_descriptor(s, &own_owner_uri, format!("agent:{agent_name}")) {
            out.push(d);
        }
    }

    // Phase 1b: synthesise descriptors for the agent's self-bundle
    // builtins — `<agent>.discover` and `<agent>.invoke`. These are
    // registered programmatically in `build_registry_with_services`,
    // not declared via on-disk TOMLs, so the workspace enumeration
    // above never sees them. Without these synthesised entries an
    // LLM running inside this agent cannot call its own discovery /
    // invocation surface even though the daemon would happily
    // dispatch them — exactly the gap that left `claude.discover`
    // missing from `tools/list` after the ability-only refactor.
    //
    // The two descriptors are intentionally per-agent: every agent
    // owns its own discover / invoke (the discovery ladder is
    // owner-scoped). Source = `kernel:built-in:self-bundle` so an
    // operator inspecting the descriptor catalogue can tell at a
    // glance the entry came from a synth path, not a TOML.
    {
        let discover_name = format!(
            "{agent_name}.{}",
            crate::runtime::agents::discover_ability::ABILITY_VERB
        );
        let invoke_name = format!(
            "{agent_name}.{}",
            crate::runtime::agents::invoke_ability::ABILITY_VERB
        );
        for (name, schema, description) in [
            (
                discover_name,
                crate::runtime::agents::discover_ability::input_schema(),
                crate::runtime::agents::discover_ability::description(),
            ),
            (
                invoke_name,
                crate::runtime::agents::invoke_ability::input_schema(),
                crate::runtime::agents::invoke_ability::description(),
            ),
        ] {
            if let Ok(d) = AbilityDescriptor::new(name, &own_owner_uri, Visibility::Scoped) {
                out.push(
                    d.with_input_schema(schema)
                        .with_source("kernel:built-in:self-bundle")
                        .with_description(description),
                );
            }
        }
    }

    // Phase 2: every OTHER registered agent's abilities. This is
    // the cross-agent surface — when agent A is the active LLM and
    // the user asks for something only agent B has the skill for,
    // agent A's tool list now includes `<B>.<verb>` so the LLM can
    // route to it. Calling those tools dispatches through the
    // daemon's per-agent fallback resolver, which then runs B's
    // own chat-translation handler with B's own skills exposed.
    //
    // `<other_name>.chat` is excluded for the same reason: chat is
    // the agent's outgoing surface, not a callable tool. Calling
    // another agent's chat from inside agent A would just spawn
    // a nested chat session that bypasses the per-ability route
    // we are trying to encourage.
    for (other_name, other_entry) in &registry.agents {
        if other_name == agent_name {
            continue;
        }
        let other_chat = format!("{other_name}.chat");
        let other_owner = format!("agent://{other_name}");
        for s in crate::runtime::abilities::abilities_for(other_name, other_entry)
            .into_iter()
            .filter(|s| s.name() != other_chat)
        {
            if let Some(d) = to_descriptor(s, &other_owner, format!("agent:{other_name}")) {
                out.push(d);
            }
        }
    }

    out
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
            .with_description("List every registered agent on this host.")
    }

    #[test]
    fn tool_spec_from_descriptor_emits_mcp_shape() {
        let spec = tool_spec_from_descriptor(&d("fleet.list_agents"));
        assert_eq!(spec["name"], "fleet.list_agents");
        // The MCP description is the human blurb from the registry,
        // NOT the provenance string. Pre-fix this asserted the
        // opposite — bug pinned upside-down. Updated when the
        // transform started reading descriptor.description.
        assert_eq!(
            spec["description"].as_str().unwrap(),
            "List every registered agent on this host."
        );
        assert!(
            !spec["description"]
                .as_str()
                .unwrap()
                .contains("kernel:built-in"),
            "description must not leak the source/provenance string"
        );
        assert_eq!(spec["inputSchema"]["type"], "object");
    }

    #[test]
    fn tool_spec_falls_back_to_name_when_description_is_empty() {
        // No `.with_description(...)` → empty string → fall back to
        // qualified name so the MCP wire never carries an empty
        // description (which Claude Code's tool list rejects).
        let desc = AbilityDescriptor::new("a.b", "u", Visibility::Public)
            .unwrap()
            .with_input_schema(serde_json::json!({"type":"object"}));
        let spec = tool_spec_from_descriptor(&desc);
        assert_eq!(spec["description"], "a.b");
    }

    #[test]
    fn tool_spec_falls_back_to_object_schema_when_input_is_null() {
        let mut desc = AbilityDescriptor::new("a.b", "u", Visibility::Public).unwrap();
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
        let p = InvokeMcpProvider::new(RecordingInvoker::new(Ok(serde_json::json!({}))), descs);
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
        assert!(
            result.is_error,
            "unknown tool must surface as is_error=true"
        );
        // Crucially: the invoker MUST NOT have been called.
        assert!(p.invoker.last_ability.borrow().is_none());
    }

    #[test]
    fn known_tool_call_is_dispatched_via_invoker() {
        let invoker = RecordingInvoker::new(Ok(serde_json::json!({"status": "healthy"})));
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
        assert!(result.payload["error"]
            .as_str()
            .unwrap()
            .contains("policy denied"));
    }

    #[test]
    fn descriptor_count_matches_input() {
        let invoker = RecordingInvoker::new(Ok(serde_json::json!({})));
        let p = InvokeMcpProvider::new(invoker, vec![d("observe.health"), d("fleet.list_agents")]);
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
        let resolver: Arc<dyn crate::runtime::invocation_target::TargetResolver> = Arc::new(
            LocalNodeResolver::new(crate::runtime::domain::NodeId::new("self")),
        );
        let proxy = Arc::new(AbilityProxy::new_with_dispatcher(
            kernel, dispatcher, resolver,
        ));
        let invoker = ProxyLocalInvoker::new(proxy);
        let result = invoker
            .invoke_sync("observe.health", serde_json::json!({}))
            .expect("observe.health must dispatch successfully");
        // observe.health returns a JSON object with at least
        // `status`. Exact shape is owned by ping.rs; we only assert
        // the dispatch happened (got a non-null Value).
        assert!(
            !result.is_null(),
            "observe.health must return a value, not null"
        );
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
        let registry =
            std::sync::Arc::new(crate::runtime::ability_dispatch::LocalAbilityRegistry::new());
        let dispatcher = AbilityDispatcher::new(registry, gateway);
        let resolver: Arc<dyn crate::runtime::invocation_target::TargetResolver> = Arc::new(
            LocalNodeResolver::new(crate::runtime::domain::NodeId::new("self")),
        );
        let proxy = Arc::new(AbilityProxy::new_with_dispatcher(
            kernel, dispatcher, resolver,
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
    fn build_stdio_server_produces_provider_with_at_least_observe_health() {
        // Single-source-of-truth contract: both `easynet mcp_server`
        // and `easynet start --mcp` go through `build_stdio_server`.
        // The result MUST advertise every device-profile ability the
        // live registry registers, anchored on whatever local-agents.json
        // says (or the literal "self" pre-join).
        let _h = crate::facade::cli::test_support::HomeGuard::new();
        let cfg = StdioServerConfig {
            server_name: "easynet-test".into(),
            tenant_id: "test-tenant".into(),
            agent_name: None,
        };
        let configured = build_stdio_server(&cfg);
        assert_eq!(configured.server_name, "easynet-test");
        assert!(
            configured.descriptor_count() > 0,
            "build_stdio_server must surface at least the device-profile abilities; \
             got descriptor_count = 0"
        );
        // The pre-join fallback anchors on "self"; the descriptors
        // we get must reference this URI as owner.
        let owners: std::collections::HashSet<String> = configured
            .provider
            .descriptors
            .iter()
            .map(|d| d.owner_agent_uri.clone())
            .collect();
        assert!(
            owners.contains("self"),
            "pre-join fallback must anchor descriptors on `self`; got owners = {owners:?}"
        );
    }

    #[test]
    fn build_stdio_server_anchors_descriptors_on_persisted_host_uri_when_present() {
        let _h = crate::facade::cli::test_support::HomeGuard::new();
        // Pre-populate local-agents.json with a host URI; build_stdio_server
        // must pick it up.
        let mut file = crate::persistence::local_agents::LocalAgentsFile::default();
        file.host_device_agent_uri = "easynet:///r/acme/agent/01DEV".into();
        crate::persistence::local_agents::save(&file).unwrap();

        let cfg = StdioServerConfig {
            server_name: "easynet-test".into(),
            tenant_id: "t".into(),
            agent_name: None,
        };
        let configured = build_stdio_server(&cfg);
        let owners: std::collections::HashSet<String> = configured
            .provider
            .descriptors
            .iter()
            .map(|d| d.owner_agent_uri.clone())
            .collect();
        assert!(
            owners.contains("easynet:///r/acme/agent/01DEV"),
            "post-join descriptors must anchor on the persisted URA; \
             got owners = {owners:?}"
        );
        assert!(
            !owners.contains("self"),
            "post-join descriptors must NOT fall back to `self` when the URA is known"
        );
    }

    #[test]
    fn build_stdio_server_with_agent_name_includes_per_workspace_abilities() {
        // The G1 fix: when --agent <name> is set on `easynet mcp serve`,
        // the descriptor list MUST include the agent's own
        // ability TOMLs from <agent_root>/abilities/. Without this
        // the agent's own abilities are invisible to the LLM
        // running inside that agent's workspace, which breaks
        // the EasyNet ontology's "agent exposes abilities"
        // promise: agents could declare abilities but the LLM
        // they wrap couldn't call them.
        use crate::facade::cli::test_support::HomeGuard;
        use crate::registry::agents::{AgentEntry, AgentType};

        let _g = HomeGuard::new();

        // Set up an agent + a custom ability under its workspace.
        // Use a name unlikely to collide with the developer's
        // real ~/.easynet/workspaces/* contents. HomeGuard already
        // isolates HOME, but multiple in-process tests can still
        // race on the same per-test tempdir if they all pick
        // generic names like "alice" or "bob".
        let agent = "g1-test-agent";

        let mut registry = crate::registry::agents::AgentRegistry::default();
        let entry = AgentEntry::new(AgentType::ClaudeCode, None);
        registry.agents.insert(agent.into(), entry);
        crate::registry::agents::save_agents(&registry).unwrap();

        let workspace_root = crate::persistence::config::agents_root().join(agent);
        std::fs::create_dir_all(workspace_root.join("abilities")).unwrap();
        std::fs::write(
            workspace_root.join("agent.toml"),
            &format!("name = \"{agent}\"\nruntime = \"claude-code\"\n"),
        )
        .unwrap();
        std::fs::write(
            workspace_root.join("abilities/code-review.ability.toml"),
            "schema_version = \"1\"\n\
             name = \"code-review\"\n\
             description = \"Custom workspace ability.\"\n\
             [input_schema]\n\
             type = \"object\"\n\
             additionalProperties = false\n",
        )
        .unwrap();

        // Build with agent_name = Some("alice").
        let cfg = StdioServerConfig {
            server_name: "easynet-test".into(),
            tenant_id: "t".into(),
            agent_name: Some(agent.to_string()),
        };
        let configured = build_stdio_server(&cfg);
        let names: Vec<String> = configured
            .provider
            .descriptors
            .iter()
            .map(|d| d.name.clone())
            .collect();
        assert!(
            names.iter().any(|n| n == &format!("{agent}.code-review")),
            "build_stdio_server with agent_name={agent} MUST include \
             the {agent}.code-review ability from its workspace; got {names:?}"
        );
        // The agent's own .chat ability is excluded — exposing it
        // to the LLM running INSIDE alice would invite recursion.
        assert!(
            !names.iter().any(|n| n == &format!("{agent}.chat")),
            "{agent}.chat must be excluded from its own MCP tool catalog \
             to prevent the agent from calling itself recursively; got {names:?}"
        );

        // Same build with agent_name = None must NOT include alice's abilities.
        let cfg_no_agent = StdioServerConfig {
            server_name: "easynet-test".into(),
            tenant_id: "t".into(),
            agent_name: None,
        };
        let no_agent = build_stdio_server(&cfg_no_agent);
        let no_agent_names: Vec<String> = no_agent
            .provider
            .descriptors
            .iter()
            .map(|d| d.name.clone())
            .collect();
        assert!(
            !no_agent_names
                .iter()
                .any(|n| n == &format!("{agent}.code-review")),
            "agent_name=None must NOT include any per-agent abilities; got {no_agent_names:?}"
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
        let resolver: Arc<dyn crate::runtime::invocation_target::TargetResolver> = Arc::new(
            LocalNodeResolver::new(crate::runtime::domain::NodeId::new("self")),
        );
        let proxy = Arc::new(AbilityProxy::new_with_dispatcher(
            kernel, dispatcher, resolver,
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
        let result = provider.handle_tool_call("observe.health", &serde_json::Map::new());
        assert!(
            !result.is_error,
            "observe.health must succeed end-to-end through MCP shim"
        );
    }
}

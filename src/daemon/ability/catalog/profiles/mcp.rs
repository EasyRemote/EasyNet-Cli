//! mcp profile — RFC-001 §1 [P6].
//!
//! Per restatement-mapping decision P6: a single mcp-profile Agent
//! advertises BOTH inbound and outbound MCP — `mcp.bridge.*` (incoming
//! MCP tools/list + tools/call) and `mcp.client.*` (outgoing MCP calls
//! to external servers). They share one Agent identity projection rather
//! than splitting into two profiles.
//!
//! This is the ONLY place MCP awareness is permitted in the CLI per
//! RFC-001 §A3 (MCP only at edge adapters; everywhere else is
//! Invocation-only). The conformance script enforces this.
//!
//! Descriptor projection
//! ---------------------
//! MCP descriptors are generated from the dispatch registry entries whose
//! projection class is `OwnerKind::Agent(DEFAULT_MCP_AGENT_ID)`. This file does
//! not infer ownership from ability name prefixes.
//!
//! What this file provides today
//! -----------------------------
//!   - `descriptors_for(owner_ura)`    : §1.6 descriptors emitter
//!   - `InvokeMcpProvider`             : the McpToolProvider impl that
//!     translates every `tools/list` and `tools/call` into a local
//!     Invoke surface. Production stdio servers call back into the
//!     live daemon; tests may still inject an in-process proxy.
//!     Replaces the legacy facade MCP provider that owned a duplicate
//!     tool catalog and called deleted bridge methods directly.
//!   - `tool_specs_from_descriptors(...)` : projects AbilityDescriptors
//!     into the JSON-Schema shape MCP clients expect.
//!
//! Facade retirement policy (P4.8d)
//! --------------------------------
//! Every MCP client touchpoint MUST flow through this module or the
//! explicit CLI edge in `src/cli/mcp_server.rs` / `src/cli/start.rs`.
//! The old facade MCP quarantine anchor is retired; the conformance
//! scripts now reject it instead of allowing a doc-only placeholder.
//! The legacy `specs.rs` catalog and `handlers.rs` dispatchers are
//! deleted, and the stdio MCP server scaffolding lives at the CLI edge
//! while this module owns provider trait wiring, route tables,
//! descriptor projection, and dispatch into daemon Invocation.

use crate::daemon::execution::mcp::stdio::{McpToolProvider, ProgressSink, ToolResult};

/// AbilityDescriptors for every mcp.bridge.* + mcp.client.* in the
/// live registry, anchored to the mcp-profile's canonical URA. All
/// SCOPED per §18 — local MCP clients only for bridge.*; the daemon
/// itself + selected internal callers for client.*. P4.7 narrows.
pub fn descriptors_for(
    owner_ura: &str,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    use crate::daemon::ability::descriptors::Visibility;
    use crate::daemon::ability::dispatch::OwnerKind;

    super::system_descriptors_for_owner(
        owner_ura,
        OwnerKind::Agent(super::DEFAULT_MCP_AGENT_ID.to_string()),
        |_| Visibility::Scoped,
    )
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
    descriptor: &crate::daemon::ability::descriptors::AbilityDescriptor,
) -> serde_json::Value {
    tool_spec_from_descriptor_with_name(descriptor, &mcp_tool_name_for_ability(&descriptor.name))
}

fn tool_spec_from_descriptor_with_name(
    descriptor: &crate::daemon::ability::descriptors::AbilityDescriptor,
    tool_name: &str,
) -> serde_json::Value {
    let base_description = if !descriptor.description.is_empty() {
        descriptor.description.clone()
    } else {
        descriptor.name.clone()
    };
    let description = annotated_mcp_description(descriptor, &base_description);
    let input_schema = if descriptor.schema_summary.input.is_null() {
        serde_json::json!({"type": "object"})
    } else {
        descriptor.schema_summary.input.clone()
    };
    serde_json::json!({
        "name": tool_name,
        "description": description,
        "inputSchema": input_schema,
        "x-easynet": {
            "ability": descriptor.name,
            "owner_ura": descriptor.owner_ura,
            "source": descriptor.source,
            "owner_user": descriptor.metadata.get("owner_user").cloned().unwrap_or_default(),
            "owner_agent": descriptor.metadata.get("owner_agent").cloned().unwrap_or_default(),
            "exec_kind": descriptor.metadata.get("exec_kind").cloned().unwrap_or_default(),
            "mcp_server": descriptor.metadata.get("mcp_server").cloned().unwrap_or_default(),
            "mcp_tool": descriptor.metadata.get("mcp_tool").cloned().unwrap_or_default(),
            "cost_kind": descriptor.metadata.get("cost_kind").cloned().unwrap_or_else(|| inferred_cost_kind(descriptor).to_string()),
            "cost_label": descriptor.metadata.get("cost_label").cloned().unwrap_or_else(|| inferred_cost_label(inferred_cost_kind(descriptor)).to_string()),
        },
    })
}

fn annotated_mcp_description(
    descriptor: &crate::daemon::ability::descriptors::AbilityDescriptor,
    base_description: &str,
) -> String {
    let owner = owner_label_for_descriptor(descriptor);
    let cost_kind = descriptor
        .metadata
        .get("cost_kind")
        .map(String::as_str)
        .unwrap_or_else(|| inferred_cost_kind(descriptor));
    let cost_label = descriptor
        .metadata
        .get("cost_label")
        .map(String::as_str)
        .unwrap_or_else(|| inferred_cost_label(cost_kind));
    format!(
        "[EasyNet ability: {} | owner: {} | cost: {} ({})] {}",
        descriptor.name, owner, cost_kind, cost_label, base_description
    )
}

fn owner_label_for_descriptor(
    descriptor: &crate::daemon::ability::descriptors::AbilityDescriptor,
) -> String {
    let user = descriptor.metadata.get("owner_user").map(String::as_str);
    let agent = descriptor.metadata.get("owner_agent").map(String::as_str);
    match (user, agent) {
        (Some(user), Some(agent)) if !user.is_empty() && !agent.is_empty() => {
            format!("user/{user} agent/{agent}")
        }
        (_, Some(agent)) if !agent.is_empty() => format!("agent/{agent}"),
        _ => parsed_owner_label(&descriptor.owner_ura)
            .unwrap_or_else(|| descriptor.owner_ura.clone()),
    }
}

fn parsed_owner_label(owner_ura: &str) -> Option<String> {
    let parsed = crate::core::ura::parse_ura(owner_ura).ok()?;
    match parsed.kind {
        crate::core::ura::URAKind::Agent => parsed
            .agent_ids()
            .map(|(user_id, agent_id)| format!("user/{user_id} agent/{agent_id}")),
        crate::core::ura::URAKind::Ability => match parsed.ability()?.owner {
            crate::core::ura::AbilityOwner::Agent { user_id, agent_id } => {
                Some(format!("user/{user_id} agent/{agent_id}"))
            }
            crate::core::ura::AbilityOwner::Device { device_id } => {
                Some(format!("device/{device_id}"))
            }
            crate::core::ura::AbilityOwner::Hub => Some("hub".to_string()),
        },
        crate::core::ura::URAKind::User => {
            parsed.user_id().map(|user_id| format!("user/{user_id}"))
        }
        crate::core::ura::URAKind::Device => parsed
            .device_id()
            .map(|device_id| format!("device/{device_id}")),
        crate::core::ura::URAKind::Hub => Some("hub".to_string()),
        _ => None,
    }
}

/// Fallback cost classification used when a descriptor's metadata
/// does not declare `cost_kind` explicitly.
///
/// **Honesty rule (load-bearing).** This used to return `"free"` for
/// any descriptor that wasn't an agent-chat surface. That mislabelled
/// every reflectively-registered upstream MCP tool — operators saw
/// billed upstreams as `cost: free (free/local)`. Per the plan §"Cost
/// is static catalog metadata" rule, advisory cost lives in the
/// ability manifest, not in heuristics, and we must NOT default to
/// `free` for catalog rows we have not seen. We therefore return
/// `"unknown"` for every descriptor that does not explicitly declare
/// otherwise; the one inference we keep is the agent-chat case (no
/// `exec_kind` AND `source = "agent:…"`) because that path is always
/// an LLM dispatch by construction.
fn inferred_cost_kind(
    descriptor: &crate::daemon::ability::descriptors::AbilityDescriptor,
) -> &'static str {
    if descriptor.source.starts_with("agent:") && !descriptor.metadata.contains_key("exec_kind") {
        return "llm_metered";
    }
    "unknown"
}

fn inferred_cost_label(cost_kind: &str) -> &'static str {
    match cost_kind {
        "free" => "free/local",
        "external_metered" => "external API billing may apply",
        "llm_metered" => "LLM token billing may apply",
        "unknown" => "cost not declared",
        _ => "cost not declared",
    }
}

pub fn tool_specs_from_descriptors(
    descriptors: &[crate::daemon::ability::descriptors::AbilityDescriptor],
) -> Vec<serde_json::Value> {
    let table = McpToolRouteTable::from_descriptors(descriptors);
    table
        .iter()
        .map(|(tool_name, index)| {
            tool_spec_from_descriptor_with_name(&descriptors[index], tool_name)
        })
        .collect()
}

fn descriptor_is_mcp_callable(
    descriptor: &crate::daemon::ability::descriptors::AbilityDescriptor,
) -> bool {
    descriptor.call_mode() == crate::daemon::ability::descriptors::CallMode::Rpc
}

/// Convert a canonical EasyNet ability name into a client-safe MCP
/// tool name. Codex and Claude Code surface MCP tools as function
/// names, and the lowest-common-denominator function-name grammar is
/// `[A-Za-z0-9_-]+`; dotted EasyNet names such as
/// `openai.mcp_unit_converter_convert_length` are therefore projected
/// as `openai_mcp_unit_converter_convert_length`.
///
/// The canonical ability name is retained in `x-easynet.ability` and in
/// the provider's route table. This keeps MCP naming an edge-adapter
/// concern; the runtime registry and federation URAs stay dotted.
pub fn mcp_tool_name_for_ability(ability_name: &str) -> String {
    let mut out = String::with_capacity(ability_name.len());
    let mut last_was_underscore = false;
    for ch in ability_name.chars() {
        let keep = ch.is_ascii_alphanumeric() || ch == '-' || ch == '_';
        let next = if keep { ch } else { '_' };
        if next == '_' && last_was_underscore && ch != '_' {
            continue;
        }
        last_was_underscore = next == '_';
        out.push(next);
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "ability".to_string()
    } else {
        trimmed.to_string()
    }
}

/// One row in the MCP-tool ↔ EasyNet-ability projection table.
/// Kept module-private because the public API is `McpToolRouteTable`
/// — callers should never index by `ToolRoute` directly.
#[derive(Debug, Clone)]
struct ToolRoute {
    /// Tool name advertised to MCP clients (lowest-common-denominator
    /// `[A-Za-z0-9_-]+` grammar; deterministic projection of
    /// `ability_name`).
    tool_name: String,
    /// Descriptor-bound local invocation target. Keeping this value in the
    /// same row that produced `tool_name` makes listing and calling one atomic
    /// projection: tools/call cannot silently replace an Agent/Hub owner with
    /// the local Device default.
    target: crate::daemon::invocation::routing::target::LocalAbilityTarget,
    /// Index of the source descriptor in the caller-provided slice.
    /// Lets `tool_spec_from_descriptor_with_name` re-attach metadata
    /// from the original descriptor without cloning it into the route.
    index: usize,
}

/// Forward + reverse routing table that maps between canonical
/// EasyNet ability names and the lowest-common-denominator names
/// MCP clients (Codex, Claude Code) expect.
///
/// **Why this is an object, not three free functions.** Three call
/// sites — the unary `mcp.bridge.call_tool` handler, the
/// inbound MCP server's `tools/list` projection, and
/// `InvokeMcpProvider` — all need the same forward+reverse views
/// of the same descriptor slice. The pre-refactor code recomputed
/// the routing from scratch in each call (O(N log N) per
/// `call_tool` invocation, repeated on every bridge handler call
/// for a 28-server mcp-bench catalogue), and the dual-state
/// `(descriptors, tool_routes)` pair inside `InvokeMcpProvider`
/// invited "forgot to keep in sync" bugs. `McpToolRouteTable`
/// captures both directions once and exposes O(log N) lookups.
///
/// Construction is deterministic in the descriptor order. Reverse
/// lookup accepts only the advertised MCP tool name; canonical dotted
/// ability names stay internal to `x-easynet.ability` and dispatch.
#[derive(Debug, Clone, Default)]
pub struct McpToolRouteTable {
    routes: Vec<ToolRoute>,
    /// MCP tool name → route row. Hot path for `call_tool` dispatch.
    reverse: std::collections::BTreeMap<String, usize>,
}

impl McpToolRouteTable {
    /// Build the routing table for a descriptor slice. The descriptor
    /// order determines the deterministic tie-break order if two
    /// canonical names project to the same MCP tool name; that
    /// ordering matches the pre-refactor behaviour.
    pub fn from_descriptors(
        descriptors: &[crate::daemon::ability::descriptors::AbilityDescriptor],
    ) -> Self {
        let mut routes = Vec::with_capacity(descriptors.len());
        let mut used: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();

        for (index, descriptor) in descriptors.iter().enumerate() {
            if !descriptor_is_mcp_callable(descriptor) {
                continue;
            }
            let ability_ura = descriptor.canonical_ability_ura().unwrap_or_else(|| {
                panic!(
                    "validated AbilityDescriptor {} owned by {} has no canonical Ability URA",
                    descriptor.name, descriptor.owner_ura
                )
            });
            let selector =
                crate::core::ura::AbilitySelector::parse(&ability_ura).unwrap_or_else(|error| {
                    panic!(
                        "validated AbilityDescriptor {} produced invalid Ability URA {}: {error}",
                        descriptor.name, ability_ura
                    )
                });
            let target =
                crate::daemon::invocation::routing::target::LocalAbilityTarget::from_selector(
                    &selector,
                );
            let base = mcp_tool_name_for_ability(target.dispatch_name());
            let mut tool_name = match used.get(&base) {
                None => base.clone(),
                Some(existing) if existing == &ability_ura => base.clone(),
                Some(_) => format!("{base}__{}", short_ability_hash(&ability_ura)),
            };
            if let Some(existing) = used.get(&tool_name) {
                if existing != &ability_ura {
                    let hash = short_ability_hash(&ability_ura);
                    let mut suffix = 2usize;
                    while used
                        .get(&tool_name)
                        .is_some_and(|existing| existing != &ability_ura)
                    {
                        tool_name = format!("{base}__{hash}_{suffix}");
                        suffix += 1;
                    }
                }
            }
            used.insert(tool_name.clone(), ability_ura);
            routes.push(ToolRoute {
                tool_name,
                target,
                index,
            });
        }

        let mut reverse = std::collections::BTreeMap::new();
        for (index, route) in routes.iter().enumerate() {
            reverse.insert(route.tool_name.clone(), index);
        }

        Self { routes, reverse }
    }

    /// Resolve `tool_name` (an MCP-facing name) back to the canonical
    /// dotted EasyNet ability name.
    pub fn canonical_for_tool<'a>(&'a self, tool_name: &str) -> Option<&'a str> {
        self.target_for_tool(tool_name)
            .map(crate::daemon::invocation::routing::target::LocalAbilityTarget::dispatch_name)
    }

    /// Resolve the complete descriptor-bound target advertised for a tool.
    pub fn target_for_tool(
        &self,
        tool_name: &str,
    ) -> Option<&crate::daemon::invocation::routing::target::LocalAbilityTarget> {
        self.route_for_tool(tool_name).map(|route| &route.target)
    }

    fn len(&self) -> usize {
        self.routes.len()
    }

    fn descriptor_index_for_tool(&self, tool_name: &str) -> Option<usize> {
        self.route_for_tool(tool_name).map(|route| route.index)
    }

    fn route_for_tool(&self, tool_name: &str) -> Option<&ToolRoute> {
        self.reverse
            .get(tool_name)
            .and_then(|index| self.routes.get(*index))
    }

    /// Iterate every row in projection order. Yields `(tool_name,
    /// descriptor_index)` pairs so a caller that holds the original
    /// descriptor slice can build per-row tool specs without
    /// re-projecting.
    fn iter(&self) -> impl Iterator<Item = (&str, usize)> + '_ {
        self.routes.iter().map(|r| (r.tool_name.as_str(), r.index))
    }
}

/// Resolve an advertised MCP tool name against a freshly-built table.
/// Existing call sites that have a short-lived `&[AbilityDescriptor]`
/// can keep their shape; longer-lived call sites should hold an
/// `McpToolRouteTable` instead.
pub fn canonical_ability_name_for_mcp_tool<'a>(
    descriptors: &'a [crate::daemon::ability::descriptors::AbilityDescriptor],
    tool_name: &str,
) -> Option<&'a str> {
    let table = McpToolRouteTable::from_descriptors(descriptors);
    descriptors
        .get(table.descriptor_index_for_tool(tool_name)?)
        .map(|descriptor| descriptor.name.as_str())
}

/// Truncated SHA-256 used to disambiguate two canonical ability names
/// that project onto the same MCP tool name after the
/// `[^A-Za-z0-9_-]` rewrite. 8 bytes (16 hex chars) gives a 2^64
/// space — the birthday bound sits past 2^32 distinct ability names,
/// well beyond any plausible federation catalogue. 4 bytes (the v0
/// width) hit birthday collisions at ~10^5 abilities, which a multi-
/// device federation will reach.
fn short_ability_hash(ability_name: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(ability_name.as_bytes());
    hex::encode(&digest[..8])
}

/// The identity trace a successful daemon invocation echoes back, folded
/// into the tool-call result under the `x-easynet-invocation` key so an
/// EasyNet-aware driver (Claude Code, Codex) can correlate the tool call
/// with the ledger. The field set mirrors what
/// `drivers::invocation_trace::parse_invocation_trace_metadata` consumes.
#[derive(Debug, Clone, Default)]
pub struct InvocationToolTrace {
    pub ability: String,
    pub mcp_tool: String,
    pub request_id: Option<String>,
    pub ability_ura: Option<String>,
    pub invocation_ura: Option<String>,
    pub caller_ura: Option<String>,
    pub callee_ura: Option<String>,
    pub subject_ura: Option<String>,
}

impl InvocationToolTrace {
    /// Project the daemon invocation `_meta` echo (see
    /// `local_daemon_grpc::invoke_local_daemon_ability_targeted_with_invocation_meta`)
    /// onto the driver-facing trace object.
    fn from_daemon_meta(
        meta: &crate::support::platform::local_invoke::VerifiedLocalInvocationMeta,
        mcp_tool: &str,
    ) -> Self {
        let meta = meta.as_value();
        let field = |key: &str| meta.get(key).and_then(|v| v.as_str()).map(str::to_string);
        Self {
            ability: field("ability").unwrap_or_default(),
            mcp_tool: mcp_tool.to_string(),
            request_id: field("request_id"),
            ability_ura: field("ability_ura"),
            invocation_ura: field("invocation_ura"),
            caller_ura: field("caller_ura"),
            callee_ura: field("callee_ura"),
            subject_ura: field("subject_ura"),
        }
    }

    fn into_value(self) -> serde_json::Value {
        let mut obj = serde_json::Map::new();
        obj.insert("ability".into(), self.ability.into());
        obj.insert("mcp_tool".into(), self.mcp_tool.into());
        let mut put = |key: &str, v: Option<String>| {
            if let Some(v) = v {
                obj.insert(key.into(), v.into());
            }
        };
        put("request_id", self.request_id);
        put("ability_ura", self.ability_ura);
        put("invocation_ura", self.invocation_ura);
        put("caller_ura", self.caller_ura);
        put("callee_ura", self.callee_ura);
        put("subject_ura", self.subject_ura);
        serde_json::Value::Object(obj)
    }
}

/// The trait the MCP provider needs from a local ability invoker.
/// Production wires this to daemon.sock Axon Invocation; tests inject
/// fakes without depending on daemon transport.
pub trait LocalInvoker {
    /// Invoke the named ability synchronously and return the raw
    /// Result frame's value, or an error message.
    fn invoke_sync(
        &self,
        target: &crate::daemon::invocation::routing::target::LocalAbilityTarget,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String>;

    /// Invoke and additionally surface the invocation identity trace.
    /// The default implementation carries no trace; the production daemon
    /// adapter overrides it so tool results can echo the ledger identity.
    /// `mcp_tool` is the wire tool name the driver matches against.
    fn invoke_traced(
        &self,
        target: &crate::daemon::invocation::routing::target::LocalAbilityTarget,
        _mcp_tool: &str,
        args: serde_json::Value,
    ) -> Result<(serde_json::Value, Option<InvocationToolTrace>), String> {
        self.invoke_sync(target, args).map(|value| (value, None))
    }
}

/// Production adapter for `easynet mcp serve`: route every tool call
/// through the live local daemon's Axon Invocation gRPC surface
/// instead of through an isolated in-process kernel snapshot.
pub struct DaemonLocalInvoker;

impl DaemonLocalInvoker {
    fn invoke_verified(
        &self,
        target: &crate::daemon::invocation::routing::target::LocalAbilityTarget,
        args: serde_json::Value,
    ) -> anyhow::Result<(
        serde_json::Value,
        crate::support::platform::local_invoke::VerifiedLocalInvocationMeta,
    )> {
        let context = crate::support::platform::local_invoke::LocalSystemInvocationContext::new(
            target.default_subject_ura(),
            axon_sdk::invocation::fresh_nonce(),
            &[],
            std::time::Duration::from_secs(30),
            None,
        )?;
        crate::support::platform::local_invoke::invoke_local_ability_target_with_invocation_meta(
            target, args, context,
        )
    }
}

impl LocalInvoker for DaemonLocalInvoker {
    fn invoke_sync(
        &self,
        target: &crate::daemon::invocation::routing::target::LocalAbilityTarget,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.invoke_verified(target, args)
            .map(|(value, _)| value)
            .map_err(|err| err.to_string())
    }

    fn invoke_traced(
        &self,
        target: &crate::daemon::invocation::routing::target::LocalAbilityTarget,
        mcp_tool: &str,
        args: serde_json::Value,
    ) -> Result<(serde_json::Value, Option<InvocationToolTrace>), String> {
        let (value, meta) = self
            .invoke_verified(target, args)
            .map_err(|err| err.to_string())?;
        Ok((
            value,
            Some(InvocationToolTrace::from_daemon_meta(&meta, mcp_tool)),
        ))
    }
}

/// Production InvokeMcpProvider — what `easynet mcp_server` and
/// `easynet start --mcp` use after the P4.8d facade retirement. Every
/// `tools/list` returns the host's AbilityDescriptors projected to
/// MCP shape; every `tools/call` routes through
/// `LocalInvoker::invoke_traced`, which production wires to daemon.sock Axon
/// Invoke and verified finalization metadata. Zero direct bridge calls; zero
/// hub-mediated MCP tool catalog.
pub struct InvokeMcpProvider<I: LocalInvoker> {
    invoker: I,
    /// Atomic snapshot returned by the daemon's live catalog at construction.
    /// The provider and route table retain this exact snapshot so tools/list
    /// and tools/call cannot observe different descriptor generations.
    descriptors: Vec<crate::daemon::ability::descriptors::AbilityDescriptor>,
    /// Tool-name routing built from `descriptors` at construction.
    /// Kept paired with `descriptors` via the constructor — this is
    /// the only place that builds the table for the provider, so
    /// the two fields cannot drift.
    routes: McpToolRouteTable,
}

impl<I: LocalInvoker> InvokeMcpProvider<I> {
    pub fn new(
        invoker: I,
        descriptors: Vec<crate::daemon::ability::descriptors::AbilityDescriptor>,
    ) -> Self {
        let routes = McpToolRouteTable::from_descriptors(&descriptors);
        Self {
            invoker,
            descriptors,
            routes,
        }
    }

    /// Number of descriptors the provider will surface in tools/list.
    /// Used by tests + by `easynet mcp_server`'s startup banner.
    pub fn descriptor_count(&self) -> usize {
        self.routes.len()
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
    /// Workspace agent label. The daemon catalog remains the only descriptor
    /// authority; the label is used solely to suppress recursive chat tools in
    /// an agent's own MCP surface.
    pub agent_name: Option<String>,
}

trait AbilityCatalogReader {
    fn read(&self) -> anyhow::Result<Vec<crate::daemon::ability::descriptors::AbilityDescriptor>>;
}

struct DaemonAbilityCatalogReader;

impl AbilityCatalogReader for DaemonAbilityCatalogReader {
    fn read(&self) -> anyhow::Result<Vec<crate::daemon::ability::descriptors::AbilityDescriptor>> {
        use anyhow::Context;

        let response = crate::support::platform::local_invoke::invoke_local_ability(
            "meta.list_abilities",
            serde_json::json!({"scope": "local"}),
        )
        .context("read live daemon ability catalog for MCP")?;
        let rows = response
            .get("abilities")
            .and_then(serde_json::Value::as_array)
            .context("meta.list_abilities response missing abilities array")?;
        let mut descriptors = Vec::with_capacity(rows.len());
        for row in rows {
            let descriptor = serde_json::from_value::<
                crate::daemon::ability::descriptors::AbilityDescriptor,
            >(row.clone())
            .context("decode live daemon AbilityDescriptor for MCP")?;
            descriptors.push(descriptor);
        }

        Ok(descriptors)
    }
}

/// Pre-built provider + server name, ready to hand to
/// `crate::daemon::execution::mcp::stdio::StdioMcpServer::new`. Returned by
/// `build_stdio_server` so callers can decide whether to run
/// foreground (mcp_server) or in a spawned thread (start --mcp).
pub struct ConfiguredStdioServer {
    pub provider: InvokeMcpProvider<DaemonLocalInvoker>,
    pub server_name: String,
}

impl ConfiguredStdioServer {
    /// Number of MCP tools the configured provider advertises.
    /// Convenience getter so callers don't reach into provider state.
    pub fn descriptor_count(&self) -> usize {
        self.provider.descriptor_count()
    }
}

/// One-stop builder: read the live daemon's authoritative ability catalog and
/// produce a configured InvokeMcpProvider ready for the stdio runner.
///
/// Both `easynet mcp_server` and `easynet start --mcp` call this
/// — they differ only in argument parsing and how they launch the
/// stdio server (foreground vs. spawned thread).
pub fn build_stdio_server(config: &StdioServerConfig) -> anyhow::Result<ConfiguredStdioServer> {
    build_stdio_server_with_catalog(config, &DaemonAbilityCatalogReader)
}

fn build_stdio_server_with_catalog(
    config: &StdioServerConfig,
    catalog: &dyn AbilityCatalogReader,
) -> anyhow::Result<ConfiguredStdioServer> {
    let mut descriptors = catalog.read()?;
    if config.agent_name.is_some() {
        descriptors.retain(|descriptor| {
            let owner_is_agent = crate::core::ura::parse_ura(&descriptor.owner_ura)
                .is_ok_and(|owner| owner.kind == crate::core::ura::URAKind::Agent);
            !(owner_is_agent && descriptor.public_name() == "chat")
        });
    }
    let invoker = DaemonLocalInvoker;
    let provider = InvokeMcpProvider::new(invoker, descriptors);
    Ok(ConfiguredStdioServer {
        provider,
        server_name: config.server_name.clone(),
    })
}

/// Fold the invocation identity trace into a successful tool-call payload
/// under the `x-easynet-invocation` key the drivers parse. An object
/// payload gains the key in place; a scalar/array payload is wrapped as
/// `{ "result": <payload>, "x-easynet-invocation": {...} }` so the trace
/// has a home without losing the original value. No trace → payload
/// untouched.
fn fold_invocation_trace(
    payload: serde_json::Value,
    trace: Option<InvocationToolTrace>,
) -> serde_json::Value {
    let Some(trace) = trace else {
        return payload;
    };
    let trace_value = trace.into_value();
    match payload {
        serde_json::Value::Object(mut map) => {
            map.insert("x-easynet-invocation".into(), trace_value);
            serde_json::Value::Object(map)
        }
        other => serde_json::json!({
            "result": other,
            "x-easynet-invocation": trace_value,
        }),
    }
}

impl<I: LocalInvoker> McpToolProvider for InvokeMcpProvider<I> {
    fn tool_specs(&self) -> Vec<serde_json::Value> {
        self.routes
            .iter()
            .map(|(tool_name, index)| {
                tool_spec_from_descriptor_with_name(&self.descriptors[index], tool_name)
            })
            .collect()
    }

    fn handle_tool_call(
        &self,
        name: &str,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> ToolResult {
        let target = self.routes.target_for_tool(name);

        // Reject calls for tools we don't advertise. The descriptor
        // list is the single source of truth — if a name isn't
        // there, this is a caller-side bug, not a transient.
        let Some(target) = target else {
            return ToolResult {
                payload: serde_json::json!({
                    "error": format!("unknown tool: `{name}`")
                }),
                is_error: true,
            };
        };
        let args_value = serde_json::Value::Object(args.clone());
        match self.invoker.invoke_traced(target, name, args_value) {
            Ok((value, trace)) => ToolResult {
                payload: fold_invocation_trace(value, trace),
                is_error: false,
            },
            Err(msg) => ToolResult {
                payload: serde_json::json!({"error": msg}),
                is_error: true,
            },
        }
    }

    /// Progress-aware variant. v1 (round-2 of the plan, slice B2a):
    /// emits at least one progress notification BEFORE diving into
    /// the unary `handle_tool_call` path — gives the client a
    /// "received, working on it" signal even when the underlying
    /// ability is unary. Real stream-ability projection (drive
    /// InvokeStream chunks → progress notifications, terminal
    /// chunk → response) is plan slice B2b, which lands once the
    /// `LocalInvoker` trait gains a stream-aware method.
    ///
    /// The wire shape is spec-correct today: client sees N progress
    /// notifications (currently N=1) + one terminal tools/call
    /// response carrying the full payload. The contract is forward-
    /// compatible — providers calling `handle_tool_call_with_progress`
    /// won't break when B2b lifts the progress count to per-chunk.
    fn handle_tool_call_with_progress(
        &self,
        name: &str,
        args: &serde_json::Map<String, serde_json::Value>,
        sink: &mut dyn ProgressSink,
    ) -> ToolResult {
        // Single "received" pulse so the client gets immediate
        // ack-of-progress even before the unary handler returns.
        // The throttle in WriterProgressSink ensures this never
        // floods. Value is intentionally tiny-but-positive so the
        // strict-increase check against the terminal 1.0 passes
        // regardless of float precision.
        let _ = sink.report(f64::EPSILON, Some(1.0), Some("dispatched"));
        let result = self.handle_tool_call(name, args);
        // Terminal "complete" pulse, monotonically increasing past
        // the start. Spec REQUIRES strict increase.
        let _ = sink.report(1.0, Some(1.0), Some("complete"));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::descriptors::{AbilityDescriptor, AdmissionAction, Visibility};
    use std::cell::{Cell, RefCell};

    const TEST_DEVICE_OWNER: &str = "easynet:///r/acme/device/01DEV";
    const TEST_AGENT_OWNER: &str = "easynet:///r/acme/agent/test-user.claude";

    #[test]
    fn descriptors_follow_registry_owner() {
        let descriptors = descriptors_for("easynet:///r/acme/agent/u1.01MCP");
        let names = descriptors
            .iter()
            .map(|d| d.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(names.contains("mcp.bridge.list_tools"));
        assert!(names.contains("mcp.bridge.call_tool"));
        assert!(names.contains("mcp.client.list"));
        assert!(names.contains("mcp.client.call"));
        assert!(!names.contains("mcp.evaluate"));
        assert!(!names.contains("skill.list"));
        assert!(!names.contains("consent.subscribe"));
    }

    fn d(name: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(
            name,
            "easynet:///r/acme/device/01DEV",
            Visibility::Scoped,
            AdmissionAction::Invoke,
        )
        .unwrap()
        .with_source("kernel:built-in")
        .with_input_schema(serde_json::json!({"type":"object"}))
        .with_description("List every registered agent on this host.")
    }

    struct FixtureAbilityCatalog {
        descriptors: Vec<AbilityDescriptor>,
        reads: Cell<usize>,
    }

    impl FixtureAbilityCatalog {
        fn new(descriptors: Vec<AbilityDescriptor>) -> Self {
            Self {
                descriptors,
                reads: Cell::new(0),
            }
        }
    }

    impl AbilityCatalogReader for FixtureAbilityCatalog {
        fn read(&self) -> anyhow::Result<Vec<AbilityDescriptor>> {
            self.reads.set(self.reads.get() + 1);
            Ok(self.descriptors.clone())
        }
    }

    #[test]
    fn tool_spec_from_descriptor_emits_mcp_shape() {
        let spec = tool_spec_from_descriptor(&d("agent.list"));
        assert_eq!(spec["name"], "agent_list");
        assert_eq!(spec["x-easynet"]["ability"], "agent.list");
        // The MCP description is the human blurb from the registry,
        // NOT the provenance string. Pre-fix this asserted the
        // opposite — bug pinned upside-down. Updated when the
        // transform started reading descriptor.description.
        // Cost defaults to `unknown` when the descriptor carries no
        // `cost_kind` metadata — the inferred fallback used to lie
        // and return "free" for every catalog row that wasn't an
        // agent-chat surface, including billed upstream MCP tools.
        // See `inferred_cost_kind` doc for the honesty rationale.
        assert!(spec["description"].as_str().unwrap().starts_with(
            "[EasyNet ability: agent.list | owner: device/01DEV | cost: unknown (cost not declared)] "
        ));
        assert!(spec["description"]
            .as_str()
            .unwrap()
            .contains("List every registered agent on this host."));
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
        let desc = AbilityDescriptor::new(
            "a.b",
            TEST_DEVICE_OWNER,
            Visibility::Public,
            AdmissionAction::Invoke,
        )
        .unwrap()
        .with_input_schema(serde_json::json!({"type":"object"}));
        let spec = tool_spec_from_descriptor(&desc);
        assert!(spec["description"].as_str().unwrap().ends_with("] a.b"));
        assert_eq!(spec["name"], "a_b");
    }

    #[test]
    fn tool_spec_falls_back_to_object_schema_when_input_is_null() {
        let mut desc = AbilityDescriptor::new(
            "a.b",
            TEST_DEVICE_OWNER,
            Visibility::Public,
            AdmissionAction::Invoke,
        )
        .unwrap();
        desc.schema_summary.input = serde_json::Value::Null;
        let spec = tool_spec_from_descriptor(&desc);
        assert_eq!(spec["inputSchema"]["type"], "object");
    }

    #[test]
    fn mcp_tool_name_for_ability_projects_dotted_names_to_function_names() {
        assert_eq!(
            mcp_tool_name_for_ability("openai.mcp_unit_converter__convert_length"),
            "openai_mcp_unit_converter__convert_length"
        );
        assert_eq!(
            mcp_tool_name_for_ability("ability.publish"),
            "ability_publish"
        );
        assert_eq!(mcp_tool_name_for_ability("..."), "ability");
    }

    #[test]
    fn tool_spec_surfaces_owner_and_cost_metadata_for_agent_mcp_ability() {
        let desc = AbilityDescriptor::new(
            "openai.mcp_google_maps__geocode",
            "easynet:///r/acme/agent/silan.openai",
            Visibility::Scoped,
            AdmissionAction::Invoke,
        )
        .unwrap()
        .with_description("Geocode an address.")
        .with_source("agent:openai")
        .with_metadata_entry("owner_user", "silan")
        .with_metadata_entry("owner_agent", "openai")
        .with_metadata_entry("exec_kind", "mcp")
        .with_metadata_entry("mcp_server", "Google Maps")
        .with_metadata_entry("mcp_tool", "geocode")
        .with_metadata_entry("cost_kind", "external_metered")
        .with_metadata_entry("cost_label", "Google Maps/API billing may apply");
        let spec = tool_spec_from_descriptor(&desc);
        assert_eq!(spec["x-easynet"]["owner_user"], "silan");
        assert_eq!(spec["x-easynet"]["owner_agent"], "openai");
        assert_eq!(spec["x-easynet"]["exec_kind"], "mcp");
        assert_eq!(spec["x-easynet"]["mcp_server"], "Google Maps");
        assert_eq!(spec["x-easynet"]["mcp_tool"], "geocode");
        assert_eq!(spec["x-easynet"]["cost_kind"], "external_metered");
        let description = spec["description"].as_str().unwrap();
        assert!(description.contains("owner: user/silan agent/openai"));
        assert!(description.contains("cost: external_metered"));
        assert!(description.contains("Geocode an address."));
    }

    /// Recording fake invoker that asserts the proxy contract: the
    /// MCP provider must hand the raw ability_name + args bag to the
    /// dispatcher and surface its result verbatim (or its error).
    struct RecordingInvoker {
        last_ability: RefCell<Option<String>>,
        last_callee_ura: RefCell<Option<String>>,
        last_args: RefCell<Option<serde_json::Value>>,
        reply: Result<serde_json::Value, String>,
    }
    impl RecordingInvoker {
        fn new(reply: Result<serde_json::Value, String>) -> Self {
            Self {
                last_ability: RefCell::new(None),
                last_callee_ura: RefCell::new(None),
                last_args: RefCell::new(None),
                reply,
            }
        }
    }
    impl LocalInvoker for RecordingInvoker {
        fn invoke_sync(
            &self,
            target: &crate::daemon::invocation::routing::target::LocalAbilityTarget,
            args: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            *self.last_ability.borrow_mut() = Some(target.dispatch_name().to_string());
            *self.last_callee_ura.borrow_mut() = Some(target.callee_ura().to_string());
            *self.last_args.borrow_mut() = Some(args);
            self.reply.clone()
        }
    }

    #[test]
    fn tool_specs_lists_every_rpc_descriptor_passed_at_construction() {
        let descs = vec![d("observe.health"), d("agent.list")];
        let p = InvokeMcpProvider::new(RecordingInvoker::new(Ok(serde_json::json!({}))), descs);
        let specs = p.tool_specs();
        assert_eq!(specs.len(), 2);
        let names: Vec<&str> = specs.iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"observe_health"));
        assert!(names.contains(&"agent_list"));
    }

    #[test]
    fn provider_excludes_geometries_it_cannot_invoke() {
        let descriptors = vec![
            d("observe.health"),
            d("consent.subscribe")
                .with_call_mode(crate::daemon::ability::descriptors::CallMode::Stream),
            d("voice.session").with_call_mode(crate::daemon::ability::descriptors::CallMode::Bidi),
        ];
        let provider = InvokeMcpProvider::new(
            RecordingInvoker::new(Ok(serde_json::json!({}))),
            descriptors,
        );

        let specs = provider.tool_specs();
        assert_eq!(provider.descriptor_count(), 1);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0]["x-easynet"]["ability"], "observe.health");
        assert!(
            provider
                .handle_tool_call("consent_subscribe", &serde_json::Map::new())
                .is_error
        );
        assert!(
            provider
                .handle_tool_call("voice_session", &serde_json::Map::new())
                .is_error
        );
    }

    #[test]
    fn tools_list_owner_is_tools_call_callee() {
        let desc = AbilityDescriptor::new(
            "discover",
            TEST_AGENT_OWNER,
            Visibility::Scoped,
            AdmissionAction::Invoke,
        )
        .unwrap()
        .with_source("kernel:built-in:self-bundle")
        .with_input_schema(crate::daemon::ability::builtins::agents::discover::input_schema())
        .with_description(crate::daemon::ability::builtins::agents::discover::description());
        let invoker = RecordingInvoker::new(Ok(serde_json::json!({
            "candidates": [],
            "scope": "device",
            "query": "weather"
        })));
        let p = InvokeMcpProvider::new(invoker, vec![desc]);

        let specs = p.tool_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0]["name"], "claude_discover");
        assert_eq!(specs[0]["x-easynet"]["ability"], "discover");
        assert_eq!(specs[0]["x-easynet"]["owner_ura"], TEST_AGENT_OWNER);
        assert_eq!(
            specs[0]["inputSchema"]["properties"]["scope"]["enum"],
            serde_json::json!(["self", "device", "user", "public"])
        );

        let mut args = serde_json::Map::new();
        args.insert("scope".into(), serde_json::json!("device"));
        args.insert("query".into(), serde_json::json!("weather"));
        let result = p.handle_tool_call("claude_discover", &args);
        assert!(!result.is_error);
        assert_eq!(result.payload["scope"], "device");
        assert_eq!(
            p.invoker.last_ability.borrow().as_deref(),
            Some("claude.discover")
        );
        assert_eq!(
            p.invoker.last_callee_ura.borrow().as_deref(),
            Some(TEST_AGENT_OWNER),
            "tools/call must use the same owner advertised by tools/list"
        );
        assert_eq!(
            p.invoker.last_args.borrow().as_ref().unwrap()["query"],
            serde_json::json!("weather")
        );
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
    fn client_safe_tool_call_is_dispatched_to_canonical_ability() {
        let invoker = RecordingInvoker::new(Ok(serde_json::json!({"status": "healthy"})));
        let p = InvokeMcpProvider::new(invoker, vec![d("observe.health")]);
        let mut args = serde_json::Map::new();
        args.insert("foo".into(), serde_json::Value::Bool(true));
        let result = p.handle_tool_call("observe_health", &args);
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
    fn mcp_bench_agent_tool_is_advertised_and_dispatched_as_direct_tool() {
        let invoker = RecordingInvoker::new(Ok(serde_json::json!({"converted": 1.0})));
        let p = InvokeMcpProvider::new(
            invoker,
            vec![d("openai.mcp_unit_converter__convert_length")],
        );
        let specs = p.tool_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(
            specs[0]["name"],
            "openai_mcp_unit_converter__convert_length"
        );
        assert_eq!(
            specs[0]["x-easynet"]["ability"],
            "openai.mcp_unit_converter__convert_length"
        );

        let result = p.handle_tool_call(
            "openai_mcp_unit_converter__convert_length",
            &serde_json::Map::new(),
        );
        assert!(!result.is_error);
        assert_eq!(
            p.invoker.last_ability.borrow().as_deref(),
            Some("openai.mcp_unit_converter__convert_length")
        );
    }

    #[test]
    fn canonical_dotted_tool_call_is_rejected() {
        let invoker = RecordingInvoker::new(Ok(serde_json::json!({"status": "healthy"})));
        let p = InvokeMcpProvider::new(invoker, vec![d("observe.health")]);
        let result = p.handle_tool_call("observe.health", &serde_json::Map::new());
        assert!(result.is_error);
        assert!(p.invoker.last_ability.borrow().is_none());
    }

    #[test]
    fn colliding_client_safe_tool_names_get_stable_suffixes() {
        let descs = vec![d("a.b.c"), d("a.b_c")];
        let specs = tool_specs_from_descriptors(&descs);
        let names: Vec<&str> = specs.iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert_eq!(names.len(), 2);
        assert_eq!(names[0], "a_b_c");
        assert!(
            names[1].starts_with("a_b_c__"),
            "second colliding tool should get a hash suffix, got {names:?}"
        );
        assert_eq!(
            canonical_ability_name_for_mcp_tool(&descs, names[0]),
            Some("a.b.c")
        );
        assert_eq!(
            canonical_ability_name_for_mcp_tool(&descs, names[1]),
            Some("a.b_c")
        );
    }

    #[test]
    fn invoker_error_surfaces_as_error_payload() {
        let invoker = RecordingInvoker::new(Err("policy denied".into()));
        let p = InvokeMcpProvider::new(invoker, vec![d("observe.health")]);
        let result = p.handle_tool_call("observe_health", &serde_json::Map::new());
        assert!(result.is_error);
        assert!(result.payload["error"]
            .as_str()
            .unwrap()
            .contains("policy denied"));
    }

    #[test]
    fn descriptor_count_matches_callable_routes() {
        let invoker = RecordingInvoker::new(Ok(serde_json::json!({})));
        let p = InvokeMcpProvider::new(
            invoker,
            vec![
                d("observe.health"),
                d("agent.list"),
                d("consent.subscribe")
                    .with_call_mode(crate::daemon::ability::descriptors::CallMode::Stream),
            ],
        );
        assert_eq!(p.descriptor_count(), 2);
    }

    #[test]
    fn daemon_local_invoker_surfaces_daemon_not_running() {
        let _h = crate::cli::commands::test_support::HomeGuard::new();
        let descriptor = d("observe.health");
        let selector =
            crate::core::ura::AbilitySelector::parse(&descriptor.canonical_ability_ura().unwrap())
                .unwrap();
        let target = crate::daemon::invocation::routing::target::LocalAbilityTarget::from_selector(
            &selector,
        );
        let err = DaemonLocalInvoker
            .invoke_sync(&target, serde_json::json!({}))
            .expect_err("daemon-backed invoker must fail when no daemon is running");
        assert!(
            err.contains("daemon not running"),
            "expected actionable daemon-down error; got {err}"
        );
    }

    #[test]
    fn build_stdio_server_fails_closed_when_daemon_is_unavailable() {
        let _h = crate::cli::commands::test_support::HomeGuard::new();
        let cfg = StdioServerConfig {
            server_name: "easynet-test".into(),
            tenant_id: "test-tenant".into(),
            agent_name: None,
        };
        let error = match build_stdio_server(&cfg) {
            Ok(_) => panic!("MCP builder must not synthesize a catalog when daemon is unavailable"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("read live daemon ability catalog for MCP"),
            "unexpected daemon-unavailable error: {error:#}"
        );
    }

    #[test]
    fn live_catalog_reader_input_is_projected_without_reconstruction() {
        let input_schema = serde_json::json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
            "additionalProperties": false
        });
        let descriptor = AbilityDescriptor::new(
            "review",
            TEST_AGENT_OWNER,
            Visibility::Scoped,
            AdmissionAction::Invoke,
        )
        .unwrap()
        .with_description("Review one change.")
        .with_source("manifest:workspace/abilities/review.ability.toml")
        .with_input_schema(input_schema.clone())
        .with_metadata_entry("owner_user", "test-user")
        .with_metadata_entry("owner_agent", "claude")
        .with_metadata_entry("exec_kind", "eal")
        .with_metadata_entry("cost_kind", "unknown")
        .with_metadata_entry("cost_label", "composed ability cost depends on steps");
        let catalog = FixtureAbilityCatalog::new(vec![descriptor.clone()]);
        let cfg = StdioServerConfig {
            server_name: "easynet-test".into(),
            tenant_id: "t".into(),
            agent_name: None,
        };
        let configured = build_stdio_server_with_catalog(&cfg, &catalog).unwrap();

        assert_eq!(catalog.reads.get(), 1, "catalog must be captured once");
        assert_eq!(configured.provider.descriptors, vec![descriptor]);
        let specs = configured.provider.tool_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0]["name"], "claude_review");
        assert_eq!(specs[0]["inputSchema"], input_schema);
        assert_eq!(specs[0]["x-easynet"]["owner_ura"], TEST_AGENT_OWNER);
        assert_eq!(specs[0]["x-easynet"]["owner_user"], "test-user");
        assert_eq!(specs[0]["x-easynet"]["owner_agent"], "claude");
        assert_eq!(specs[0]["x-easynet"]["exec_kind"], "eal");
        assert_eq!(specs[0]["x-easynet"]["cost_kind"], "unknown");
    }

    #[test]
    fn workspace_catalog_filters_agent_chat_without_rebuilding_live_descriptors() {
        let own_chat = AbilityDescriptor::new(
            "chat",
            TEST_AGENT_OWNER,
            Visibility::Scoped,
            AdmissionAction::Invoke,
        )
        .unwrap();
        let other_chat = AbilityDescriptor::new(
            "chat",
            "easynet:///r/acme/agent/test-user.codex",
            Visibility::Scoped,
            AdmissionAction::Invoke,
        )
        .unwrap();
        let discover = AbilityDescriptor::new(
            "discover",
            TEST_AGENT_OWNER,
            Visibility::Scoped,
            AdmissionAction::Invoke,
        )
        .unwrap();
        let catalog = FixtureAbilityCatalog::new(vec![own_chat, other_chat, discover.clone()]);
        let cfg = StdioServerConfig {
            server_name: "easynet-test".into(),
            tenant_id: "t".into(),
            agent_name: Some("claude".into()),
        };
        let configured = build_stdio_server_with_catalog(&cfg, &catalog).unwrap();
        assert_eq!(catalog.reads.get(), 1);
        assert_eq!(configured.provider.descriptors, vec![discover]);
        assert_eq!(
            configured.provider.tool_specs()[0]["name"],
            "claude_discover"
        );
    }

    #[test]
    fn invoke_provider_routes_observe_health_through_local_invoker() {
        use crate::daemon::ability::descriptors::Visibility;
        let invoker = FakeInvoker {
            value: serde_json::json!({"echo": {}}),
        };

        let descs = vec![AbilityDescriptor::new(
            "observe.health",
            "easynet:///r/acme/device/01DEV",
            Visibility::Public,
            AdmissionAction::Invoke,
        )
        .unwrap()];
        let provider = InvokeMcpProvider::new(invoker, descs);

        // tools/list mirrors the descriptor.
        let specs = provider.tool_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0]["name"], "observe_health");
        assert_eq!(specs[0]["x-easynet"]["ability"], "observe.health");

        // tools/call dispatches through the provider's local invoker.
        let result = provider.handle_tool_call("observe_health", &serde_json::Map::new());
        assert!(
            !result.is_error,
            "observe.health must succeed end-to-end through MCP shim"
        );
    }

    /// Minimal invoker that just returns a fixed value so we can
    /// exercise the progress-aware dispatch path without spinning
    /// up the whole proxy stack.
    struct FakeInvoker {
        value: serde_json::Value,
    }
    impl LocalInvoker for FakeInvoker {
        fn invoke_sync(
            &self,
            _target: &crate::daemon::invocation::routing::target::LocalAbilityTarget,
            _args: serde_json::Value,
        ) -> std::result::Result<serde_json::Value, String> {
            Ok(self.value.clone())
        }
    }

    /// Counts progress reports — used to assert the InvokeMcpProvider
    /// emits the expected per-call pulses.
    type CountingReports =
        std::sync::Arc<std::sync::Mutex<Vec<(f64, Option<f64>, Option<String>)>>>;

    struct CountingSink {
        reports: CountingReports,
    }
    impl ProgressSink for CountingSink {
        fn report(
            &mut self,
            progress: f64,
            total: Option<f64>,
            message: Option<&str>,
        ) -> anyhow::Result<crate::daemon::execution::mcp::stdio::ReportOutcome> {
            self.reports
                .lock()
                .unwrap()
                .push((progress, total, message.map(|s| s.to_string())));
            Ok(crate::daemon::execution::mcp::stdio::ReportOutcome::Emitted)
        }
    }

    #[test]
    fn handle_tool_call_with_progress_emits_dispatched_and_complete_pulses() {
        let provider = InvokeMcpProvider::new(
            FakeInvoker {
                value: serde_json::json!({"ok": true}),
            },
            vec![AbilityDescriptor::new(
                "observe.health",
                "easynet:///r/acme/device/01DEV",
                Visibility::Public,
                AdmissionAction::Invoke,
            )
            .unwrap()],
        );
        let reports = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut sink = CountingSink {
            reports: std::sync::Arc::clone(&reports),
        };
        let out = provider.handle_tool_call_with_progress(
            "observe_health",
            &serde_json::Map::new(),
            &mut sink,
        );
        assert!(!out.is_error);
        assert_eq!(out.payload, serde_json::json!({"ok": true}));
        let r = reports.lock().unwrap();
        // Exactly two pulses: a strictly-positive "dispatched"
        // first, then a strictly-greater "complete". The values
        // are intentionally not pinned to specific floats — the
        // contract is "two strictly-increasing reports".
        assert_eq!(
            r.len(),
            2,
            "expected exactly two progress pulses, got: {r:?}"
        );
        assert!(r[0].0 > 0.0);
        assert!(r[1].0 > r[0].0, "second pulse must strictly increase");
        assert_eq!(r[0].2.as_deref(), Some("dispatched"));
        assert_eq!(r[1].2.as_deref(), Some("complete"));
    }

    #[test]
    fn handle_tool_call_with_progress_falls_through_to_unary_payload() {
        // Even when the unary handler returns an error, the
        // progress-aware path must still emit pulses + propagate
        // the error verbatim. Important because real upstreams
        // hit error paths and the client still wants the
        // dispatched/complete pulses for UX consistency.
        let provider = InvokeMcpProvider::new(
            FakeInvoker {
                value: serde_json::Value::Null,
            },
            vec![AbilityDescriptor::new(
                "observe.health",
                "easynet:///r/acme/device/01DEV",
                Visibility::Public,
                AdmissionAction::Invoke,
            )
            .unwrap()],
        );
        let reports = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut sink = CountingSink {
            reports: std::sync::Arc::clone(&reports),
        };
        // Call an unknown tool → unary handler returns is_error: true.
        let out = provider.handle_tool_call_with_progress(
            "device.does.not.exist",
            &serde_json::Map::new(),
            &mut sink,
        );
        assert!(out.is_error, "unknown tool surfaces as error");
        // But progress pulses still went out.
        assert_eq!(reports.lock().unwrap().len(), 2);
    }
}

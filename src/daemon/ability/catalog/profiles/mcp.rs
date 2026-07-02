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

/// AbilityDescriptors for every mcp.bridge.* + mcp.client.* in the
/// live registry, anchored to the mcp-profile's canonical URA. All
/// SCOPED per §18 — local MCP clients only for bridge.*; the daemon
/// itself + selected internal callers for client.*. P4.7 narrows.
pub fn descriptors_for(
    owner_ura: &str,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    use crate::runtime::ability_descriptor::Visibility;
    use crate::runtime::ability_dispatch::OwnerKind;

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
    descriptor: &crate::runtime::ability_descriptor::AbilityDescriptor,
) -> serde_json::Value {
    tool_spec_from_descriptor_with_name(descriptor, &mcp_tool_name_for_ability(&descriptor.name))
}

fn tool_spec_from_descriptor_with_name(
    descriptor: &crate::runtime::ability_descriptor::AbilityDescriptor,
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
    descriptor: &crate::runtime::ability_descriptor::AbilityDescriptor,
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
    descriptor: &crate::runtime::ability_descriptor::AbilityDescriptor,
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
    let parsed = crate::ura::parse_ura(owner_ura).ok()?;
    match parsed.kind {
        crate::ura::URAKind::Agent => parsed
            .agent_ids()
            .map(|(user_id, agent_id)| format!("user/{user_id} agent/{agent_id}")),
        crate::ura::URAKind::Ability => match parsed.ability()?.owner {
            crate::ura::AbilityOwner::Agent { user_id, agent_id } => {
                Some(format!("user/{user_id} agent/{agent_id}"))
            }
            crate::ura::AbilityOwner::Device { device_id } => Some(format!("device/{device_id}")),
            crate::ura::AbilityOwner::Hub => Some("hub".to_string()),
        },
        crate::ura::URAKind::User => parsed.user_id().map(|user_id| format!("user/{user_id}")),
        crate::ura::URAKind::Device => parsed
            .device_id()
            .map(|device_id| format!("device/{device_id}")),
        crate::ura::URAKind::Hub => Some("hub".to_string()),
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
    descriptor: &crate::runtime::ability_descriptor::AbilityDescriptor,
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
    descriptors: &[crate::runtime::ability_descriptor::AbilityDescriptor],
) -> Vec<serde_json::Value> {
    let table = McpToolRouteTable::from_descriptors(descriptors);
    table
        .iter()
        .map(|(tool_name, index)| {
            tool_spec_from_descriptor_with_name(&descriptors[index], tool_name)
        })
        .collect()
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
    /// Canonical dotted EasyNet ability name (the runtime registry key
    /// and URA tail).
    ability_name: String,
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
    /// MCP tool name → canonical ability name. Hot path for
    /// `call_tool` dispatch.
    reverse: std::collections::BTreeMap<String, String>,
}

impl McpToolRouteTable {
    /// Build the routing table for a descriptor slice. The descriptor
    /// order determines the deterministic tie-break order if two
    /// canonical names project to the same MCP tool name; that
    /// ordering matches the pre-refactor behaviour.
    pub fn from_descriptors(
        descriptors: &[crate::runtime::ability_descriptor::AbilityDescriptor],
    ) -> Self {
        let mut routes = Vec::with_capacity(descriptors.len());
        let mut used: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();

        for (index, descriptor) in descriptors.iter().enumerate() {
            let base = mcp_tool_name_for_ability(&descriptor.name);
            let mut tool_name = match used.get(&base) {
                None => base.clone(),
                Some(existing) if existing == &descriptor.name => base.clone(),
                Some(_) => format!("{base}__{}", short_ability_hash(&descriptor.name)),
            };
            if let Some(existing) = used.get(&tool_name) {
                if existing != &descriptor.name {
                    let hash = short_ability_hash(&descriptor.name);
                    let mut suffix = 2usize;
                    while used
                        .get(&tool_name)
                        .is_some_and(|existing| existing != &descriptor.name)
                    {
                        tool_name = format!("{base}__{hash}_{suffix}");
                        suffix += 1;
                    }
                }
            }
            used.insert(tool_name.clone(), descriptor.name.clone());
            routes.push(ToolRoute {
                tool_name,
                ability_name: descriptor.name.clone(),
                index,
            });
        }

        let mut reverse = std::collections::BTreeMap::new();
        for r in &routes {
            reverse.insert(r.tool_name.clone(), r.ability_name.clone());
        }

        Self { routes, reverse }
    }

    /// Resolve `tool_name` (an MCP-facing name) back to the canonical
    /// dotted EasyNet ability name.
    pub fn canonical_for_tool<'a>(&'a self, tool_name: &str) -> Option<&'a str> {
        self.reverse.get(tool_name).map(String::as_str)
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
    descriptors: &'a [crate::runtime::ability_descriptor::AbilityDescriptor],
    tool_name: &str,
) -> Option<&'a str> {
    let table = McpToolRouteTable::from_descriptors(descriptors);
    // We cannot return `&'a str` from the table's owned strings, so
    // resolve through the descriptor slice once we know the canonical
    // ability name. The clone is cheap (a single lookup); long-lived
    // call sites should switch to `McpToolRouteTable::canonical_for_tool`
    // directly to avoid even this.
    let canonical = table.canonical_for_tool(tool_name)?.to_string();
    descriptors
        .iter()
        .find(|d| d.name == canonical)
        .map(|d| d.name.as_str())
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

fn metadata_for_agent_ability(
    agent_name: &str,
    manifest: Option<&crate::core::ability_spec::AbilityManifest>,
) -> Vec<(&'static str, String)> {
    let mut metadata = vec![
        ("owner_agent", agent_name.to_string()),
        ("owner_user", "local".to_string()),
    ];
    // Exec kind + exec-specific metadata first. Cost is layered on
    // top: a manifest-declared `[cost]` table wins over the per-exec
    // heuristic so an operator who declares "this MCP-backed tool
    // hits Google Maps and bills $5/1000" sees that label propagate
    // verbatim into the MCP description and the `x-easynet.cost_*`
    // fields. When the manifest is silent we fall back to the
    // per-exec default — `unknown` for anything we cannot prove is
    // local (the honesty rule that replaced the older "free for
    // everything we don't recognise" lie).
    let (heur_kind, heur_label): (&str, &str) = match manifest.and_then(|m| m.exec()) {
        Some(crate::core::ability_spec::AbilityExec::Mcp(exec)) => {
            metadata.push(("exec_kind", "mcp".to_string()));
            metadata.push(("mcp_server", exec.server.clone()));
            metadata.push(("mcp_tool", exec.tool.clone()));
            ("unknown", "upstream cost declared by operator")
        }
        Some(crate::core::ability_spec::AbilityExec::Http(_)) => {
            metadata.push(("exec_kind", "http".to_string()));
            ("external_metered", "HTTP/API billing may apply")
        }
        Some(crate::core::ability_spec::AbilityExec::Shell(_)) => {
            metadata.push(("exec_kind", "shell".to_string()));
            ("free", "free/local")
        }
        Some(crate::core::ability_spec::AbilityExec::Eal(_)) => {
            metadata.push(("exec_kind", "eal".to_string()));
            ("unknown", "composed ability cost depends on steps")
        }
        Some(crate::core::ability_spec::AbilityExec::HostStream(_)) => {
            metadata.push(("exec_kind", "host_stream".to_string()));
            ("free", "free/local")
        }
        None => {
            metadata.push(("exec_kind", "agent_chat".to_string()));
            ("llm_metered", "LLM token billing may apply")
        }
    };
    let (cost_kind, cost_label) = match manifest.and_then(|m| m.cost()) {
        Some(declared) => {
            let kind = declared.kind.as_wire_str().to_string();
            let label = declared
                .label
                .clone()
                .unwrap_or_else(|| heur_label.to_string());
            (kind, label)
        }
        None => (heur_kind.to_string(), heur_label.to_string()),
    };
    metadata.push(("cost_kind", cost_kind));
    metadata.push(("cost_label", cost_label));
    metadata
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
    /// `local_daemon_grpc::invoke_local_daemon_ability_with_invocation_meta`)
    /// onto the driver-facing trace object.
    fn from_daemon_meta(meta: &serde_json::Value, mcp_tool: &str) -> Self {
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
        ability: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String>;

    /// Invoke and additionally surface the invocation identity trace.
    /// The default implementation carries no trace; the production daemon
    /// adapter overrides it so tool results can echo the ledger identity.
    /// `mcp_tool` is the wire tool name the driver matches against.
    fn invoke_traced(
        &self,
        ability: &str,
        _mcp_tool: &str,
        args: serde_json::Value,
    ) -> Result<(serde_json::Value, Option<InvocationToolTrace>), String> {
        self.invoke_sync(ability, args).map(|value| (value, None))
    }
}

/// Production adapter for `easynet mcp serve`: route every tool call
/// through the live local daemon's Axon Invocation gRPC surface
/// instead of through an isolated in-process kernel snapshot.
pub struct DaemonLocalInvoker;

impl LocalInvoker for DaemonLocalInvoker {
    fn invoke_sync(
        &self,
        ability: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        crate::support::local_invoke::invoke_local_ability(ability, args)
            .map_err(|err| err.to_string())
    }

    fn invoke_traced(
        &self,
        ability: &str,
        mcp_tool: &str,
        args: serde_json::Value,
    ) -> Result<(serde_json::Value, Option<InvocationToolTrace>), String> {
        let (value, meta) =
            crate::support::local_invoke::invoke_local_ability_with_invocation_meta(
                ability,
                args,
                None,
                &[],
                None,
                None,
                None,
            )
            .map_err(|err| err.to_string())?;
        let trace = InvocationToolTrace::from_daemon_meta(&meta, mcp_tool);
        Ok((value, Some(trace)))
    }
}

/// Production InvokeMcpProvider — what `easynet mcp_server` and
/// `easynet start --mcp` use after the P4.8d facade retirement. Every
/// `tools/list` returns the host's AbilityDescriptors projected to
/// MCP shape; every `tools/call` routes through
/// `LocalInvoker::invoke_sync`, which production wires to daemon.sock
/// Axon Invoke. Zero direct bridge calls; zero hub-mediated MCP tool
/// catalog.
pub struct InvokeMcpProvider<I: LocalInvoker> {
    invoker: I,
    /// Snapshot of the host's ability descriptors at construction.
    /// Refreshed on daemon restart; for now we keep a static list
    /// because the registry doesn't change at runtime.
    descriptors: Vec<crate::runtime::ability_descriptor::AbilityDescriptor>,
    /// Tool-name routing built from `descriptors` at construction.
    /// Kept paired with `descriptors` via the constructor — this is
    /// the only place that builds the table for the provider, so
    /// the two fields cannot drift.
    routes: McpToolRouteTable,
}

impl<I: LocalInvoker> InvokeMcpProvider<I> {
    pub fn new(
        invoker: I,
        descriptors: Vec<crate::runtime::ability_descriptor::AbilityDescriptor>,
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

/// One-stop builder: derive the host's AbilityDescriptors from
/// local-agents.json and produce a configured InvokeMcpProvider ready
/// for the stdio runner.
///
/// Both `easynet mcp_server` and `easynet start --mcp` call this
/// — they differ only in argument parsing and how they launch the
/// stdio server (foreground vs. spawned thread).
pub fn build_stdio_server(config: &StdioServerConfig) -> ConfiguredStdioServer {
    let mut descriptors = crate::daemon::ability::catalog::profiles::load_host_descriptors();

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

    let invoker = DaemonLocalInvoker;
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

    let to_descriptor = |s: crate::runtime::agent_ability_specs::AgentAbilitySpec,
                         owner_ura: &str,
                         source: String,
                         metadata: Vec<(&'static str, String)>|
     -> Option<AbilityDescriptor> {
        AbilityDescriptor::new(s.name().to_string(), owner_ura, Visibility::Scoped)
            .ok()
            .map(|mut d| {
                // AgentAbilitySpec calls its JSON-Schema field
                // `parameters()` (carrying the input schema in
                // the chat-style "parameters" shape) — that
                // IS the input schema for the descriptor.
                d = d
                    .with_input_schema(s.parameters().clone())
                    .with_source(source)
                    .with_description(s.description());
                for (key, value) in metadata {
                    d = d.with_metadata_entry(key, value);
                }
                d
            })
    };

    // Phase 1: this agent's own abilities. Owner URA uses the
    // agent's own name. The agent's `<agent_name>.chat` ability is
    // filtered out — it is the outgoing surface, not something to
    // expose AS a tool to the LLM running INSIDE it (that would
    // invite infinite recursion).
    let own_manifests: std::collections::BTreeMap<
        String,
        crate::core::ability_spec::AbilityManifest,
    > = crate::runtime::agent_ability_specs::manifests_for(agent_name, entry)
        .into_iter()
        .map(|manifest| (manifest.qualified_name(agent_name), manifest))
        .collect();
    let own_specs = crate::runtime::agent_ability_specs::abilities_for(agent_name, entry);
    let own_owner_ura = format!("agent://{agent_name}");
    let self_chat = format!("{agent_name}.chat");
    for s in own_specs.into_iter().filter(|s| s.name() != self_chat) {
        let metadata = metadata_for_agent_ability(agent_name, own_manifests.get(s.name()));
        if let Some(d) = to_descriptor(s, &own_owner_ura, format!("agent:{agent_name}"), metadata) {
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
    // projection advertises its own discover / invoke (the discovery
    // ladder is projection-scoped). Source = `kernel:built-in:self-bundle`
    // so an operator inspecting the descriptor catalogue can tell at a
    // glance the entry came from a synth path, not a TOML.
    {
        let discover_name = format!(
            "{agent_name}.{}",
            crate::daemon::ability::builtins::agents::discover::ABILITY_VERB
        );
        let invoke_name = format!(
            "{agent_name}.{}",
            crate::daemon::ability::builtins::agents::invoke::ABILITY_VERB
        );
        for (name, schema, description) in [
            (
                discover_name,
                crate::daemon::ability::builtins::agents::discover::input_schema(),
                crate::daemon::ability::builtins::agents::discover::description(),
            ),
            (
                invoke_name,
                crate::daemon::ability::builtins::agents::invoke::input_schema(),
                crate::daemon::ability::builtins::agents::invoke::description(),
            ),
        ] {
            if let Ok(d) = AbilityDescriptor::new(name, &own_owner_ura, Visibility::Scoped) {
                let mut d = d
                    .with_input_schema(schema)
                    .with_source("kernel:built-in:self-bundle")
                    .with_description(description);
                for (key, value) in [
                    ("owner_agent", agent_name.to_string()),
                    ("owner_user", "local".to_string()),
                    ("exec_kind", "builtin".to_string()),
                    ("cost_kind", "free".to_string()),
                    ("cost_label", "free/local".to_string()),
                ] {
                    d = d.with_metadata_entry(key, value);
                }
                out.push(d);
            }
        }
    }

    // Phase 2: every OTHER registered agent's abilities. This is
    // the cross-agent surface — when agent A is the active LLM and
    // the user asks for something only agent B has the skill for,
    // agent A's tool list now includes `<B>.<verb>` so the LLM can
    // route to it. Calling those tools dispatches through B's
    // materialized per-agent handler with B's own skills exposed.
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
        let other_manifests: std::collections::BTreeMap<
            String,
            crate::core::ability_spec::AbilityManifest,
        > = crate::runtime::agent_ability_specs::manifests_for(other_name, other_entry)
            .into_iter()
            .map(|manifest| (manifest.qualified_name(other_name), manifest))
            .collect();
        for s in crate::runtime::agent_ability_specs::abilities_for(other_name, other_entry)
            .into_iter()
            .filter(|s| s.name() != other_chat)
        {
            let metadata = metadata_for_agent_ability(other_name, other_manifests.get(s.name()));
            if let Some(d) = to_descriptor(s, &other_owner, format!("agent:{other_name}"), metadata)
            {
                out.push(d);
            }
        }
    }

    out
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

impl<I: LocalInvoker> easynet_axon::mcp::McpToolProvider for InvokeMcpProvider<I> {
    fn tool_specs(&self) -> Vec<serde_json::Value> {
        tool_specs_from_descriptors(&self.descriptors)
    }

    fn handle_tool_call(
        &self,
        name: &str,
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> easynet_axon::mcp::ToolResult {
        let ability_name = self.routes.canonical_for_tool(name);

        // Reject calls for tools we don't advertise. The descriptor
        // list is the single source of truth — if a name isn't
        // there, this is a caller-side bug, not a transient.
        let Some(ability_name) = ability_name else {
            return easynet_axon::mcp::ToolResult {
                payload: serde_json::json!({
                    "error": format!("unknown tool: `{name}`")
                }),
                is_error: true,
            };
        };
        let args_value = serde_json::Value::Object(args.clone());
        match self.invoker.invoke_traced(ability_name, name, args_value) {
            Ok((value, trace)) => easynet_axon::mcp::ToolResult {
                payload: fold_invocation_trace(value, trace),
                is_error: false,
            },
            Err(msg) => easynet_axon::mcp::ToolResult {
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
        sink: &mut dyn easynet_axon::mcp::ProgressSink,
    ) -> easynet_axon::mcp::ToolResult {
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
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
    use easynet_axon::mcp::McpToolProvider;
    use std::cell::RefCell;

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
        AbilityDescriptor::new(name, "easynet:///r/acme/device/01DEV", Visibility::Scoped)
            .unwrap()
            .with_source("kernel:built-in")
            .with_input_schema(serde_json::json!({"type":"object"}))
            .with_description("List every registered agent on this host.")
    }

    fn create_manifest_backed_agent_entry(
        agent: &str,
    ) -> (std::path::PathBuf, crate::registry::agents::AgentEntry) {
        use crate::core::agent_spec::{AgentSpec, RuntimeKind};
        use crate::registry::agents::{AgentEntry, AgentType};
        use crate::runtime::directory::{AgentDirectory, Location};

        let workspace_root = crate::persistence::config::agents_root().join(agent);
        let _ = std::fs::remove_dir_all(&workspace_root);
        AgentDirectory::create(
            &Location::Local {
                root: workspace_root.clone(),
            },
            AgentSpec::new(agent.to_string(), RuntimeKind::ClaudeCode),
        )
        .expect("create manifest-backed test agent directory");

        let mut entry = AgentEntry::new(AgentType::ClaudeCode, None);
        entry.root_path = Some(workspace_root.clone());
        (workspace_root, entry)
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
        let desc = AbilityDescriptor::new("a.b", "u", Visibility::Public)
            .unwrap()
            .with_input_schema(serde_json::json!({"type":"object"}));
        let spec = tool_spec_from_descriptor(&desc);
        assert!(spec["description"].as_str().unwrap().ends_with("] a.b"));
        assert_eq!(spec["name"], "a_b");
    }

    #[test]
    fn tool_spec_falls_back_to_object_schema_when_input_is_null() {
        let mut desc = AbilityDescriptor::new("a.b", "u", Visibility::Public).unwrap();
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
            "agent://openai",
            Visibility::Scoped,
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
        let descs = vec![d("observe.health"), d("agent.list")];
        let p = InvokeMcpProvider::new(RecordingInvoker::new(Ok(serde_json::json!({}))), descs);
        let specs = p.tool_specs();
        assert_eq!(specs.len(), 2);
        let names: Vec<&str> = specs.iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"observe_health"));
        assert!(names.contains(&"agent_list"));
    }

    #[test]
    fn mcp_provider_advertises_and_routes_agent_discover() {
        let desc = AbilityDescriptor::new("claude.discover", "agent://claude", Visibility::Scoped)
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
        assert_eq!(specs[0]["x-easynet"]["ability"], "claude.discover");
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
    fn descriptor_count_matches_input() {
        let invoker = RecordingInvoker::new(Ok(serde_json::json!({})));
        let p = InvokeMcpProvider::new(invoker, vec![d("observe.health"), d("agent.list")]);
        assert_eq!(p.descriptor_count(), 2);
    }

    #[test]
    fn daemon_local_invoker_surfaces_daemon_not_running() {
        let _h = crate::cli::test_support::HomeGuard::new();
        let err = DaemonLocalInvoker
            .invoke_sync("observe.health", serde_json::json!({}))
            .expect_err("daemon-backed invoker must fail when no daemon is running");
        assert!(
            err.contains("daemon not running"),
            "expected actionable daemon-down error; got {err}"
        );
    }

    #[test]
    fn build_stdio_server_produces_provider_with_at_least_observe_health() {
        // Single-source-of-truth contract: both `easynet mcp_server`
        // and `easynet start --mcp` go through `build_stdio_server`.
        // The result MUST advertise every device-profile ability the
        // live registry registers, anchored on whatever local-agents.json
        // says (or the literal "self" pre-join).
        let _h = crate::cli::test_support::HomeGuard::new();
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
        // we get must reference this URA as owner.
        let owners: std::collections::HashSet<String> = configured
            .provider
            .descriptors
            .iter()
            .map(|d| d.owner_ura.clone())
            .collect();
        assert!(
            owners.contains("self"),
            "pre-join fallback must anchor descriptors on `self`; got owners = {owners:?}"
        );
    }

    #[test]
    fn build_stdio_server_anchors_descriptors_on_persisted_host_ura_when_present() {
        let _h = crate::cli::test_support::HomeGuard::new();
        // Pre-populate local-agents.json with a host URA; build_stdio_server
        // must pick it up.
        let file = crate::persistence::local_agents::LocalAgentsFile {
            host_device_agent_ura: "easynet:///r/acme/device/01DEV".into(),
            ..crate::persistence::local_agents::LocalAgentsFile::default()
        };
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
            .map(|d| d.owner_ura.clone())
            .collect();
        assert!(
            owners.contains("easynet:///r/acme/device/01DEV"),
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
        use crate::cli::test_support::HomeGuard;

        let _g = HomeGuard::new();

        // Set up an agent + a custom ability under its workspace.
        // Use a name unlikely to collide with the developer's
        // real ~/.easynet/workspaces/* contents. HomeGuard already
        // isolates HOME, but multiple in-process tests can still
        // race on the same per-test tempdir if they all pick
        // generic names like "alice" or "bob".
        let agent = "g1-test-agent";

        let mut registry = crate::registry::agents::AgentRegistry::default();
        let (workspace_root, entry) = create_manifest_backed_agent_entry(agent);
        registry.agents.insert(agent.into(), entry);
        crate::registry::agents::save_agents(&registry).unwrap();

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
        std::fs::write(
            workspace_root.join("abilities/mcp-google-maps__geocode.ability.toml"),
            "schema_version = \"1\"\n\
             name = \"mcp-google-maps__geocode\"\n\
             description = \"Geocode an address using Google Maps.\"\n\
             [input_schema]\n\
             type = \"object\"\n\
             [exec]\n\
             kind = \"mcp\"\n\
             server = \"Google Maps\"\n\
             tool = \"geocode\"\n",
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
        let mcp_desc = configured
            .provider
            .descriptors
            .iter()
            .find(|d| d.name == format!("{agent}.mcp-google-maps__geocode"))
            .expect("mcp-backed ability descriptor");
        assert_eq!(
            mcp_desc.metadata.get("owner_agent").map(String::as_str),
            Some(agent)
        );
        assert_eq!(
            mcp_desc.metadata.get("exec_kind").map(String::as_str),
            Some("mcp")
        );
        // MCP-backed abilities default to `cost_kind = "unknown"` —
        // the manifest's [exec] kind="mcp" block does not declare
        // cost, and substring-sniffing the upstream server/tool name
        // (e.g. "google-maps") for the word "google" / "map" used to
        // mis-tag internal upstreams as `external_metered`. The
        // honest contract: catalog metadata must be declared, not
        // inferred. The wire prefix surfaces the same string so an
        // LLM sampling only the description sees the disclosure.
        assert_eq!(
            mcp_desc.metadata.get("cost_kind").map(String::as_str),
            Some("unknown")
        );
        let mcp_tool = tool_spec_from_descriptor(mcp_desc);
        assert!(mcp_tool["description"]
            .as_str()
            .unwrap()
            .contains("cost: unknown"));
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
    fn manifest_declared_cost_overrides_exec_kind_heuristic() {
        // A `[cost]` table in the per-workspace manifest must win
        // over `metadata_for_agent_ability`'s exec-kind fallback.
        // This is the whole point of Day-1 work — give operators a
        // way to say "this MCP-backed tool actually hits a billed
        // upstream" without the system burying that under the
        // fallback `cost: unknown`.
        use crate::cli::test_support::HomeGuard;

        let _g = HomeGuard::new();
        let agent = "day1-cost-test-agent";

        let mut registry = crate::registry::agents::AgentRegistry::default();
        let (workspace_root, entry) = create_manifest_backed_agent_entry(agent);
        registry.agents.insert(agent.into(), entry);
        crate::registry::agents::save_agents(&registry).unwrap();

        // MCP-backed ability whose [exec] kind = "mcp" would normally
        // resolve to `cost_kind = unknown` via the heuristic. The
        // [cost] table here overrides that to the declared bucket +
        // label, which is what an LLM choosing between two geocoders
        // should see.
        std::fs::write(
            workspace_root.join("abilities/geocode.ability.toml"),
            "schema_version = \"1\"\n\
             name = \"geocode\"\n\
             description = \"Geocode an address.\"\n\
             [input_schema]\n\
             type = \"object\"\n\
             [exec]\n\
             kind = \"mcp\"\n\
             server = \"Google Maps\"\n\
             tool = \"geocode\"\n\
             [cost]\n\
             kind = \"external_metered\"\n\
             label = \"Google Maps Geocoding — $5 per 1000 requests\"\n",
        )
        .unwrap();

        let cfg = StdioServerConfig {
            server_name: "easynet-test".into(),
            tenant_id: "t".into(),
            agent_name: Some(agent.to_string()),
        };
        let configured = build_stdio_server(&cfg);
        let desc = configured
            .provider
            .descriptors
            .iter()
            .find(|d| d.name == format!("{agent}.geocode"))
            .expect("geocode descriptor must be registered");

        // Declared bucket wins over the exec=mcp fallback (`unknown`).
        assert_eq!(
            desc.metadata.get("cost_kind").map(String::as_str),
            Some("external_metered"),
            "manifest [cost] must override the exec_kind heuristic"
        );
        // Declared label flows verbatim — what reaches the operator's
        // eye in MCP descriptions and the `x-easynet.cost_label`
        // field.
        assert_eq!(
            desc.metadata.get("cost_label").map(String::as_str),
            Some("Google Maps Geocoding — $5 per 1000 requests")
        );

        // And the MCP description prefix includes the declared label.
        let spec = tool_spec_from_descriptor(desc);
        let description = spec["description"].as_str().unwrap();
        assert!(
            description.contains("cost: external_metered"),
            "description must surface declared cost bucket; got: {description}"
        );
        assert!(
            description.contains("Google Maps Geocoding — $5 per 1000 requests"),
            "description must surface declared cost label; got: {description}"
        );
    }

    #[test]
    fn invoke_provider_routes_observe_health_through_local_invoker() {
        use crate::runtime::ability_descriptor::Visibility;
        use easynet_axon::mcp::McpToolProvider;
        let invoker = FakeInvoker {
            value: serde_json::json!({"echo": {}}),
        };

        let descs = vec![AbilityDescriptor::new(
            "observe.health",
            "easynet:///r/acme/device/01DEV",
            Visibility::Public,
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
            _ability: &str,
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
    impl easynet_axon::mcp::ProgressSink for CountingSink {
        fn report(
            &mut self,
            progress: f64,
            total: Option<f64>,
            message: Option<&str>,
        ) -> easynet_axon::AxonResult<easynet_axon::mcp::ReportOutcome> {
            self.reports
                .lock()
                .unwrap()
                .push((progress, total, message.map(|s| s.to_string())));
            Ok(easynet_axon::mcp::ReportOutcome::Emitted)
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

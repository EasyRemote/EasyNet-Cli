// EasyNet CLI — Reflective MCP-tool → ability registry
// =====================================================
//
// File: src/runtime/agents/mcp_reflective_registry.rs
//
// Per the F-01 hard-discipline list (Frame doc + plan
// wondrous-jumping-rocket.md):
//
//   - Each upstream MCP tool becomes ONE ability with its own URA.
//   - The URA shape MUST be the canonical
//       easynet:///r/<realm>/ability/<owner>.<tail>
//     where `<owner>` is the mcp-profile agent URA. The URA literal
//     MUST NOT carry any implementation-source tag (e.g. no
//     `mcp_upstream` substring). Provenance lives on
//     `AbilityDescriptor.source`, not in the address.
//   - Naming collisions are resolved at config-load time via the
//     `name_prefix` / `aliases` fields on `McpServerSpec` — the
//     registry refuses to overwrite an existing handler.
//   - Axon protocol wire is untouched: reflection registers ordinary
//     local abilities; cross-device dispatch reuses
//     `federation.forward_invoke` like any other ability.
//
// What this module does NOT do (kept narrow on purpose):
//
//   - Does not own the lifecycle of `McpClientService` connections.
//     The caller passes a service that's already been built from
//     `mcp_clients.json`; we just call `tools/list` + register.
//   - Does not yet handle `notifications/tools/list_changed`
//     (round-2 of the plan).
//   - Does not yet handle reflective registration over HTTP
//     transport — falls out for free once `McpClientService` learns
//     to dispatch HTTP (task #3 in the plan).
//   - Does not yet implement `unregister(name)` for graceful tool
//     removal — the registry's no-overwrite policy means callers
//     have to drop the whole registry to refresh. Round-2.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::core::ability_spec::AbilityManifest;
use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
use crate::runtime::ability_dispatch::{LocalAbilityRegistry, OwnerKind};
use crate::runtime::execution::mcp_client::McpClientService;

/// Stable prefix stamped into `AbilityDescriptor.source` for every
/// reflectively-registered upstream MCP tool, before the
/// `<server_name>:<upstream_tool>` discriminator.
///
/// Pinned as a `pub const` so the producer ([`format_mcp_upstream_source`])
/// and every consumer ([`is_mcp_upstream_source`], cost-classification in
/// `profiles::mcp`, tests asserting on the source field) share one
/// spelling. A rename here is a compile-time event across the crate
/// instead of a silent string drift between call sites.
pub const MCP_UPSTREAM_SOURCE_PREFIX: &str = "mcp_upstream:";

/// Build the canonical `AbilityDescriptor.source` value for a tool
/// reflected from upstream MCP server `server_name` whose
/// server-reported tool name is `upstream_tool`.
///
/// Shape: `mcp_upstream:<server_name>:<upstream_tool>`. See
/// [`MCP_UPSTREAM_SOURCE_PREFIX`] for the prefix contract; the rest
/// is the (server, tool) pair the reflective registry recorded at
/// registration time.
#[must_use]
pub fn format_mcp_upstream_source(server_name: &str, upstream_tool: &str) -> String {
    format!("{MCP_UPSTREAM_SOURCE_PREFIX}{server_name}:{upstream_tool}")
}

/// True iff `source` was produced by [`format_mcp_upstream_source`]
/// — i.e. the descriptor was minted by the reflective registry.
/// Use this from consumer paths (cost classification, audit log
/// filters) rather than open-coding `starts_with` with a raw string.
#[must_use]
pub fn is_mcp_upstream_source(source: &str) -> bool {
    source.starts_with(MCP_UPSTREAM_SOURCE_PREFIX)
}

/// One successfully reflected tool. Returned to the caller so it can
/// log, surface in UI, or feed downstream descriptor advertisement.
#[derive(Debug, Clone)]
pub struct ReflectedAbility {
    /// Local ability name as registered (after `apply_local_name`).
    pub ability_name: String,
    /// The descriptor written for downstream consumers
    /// (`meta.list_abilities`, `federation.advertise_abilities`,
    /// the inbound MCP bridge's projection).
    pub descriptor: AbilityDescriptor,
    /// Upstream server's local name (the operator-chosen short
    /// identifier from `mcp_clients.json`).
    pub server: String,
    /// Upstream tool name as the server reported it. Distinguishing
    /// this from `ability_name` matters when an alias was applied.
    pub upstream_tool: String,
}

/// One failed reflection — kept separate from successes so the
/// caller can decide whether to fail boot or just log + carry on
/// (matching the "graceful upstream failure" pattern already used
/// by `device.mcp.client.list`).
#[derive(Debug, Clone)]
pub struct ReflectFailure {
    pub server: String,
    /// `None` when the failure happened during `tools/list` (i.e.
    /// the upstream itself is broken); `Some(tool)` when a single
    /// tool failed to register (e.g. name collision).
    pub tool: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct ReflectResult {
    pub registered: Vec<ReflectedAbility>,
    pub failed: Vec<ReflectFailure>,
}

/// Reflect every tool of every configured upstream server into
/// `registry`, anchored to `owner_agent_ura` (the mcp-profile
/// agent URA the daemon constructs at boot).
///
/// The function is async because `McpClientService::rpc` is async.
/// Callers running off the tokio runtime can wrap in
/// `Handle::block_on` if needed; the canonical call site is the
/// daemon boot path, which is async by construction.
///
/// Concurrency: this serialises across servers (one `tools/list`
/// at a time) because the daemon boot path is sequential anyway
/// and parallelism only matters for ≥10 servers. The 28-server
/// mcp-bench setup completes serially in well under the operator's
/// patience budget on a warm host. Round-2 can fan out per-server
/// if profiling shows it worth it.
pub async fn reflect_all(
    client: &McpClientService,
    registry: &mut LocalAbilityRegistry,
    owner_agent_ura: &str,
) -> ReflectResult {
    let mut out = ReflectResult::default();

    for server_name in client.server_names().await {
        // tools/list is the first MCP RPC after the handshake; a
        // broken upstream surfaces here, not at first invocation.
        let listing = match tokio::time::timeout(
            mcp_tools_list_timeout(),
            client.rpc(&server_name, "tools/list", json!({})),
        )
        .await
        {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                out.failed.push(ReflectFailure {
                    server: server_name.clone(),
                    tool: None,
                    reason: format!("tools/list failed: {e}"),
                });
                continue;
            }
            Err(_) => {
                out.failed.push(ReflectFailure {
                    server: server_name.clone(),
                    tool: None,
                    reason: format!(
                        "tools/list timed out after {}s",
                        mcp_tools_list_timeout().as_secs()
                    ),
                });
                continue;
            }
        };

        // Fetch the spec once per server so we can apply the
        // operator's name_prefix / aliases without re-locking the
        // client for every tool.
        let spec = match client.spec(&server_name).await {
            Some(s) => s,
            None => {
                // Should be unreachable — server_names() came from
                // the same map — but defensively skip rather than
                // panic.
                out.failed.push(ReflectFailure {
                    server: server_name.clone(),
                    tool: None,
                    reason: "server vanished between server_names() and spec()".into(),
                });
                continue;
            }
        };

        let tools = match listing.get("tools").and_then(Value::as_array) {
            Some(arr) => arr.clone(),
            None => {
                out.failed.push(ReflectFailure {
                    server: server_name.clone(),
                    tool: None,
                    reason: format!(
                        "tools/list response missing `tools` array (got {})",
                        listing
                    ),
                });
                continue;
            }
        };

        let mut writer = StaticWriter { reg: registry };
        for tool in tools {
            match register_one_tool(
                &mut writer,
                &client.clone(),
                &server_name,
                owner_agent_ura,
                &spec,
                &tool,
            ) {
                Ok(rec) => out.registered.push(rec),
                Err(fail) => out.failed.push(fail),
            }
        }
    }

    out
}

/// Result of a `refresh_server` call. Mirrors `ReflectResult` but
/// separates "tools that were added", "tools that were removed",
/// and "tools that already existed and still exist" so the caller
/// (a notifications/tools/list_changed handler) can log the diff
/// coherently.
#[derive(Debug, Clone, Default)]
pub struct RefreshDiff {
    /// Newly-registered abilities (present in upstream now, absent
    /// from registry before refresh).
    pub added: Vec<ReflectedAbility>,
    /// Local ability names that the upstream no longer advertises
    /// — these were unregistered from the registry.
    pub removed: Vec<String>,
    /// Tools that were already registered and remain registered.
    /// We don't currently re-register them (input schema /
    /// description changes are NOT detected in v1; round-3 work).
    pub unchanged: Vec<String>,
    /// Per-tool failures during refresh (e.g. name collision).
    pub failed: Vec<ReflectFailure>,
}

/// Refresh the reflection state for ONE upstream server.
///
/// Use case (plan §B4): the `McpClientService::NotificationSink`
/// observes `notifications/tools/list_changed`, the
/// `mcp_reflective_registry`'s notification handler calls this to
/// reconcile the registry with the upstream's new catalogue.
///
/// What this does:
///   1. Re-run `tools/list` against the named upstream.
///   2. Compute the set of currently-reflected local ability names
///      whose `AbilityDescriptor.source` carries the prefix
///      [`MCP_UPSTREAM_SOURCE_PREFIX`] followed by
///      `<server_name>:` — that's "owned by this upstream in the
///      previous refresh".
///   3. For each tool the upstream now advertises:
///      - if its local name is already registered AND was owned by
///        this server → mark unchanged
///      - else → register fresh
///   4. For each previously-owned local name NOT in the new tools
///      list → call `registry.unregister(name)`.
///
/// What this does NOT do (round-3):
///   * Re-register on schema/description change. v1 keeps the old
///     descriptor unless the tool name itself disappeared.
///   * Hot-reload the daemon's published_abilities snapshot. The
///     reflective registry mutates `registry` only; the
///     federation.advertise_abilities surface refreshes on its
///     own cadence.
///
/// **Implementation note.** The body lives in
/// [`refresh_server_inner`], parametrised over the registry surface
/// (`StaticWriter` for boot/explicit refresh, `DynamicWriter` for
/// runtime hot-reload via `RegistryRefreshSink`) so both flows share
/// exactly one implementation. The trailing `flavour` arg is folded
/// into the error messages (`refresh` vs `dynamic refresh`) —
/// operators reading stderr need to know which path emitted the line.
async fn refresh_server_inner<W: RegistryWriter>(
    writer: &mut W,
    client: &McpClientService,
    owner_agent_ura: &str,
    server_name: &str,
    previously_reflected: &[String],
    flavour: &'static str,
) -> RefreshDiff {
    let mut diff = RefreshDiff::default();

    let listing = match client.rpc(server_name, "tools/list", json!({})).await {
        Ok(v) => v,
        Err(e) => {
            diff.failed.push(ReflectFailure {
                server: server_name.to_string(),
                tool: None,
                reason: format!("tools/list failed during {flavour}: {e}"),
            });
            return diff;
        }
    };
    let spec = match client.spec(server_name).await {
        Some(s) => s,
        None => {
            diff.failed.push(ReflectFailure {
                server: server_name.to_string(),
                tool: None,
                reason: format!("server vanished during {flavour}"),
            });
            return diff;
        }
    };
    let tools = match listing.get("tools").and_then(Value::as_array) {
        Some(arr) => arr.clone(),
        None => {
            diff.failed.push(ReflectFailure {
                server: server_name.to_string(),
                tool: None,
                reason: format!(
                    "tools/list response missing `tools` array on {flavour} (got {listing})"
                ),
            });
            return diff;
        }
    };

    // Compute the new local name set so we can retire vanished names.
    let mut new_local_names = std::collections::HashSet::new();
    for tool in &tools {
        let upstream_name = tool.get("name").and_then(Value::as_str).unwrap_or("");
        if upstream_name.is_empty() {
            continue;
        }
        new_local_names.insert(spec.apply_local_name(upstream_name));
    }

    // Register-or-mark-unchanged each new tool.
    for tool in &tools {
        let local_name = match tool.get("name").and_then(Value::as_str) {
            Some(n) if !n.is_empty() => spec.apply_local_name(n),
            _ => continue,
        };
        // Skip names already known to the registry (static OR
        // dynamic, per writer.has). Static-side hits are silent
        // shadows; dynamic-side hits mean we've already reflected
        // this tool — no diff.
        if writer.has(&local_name) {
            diff.unchanged.push(local_name);
            continue;
        }
        match register_one_tool(writer, client, server_name, owner_agent_ura, &spec, tool) {
            Ok(rec) => diff.added.push(rec),
            Err(fail) => diff.failed.push(fail),
        }
    }

    // Retire previously-reflected names that the upstream no longer
    // advertises. The trait dispatch lands in the right surface:
    // `StaticWriter::unregister` clears the boot maps;
    // `DynamicWriter::unregister` clears only the hot-reload side
    // table (see the
    // `hot_unregister_removes_dynamic_entry_without_touching_static`
    // pin in `ability_dispatch`).
    for prev in previously_reflected {
        if !new_local_names.contains(prev) && writer.unregister(prev) {
            diff.removed.push(prev.clone());
        }
    }

    diff
}

/// Dynamic-side refresh facade. Reacts to a runtime
/// `notifications/tools/list_changed` push after the registry has
/// been frozen behind `Arc<LocalAbilityRegistry>` at daemon boot.
///
/// The hot path's lookup order is static → dynamic → fallback, so
/// a dynamic-side rewrite is invisible to any boot-registered
/// ability: a hot-listed tool whose name happens to collide with a
/// system ability is silently shadowed by the static entry. See
/// `static_lookup_wins_over_dynamic_on_name_collision` for the pin.
pub async fn refresh_server_dynamic(
    client: &McpClientService,
    registry: &LocalAbilityRegistry,
    owner_agent_ura: &str,
    server_name: &str,
    previously_reflected: &[String],
) -> RefreshDiff {
    let mut writer = DynamicWriter { reg: registry };
    refresh_server_inner(
        &mut writer,
        client,
        owner_agent_ura,
        server_name,
        previously_reflected,
        "dynamic refresh",
    )
    .await
}

/// Static-side refresh facade. The boot path's explicit
/// reconcile-one-server entry point; used by callers that hold
/// `&mut LocalAbilityRegistry` (the boot reflective sweep, tests).
pub async fn refresh_server(
    client: &McpClientService,
    registry: &mut LocalAbilityRegistry,
    owner_agent_ura: &str,
    server_name: &str,
    previously_reflected: &[String],
) -> RefreshDiff {
    let mut writer = StaticWriter { reg: registry };
    refresh_server_inner(
        &mut writer,
        client,
        owner_agent_ura,
        server_name,
        previously_reflected,
        "refresh",
    )
    .await
}

/// Env var that overrides the per-server `tools/list` timeout used
/// during reflective registration at daemon boot. Discoverable here
/// so operators can grep for the knob; the canonical list of EasyNet
/// env vars also documents this name. Must parse as a positive
/// integer (seconds); anything else falls back to the default below.
pub const ENV_MCP_TOOLS_LIST_TIMEOUT_SECS: &str = "EASYNET_MCP_TOOLS_LIST_TIMEOUT_SECS";

/// Default `tools/list` timeout when the env override is absent or
/// malformed. 20s is long enough for a cold-spawn stdio upstream to
/// complete its initialize+tools/list round-trip on a warm host, yet
/// short enough that a broken upstream does not stall the rest of
/// daemon boot.
const DEFAULT_MCP_TOOLS_LIST_TIMEOUT_SECS: u64 = 20;

fn mcp_tools_list_timeout() -> Duration {
    let secs = std::env::var(ENV_MCP_TOOLS_LIST_TIMEOUT_SECS)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MCP_TOOLS_LIST_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// Abstract over the two registry-side surfaces (static maps owned
/// by `&mut LocalAbilityRegistry`, hot-reload side table owned via
/// `&LocalAbilityRegistry` interior mutability) so the two reflection
/// flows — boot-time `reflect_all`/`refresh_server` and runtime
/// `RegistryRefreshSink`-driven `refresh_server_dynamic` — share
/// one body.
///
/// Why a trait rather than two parallel functions
/// ----------------------------------------------
/// Before this trait existed, [`register_one_tool`] /
/// `register_one_tool_dynamic` and [`refresh_server`] /
/// `refresh_server_dynamic` were near-verbatim duplicates differing
/// only in (a) the registry reference type and (b) which two registry
/// methods they called. Every per-tool enhancement (cost metadata,
/// progress-frame shape, descriptor tagging) had to land on both
/// copies; one missed mirror silently broke runtime hot-reload while
/// boot kept working. This trait makes the bifurcation explicit and
/// the lockstep automatic.
///
/// Trait implementors must call into the registry through whichever
/// surface they own — see [`StaticWriter`] and [`DynamicWriter`].
trait RegistryWriter {
    /// True if any of the three handler maps (rpc / stream / bidi)
    /// hold this name. The reflective registry refuses to overwrite
    /// — operators have to set `name_prefix` / `aliases` instead.
    fn has(&self, name: &str) -> bool;

    /// Register a stream-shaped handler under `name`. The
    /// implementor decides whether this lands in the static maps
    /// (boot path) or the hot-reload side table (runtime path).
    fn register_stream(
        &mut self,
        name: String,
        owner: OwnerKind,
        manifest: AbilityManifest,
        handler: crate::runtime::ability_dispatch::LocalStreamHandler,
    );

    /// Remove every trace of `name`. Returns `true` when something
    /// was actually removed. Static writers touch the boot maps;
    /// dynamic writers touch only the hot-reload side table.
    fn unregister(&mut self, name: &str) -> bool;

    /// Discriminator shown to operators in the collision error
    /// message so they can tell whether the prior registration came
    /// from boot (static) or from a hot-reload (dynamic). `None`
    /// means "no qualifier needed" (the static writer is the only
    /// possible prior); `Some(s)` is rendered as ` (s)` by
    /// [`format_collision_hint`]. Kept on the trait so neither
    /// writer leaks its kind through ad-hoc parameters.
    const COLLISION_KIND_HINT: Option<&'static str>;
}

/// Render [`RegistryWriter::COLLISION_KIND_HINT`] into the
/// parenthesised qualifier slot of the collision error message,
/// or the empty string when the hint is `None`. Centralised so the
/// "leading space + parens" boilerplate is not duplicated at every
/// call site and a future hint variant (e.g. a third writer) cannot
/// drift out of shape.
fn format_collision_hint(hint: Option<&'static str>) -> String {
    match hint {
        Some(s) => format!(" ({s})"),
        None => String::new(),
    }
}

/// Static side: `&mut LocalAbilityRegistry`. Used by boot-time
/// `reflect_all` and the original `refresh_server` facade.
struct StaticWriter<'a> {
    reg: &'a mut LocalAbilityRegistry,
}

impl RegistryWriter for StaticWriter<'_> {
    fn has(&self, name: &str) -> bool {
        self.reg.has_rpc(name) || self.reg.has_stream(name) || self.reg.has_bidi(name)
    }

    fn register_stream(
        &mut self,
        name: String,
        owner: OwnerKind,
        manifest: AbilityManifest,
        handler: crate::runtime::ability_dispatch::LocalStreamHandler,
    ) {
        self.reg
            .register_stream_with_spec(name, owner, manifest, handler);
    }

    fn unregister(&mut self, name: &str) -> bool {
        self.reg.unregister(name)
    }

    const COLLISION_KIND_HINT: Option<&'static str> = None;
}

/// Dynamic side: `&LocalAbilityRegistry` (interior mutability via
/// `hot_*` methods). Used by `RegistryRefreshSink` to react to
/// upstream `notifications/tools/list_changed` after the registry
/// has been frozen behind `Arc<LocalAbilityRegistry>` at daemon
/// boot.
///
/// The implementor takes `&self` on the underlying registry —
/// `&mut self` on the writer is purely to let the trait stay
/// shared-shape. The `LocalAbilityRegistry` itself is borrowed
/// shared, so this writer can live alongside other readers of the
/// registry without violating borrow rules.
struct DynamicWriter<'a> {
    reg: &'a LocalAbilityRegistry,
}

impl RegistryWriter for DynamicWriter<'_> {
    fn has(&self, name: &str) -> bool {
        self.reg.has_rpc(name) || self.reg.has_stream(name) || self.reg.has_bidi(name)
    }

    fn register_stream(
        &mut self,
        name: String,
        owner: OwnerKind,
        manifest: AbilityManifest,
        handler: crate::runtime::ability_dispatch::LocalStreamHandler,
    ) {
        self.reg
            .hot_register_stream_with_spec(name, owner, manifest, handler);
    }

    fn unregister(&mut self, name: &str) -> bool {
        self.reg.hot_unregister(name)
    }

    const COLLISION_KIND_HINT: Option<&'static str> = Some("static or dynamic");
}

/// Register exactly one upstream tool as a local ability through
/// the abstract writer. Returns the reflected record on success, or
/// a `ReflectFailure` when the upstream tool descriptor is malformed
/// OR the operator's config maps it to a name already taken in the
/// registry.
///
/// This function holds the per-tool handler shape and the descriptor
/// projection contract; any enhancement (cost metadata, progress
/// frame variant, descriptor tagging) lands here exactly once and
/// reaches both registry sides through the trait.
fn register_one_tool<W: RegistryWriter>(
    writer: &mut W,
    client: &McpClientService,
    server_name: &str,
    owner_agent_ura: &str,
    spec: &crate::runtime::execution::mcp_client::McpServerSpec,
    tool: &Value,
) -> Result<ReflectedAbility, ReflectFailure> {
    let upstream_tool = tool
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| ReflectFailure {
            server: server_name.to_string(),
            tool: None,
            reason: format!("tool entry missing `name` field: {tool}"),
        })?
        .to_string();
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let input_schema = tool
        .get("inputSchema")
        .cloned()
        .unwrap_or_else(|| json!({"type": "object"}));
    let local_name = spec.apply_local_name(&upstream_tool);
    let manifest_verb = local_name
        .rsplit('.')
        .next()
        .unwrap_or(&local_name)
        .to_string();

    if writer.has(&local_name) {
        return Err(ReflectFailure {
            server: server_name.to_string(),
            tool: Some(upstream_tool.clone()),
            reason: format!(
                "ability `{local_name}` already registered{kind}; \
                 set `name_prefix` or an entry in `aliases` for \
                 server `{server_name}` in mcp_clients.json",
                kind = format_collision_hint(W::COLLISION_KIND_HINT),
            ),
        });
    }

    let desc_text = if description.is_empty() {
        upstream_tool.clone()
    } else {
        description.clone()
    };
    let manifest = AbilityManifest::new(manifest_verb, desc_text.clone(), input_schema.clone())
        .map_err(|e| ReflectFailure {
            server: server_name.to_string(),
            tool: Some(upstream_tool.clone()),
            reason: format!("manifest build failed: {e}"),
        })?;

    let provenance = format_mcp_upstream_source(server_name, &upstream_tool);
    let client_for_handler = client.clone();
    let server_for_handler = server_name.to_string();
    let upstream_for_handler = upstream_tool.clone();
    let local_name_for_handler = local_name.clone();
    let handler: crate::runtime::ability_dispatch::LocalStreamHandler =
        Arc::new(move |args: Value| -> anyhow::Result<crate::runtime::ability_dispatch::StreamSource> {
            // Allocate the broadcast channel BEFORE spawning so the
            // receiver is in hand the moment we return — caller's
            // first `recv()` cannot race the producer.
            //
            // Bound 64: enough to absorb a burst of progress frames
            // from a chatty upstream while the caller drains them.
            // Lagged receivers surface as a typed stream error
            // rather than a silent drop.
            let (tx, rx) = tokio::sync::broadcast::channel::<Value>(64);

            // Auto-allocated progress token. The MCP spec requires
            // the token be unique across active requests; a UUID is
            // overkill, so we use server+tool+monotonic-ns.
            let token = serde_json::json!(format!(
                "{}:{}:{}",
                server_for_handler,
                upstream_for_handler,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));

            // Reflective stream handlers are only ever invoked from
            // the daemon's tonic-driven `InvokeStream` path or from
            // an integration test that already runs inside
            // `#[tokio::test]`. Both provide an ambient runtime, so
            // missing `Handle::current()` indicates caller bug —
            // fail fast rather than ship a fragile detached-thread
            // fallback whose lifecycle depends on undocumented
            // runtime behaviour.
            tokio::runtime::Handle::try_current().map_err(|_| {
                anyhow::anyhow!(
                    "mcp reflective stream handler `{ability}` invoked outside a tokio runtime; \
                     callers must drive this through the daemon's async InvokeStream path \
                     or a `#[tokio::test]`-managed runtime",
                    ability = local_name_for_handler,
                )
            })?;

            tokio::spawn(stream_one_upstream_call(
                client_for_handler.clone(),
                server_for_handler.clone(),
                upstream_for_handler.clone(),
                args,
                token,
                tx,
            ));

            Ok(crate::runtime::ability_dispatch::StreamSource::Live(rx))
        });

    writer.register_stream(local_name.clone(), OwnerKind::Device, manifest, handler);

    // Build the descriptor that downstream `meta.list_abilities`
    // and `federation.advertise_abilities` will surface. CRITICAL:
    // the URA the caller sees later is derived from `owner_agent_ura`
    // + `local_name` (no `mcp_upstream` substring). Provenance goes
    // ONLY into `source`.
    let descriptor =
        AbilityDescriptor::new(local_name.clone(), owner_agent_ura, Visibility::Scoped)
            .map_err(|e| ReflectFailure {
                server: server_name.to_string(),
                tool: Some(upstream_tool.clone()),
                reason: format!("descriptor build failed: {e}"),
            })?
            .with_input_schema(input_schema)
            .with_description(desc_text)
            .with_source(provenance)
            .with_metadata_entry("mcp_server", server_name.to_string())
            .with_metadata_entry("mcp_tool", upstream_tool.clone());

    Ok(ReflectedAbility {
        ability_name: local_name,
        descriptor,
        server: server_name.to_string(),
        upstream_tool,
    })
}

/// `NotificationSink` that forwards every upstream
/// `notifications/progress` frame to the broadcast channel feeding
/// the caller's `InvokeStream`. Other notification kinds
/// (`tools/list_changed`, server-side log frames) are dropped — the
/// caller asked for `tools/call`, not a directory watcher; mixing
/// them into the same stream would change the contract of the frame
/// shape.
struct StreamForwardingSink {
    sender: tokio::sync::broadcast::Sender<Value>,
}

impl crate::runtime::execution::mcp_client::NotificationSink for StreamForwardingSink {
    fn observe(&mut self, n: crate::runtime::execution::mcp_client::ObservedNotification) {
        if let Some(p) = n.as_progress() {
            let frame = serde_json::json!({
                "type": "progress",
                "token": p.token,
                "progress": p.progress,
                "total": p.total,
                "message": p.message,
            });
            // Err means the caller dropped the stream mid-call.
            // That is not our concern — the upstream call still
            // completes, and the terminal frame fails to send for
            // the same reason without leaking the task.
            let _ = self.sender.send(frame);
        }
    }
}

/// Long-lived `NotificationSink` that reacts to
/// `notifications/tools/list_changed` by re-running `tools/list` on
/// the originating MCP server and rewriting the dynamic side table.
/// Distinct from `StreamForwardingSink`: that one is per-call
/// (passed through `rpc_with_progress`), this one is registered once
/// per upstream at daemon boot and observes notifications at any
/// time, including while the daemon is idle.
///
/// Holds a `Weak<LocalAbilityRegistry>` so the sink does not extend
/// the daemon's registry lifetime — when the registry is dropped at
/// shutdown the sink becomes a no-op rather than blocking shutdown.
///
/// Holds a `Weak<McpClientService>` for the same reason; the sink
/// re-runs `tools/list` through this handle. If the service has
/// been torn down (orderly daemon shutdown), the sink silently
/// drops the notification.
///
/// `reflected_names` tracks every local-name the sink previously
/// dynamic-registered for this server, so the diff-driven refresh
/// can retire vanished names without leaving stale entries.
pub struct RegistryRefreshSink {
    registry: std::sync::Weak<LocalAbilityRegistry>,
    client: std::sync::Weak<crate::runtime::execution::mcp_client::McpClientService>,
    server_name: String,
    owner_agent_ura: String,
    /// Names previously reflected through this sink. Wrapped in Arc
    /// so we can hand a clone to the spawned refresh task without
    /// extending the sink's lifetime (the sink itself lives in the
    /// listener task's `notification_sinks` map; the refresh task
    /// only touches this one Mutex).
    reflected_names: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl RegistryRefreshSink {
    pub fn new(
        registry: std::sync::Weak<LocalAbilityRegistry>,
        client: std::sync::Weak<crate::runtime::execution::mcp_client::McpClientService>,
        server_name: String,
        owner_agent_ura: String,
        initially_reflected: Vec<String>,
    ) -> Self {
        Self {
            registry,
            client,
            server_name,
            owner_agent_ura,
            reflected_names: std::sync::Arc::new(std::sync::Mutex::new(initially_reflected)),
        }
    }
}

impl crate::runtime::execution::mcp_client::NotificationSink for RegistryRefreshSink {
    fn observe(&mut self, n: crate::runtime::execution::mcp_client::ObservedNotification) {
        if n.method != "notifications/tools/list_changed" {
            return;
        }
        let Some(registry) = self.registry.upgrade() else {
            // Daemon shutdown in progress — drop quietly. The sink
            // will be torn down when the listener task it lives on
            // exits.
            return;
        };
        let Some(client) = self.client.upgrade() else {
            return;
        };
        // Snapshot prev BEFORE spawning so the async task sees a
        // consistent view; the post-refresh writeback below merges
        // diff.removed / diff.added into the live Vec.
        let prev = self
            .reflected_names
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let server = self.server_name.clone();
        let owner = self.owner_agent_ura.clone();
        let names = std::sync::Arc::clone(&self.reflected_names);
        // We're called from the mcp_client listener task, which is a
        // tokio task — `Handle::current()` is available. observe()
        // must return quickly (the listener is still draining
        // frames), so the network-bound refresh runs detached.
        tokio::spawn(async move {
            let diff =
                refresh_server_dynamic(&client, &registry, &owner, &server, &prev).await;
            if let Ok(mut g) = names.lock() {
                g.retain(|n| !diff.removed.contains(n));
                for added in &diff.added {
                    if !g.contains(&added.ability_name) {
                        g.push(added.ability_name.clone());
                    }
                }
            }
        });
    }
}

/// Drive one upstream `tools/call` and translate the result into
/// the EasyNet stream frame contract:
///   * progress frames: `{ type: "progress", token, progress, total, message }`
///   * terminal success: `{ type: "response", result: <verbatim MCP result> }`
///   * terminal error:   `{ type: "error", message }`
///
/// The future ends with `tx` dropping, which closes the broadcast
/// channel and signals end-of-stream to the receiver.
async fn stream_one_upstream_call(
    client: crate::runtime::execution::mcp_client::McpClientService,
    server: String,
    upstream_tool: String,
    args: Value,
    progress_token: Value,
    tx: tokio::sync::broadcast::Sender<Value>,
) {
    let mut sink = StreamForwardingSink {
        sender: tx.clone(),
    };
    let params = serde_json::json!({
        "name": upstream_tool,
        "arguments": args,
        // Attach the token so the upstream knows it SHOULD emit
        // progress (per MCP spec §"Progress" #1). Upstreams MAY
        // ignore it; that's fine — no progress frames just means
        // the caller sees only the terminal frame.
        "_meta": { "progressToken": progress_token },
    });
    let terminal = match client
        .rpc_with_progress(&server, "tools/call", params, &mut sink)
        .await
    {
        Ok(value) => serde_json::json!({
            "type": "response",
            "result": value,
        }),
        Err(e) => serde_json::json!({
            "type": "error",
            "message": e.to_string(),
        }),
    };
    let _ = tx.send(terminal);
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::execution::mcp_client::{McpClientService, McpClientsFile, McpServerSpec};
    use std::collections::HashMap;

    /// Build an in-process MCP client wrapping a small Python echo
    /// server. The server answers `tools/list` with two tools and
    /// `tools/call` by echoing the passed arguments — enough to
    /// exercise the full register-then-invoke loop.
    #[cfg(unix)]
    fn make_echo_client(server_name: &str) -> (tempfile::TempDir, Arc<McpClientService>) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("echo_mcp.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
exec python3 -u -c '
import sys, json
def read_msg():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.decode().strip()
        if not line:
            break
        name, value = line.split(":", 1)
        headers[name.lower()] = value.strip()
    body = sys.stdin.buffer.read(int(headers["content-length"]))
    return json.loads(body)
def write_msg(resp):
    body = json.dumps(resp).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()
while True:
    req = read_msg()
    if req is None:
        break
    rid = req.get("id")
    method = req.get("method")
    if method == "initialize":
        result = {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "echo", "version": "0"}}
    elif method == "tools/list":
        result = {"tools": [
            {"name": "echo_one", "description": "echoes one", "inputSchema": {"type": "object"}},
            {"name": "echo_two", "description": "echoes two", "inputSchema": {"type": "object", "properties": {"x": {"type": "string"}}}}
        ]}
    elif method == "tools/call":
        params = req.get("params") or {}
        result = {"content": [{"type": "text", "text": json.dumps(params.get("arguments", {}))}], "isError": False}
    else:
        result = {}
    write_msg({"jsonrpc": "2.0", "id": rid, "result": result})
'
"#,
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let svc = McpClientService::from_file(McpClientsFile {
            servers: vec![McpServerSpec {
                name: server_name.into(),
                command: script.to_string_lossy().to_string(),
                stdio_framing: "content-length".into(),
                ..Default::default()
            }],
        });
        (dir, Arc::new(svc))
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reflects_two_tools_with_clean_descriptors() {
        let (_dir, svc) = make_echo_client("echo");
        let mut reg = LocalAbilityRegistry::new();
        // mcp-profile agent URA — same shape as the daemon would
        // construct: easynet:///r/<realm>/agent/<user>.mcp
        let owner = "easynet:///r/test-realm/agent/test-user.mcp";

        let result = reflect_all(&svc, &mut reg, owner).await;

        assert!(
            result.failed.is_empty(),
            "no failures expected, got {:?}",
            result.failed
        );
        assert_eq!(result.registered.len(), 2);

        // Names registered verbatim (no prefix, no alias).
        let names: Vec<&str> = result
            .registered
            .iter()
            .map(|r| r.ability_name.as_str())
            .collect();
        assert!(names.contains(&"echo_one"));
        assert!(names.contains(&"echo_two"));

        // Provenance ends up on source, NEVER in the URA-shaped
        // owner field. The exact prefix is pinned by
        // `MCP_UPSTREAM_SOURCE_PREFIX`; assert through the helper so
        // a future rename of the constant lands in one place.
        for rec in &result.registered {
            assert!(
                is_mcp_upstream_source(&rec.descriptor.source),
                "source must carry the mcp_upstream prefix, got {:?}",
                rec.descriptor.source
            );
            assert!(
                rec.descriptor.source.starts_with(&format!(
                    "{MCP_UPSTREAM_SOURCE_PREFIX}echo:"
                )),
                "source must include the server discriminator, got {:?}",
                rec.descriptor.source
            );
            assert_eq!(rec.descriptor.owner_agent_ura, owner);
            // The discipline check that gate 2 enforces at script
            // level — assert it in code too so a refactor that
            // accidentally embeds the label in the owner trips
            // here before the gate.
            assert!(
                !rec.descriptor.owner_agent_ura.contains("mcp_upstream"),
                "owner URA must NOT contain implementation label"
            );
            assert!(!rec.ability_name.contains("mcp_upstream"));
        }

        // Registry actually has the handlers.
        // Reflective registration produces STREAM abilities (B2b).
        // Callers can still use Axon's unary Invoke RPC — runtime
        // flattens the stream's terminal frame into a unary
        // response — but the registry-level key lives in the
        // stream map.
        assert!(reg.has_stream("echo_one"));
        assert!(reg.has_stream("echo_two"));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn applies_name_prefix_from_spec() {
        let (_dir, _svc) = make_echo_client("ignored");
        // Re-build the service with a prefix on the spec.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("echo_mcp.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
exec python3 -u -c '
import sys, json
def read_msg():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.decode().strip()
        if not line:
            break
        name, value = line.split(":", 1)
        headers[name.lower()] = value.strip()
    body = sys.stdin.buffer.read(int(headers["content-length"]))
    return json.loads(body)
def write_msg(resp):
    body = json.dumps(resp).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()
while True:
    req = read_msg()
    if req is None:
        break
    method = req.get("method")
    if method == "initialize":
        result = {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "ctx", "version": "0"}}
    elif method == "tools/list":
        result = {"tools": [{"name": "search_docs", "inputSchema": {"type": "object"}}]}
    else:
        result = {}
    write_msg({"jsonrpc": "2.0", "id": req.get("id"), "result": result})
'
"#,
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let svc = McpClientService::from_file(McpClientsFile {
            servers: vec![McpServerSpec {
                name: "context7".into(),
                command: script.to_string_lossy().to_string(),
                name_prefix: "ctx7.".into(),
                stdio_framing: "content-length".into(),
                ..Default::default()
            }],
        });
        let mut reg = LocalAbilityRegistry::new();
        let result = reflect_all(&svc, &mut reg, "easynet:///r/r/agent/u.mcp").await;

        assert!(
            result.failed.is_empty(),
            "expected zero failures, got: {:?}",
            result.failed
        );
        assert_eq!(result.registered.len(), 1);
        assert_eq!(result.registered[0].ability_name, "ctx7.search_docs");
        assert_eq!(result.registered[0].upstream_tool, "search_docs");
        assert!(reg.has_stream("ctx7.search_docs"));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn name_collision_fails_explicitly() {
        // Pre-register a handler under the name the upstream tool
        // would claim, then verify reflection refuses to overwrite.
        let (_dir, svc) = make_echo_client("echo");
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc("echo_one", Arc::new(|_: Value| Ok(json!("local"))));

        let result = reflect_all(&svc, &mut reg, "easynet:///r/r/agent/u.mcp").await;

        // echo_two registers; echo_one fails with a clear message
        // pointing the operator at the config knob.
        let registered_names: Vec<&str> = result
            .registered
            .iter()
            .map(|r| r.ability_name.as_str())
            .collect();
        assert_eq!(registered_names, vec!["echo_two"]);
        assert_eq!(result.failed.len(), 1);
        let f = &result.failed[0];
        assert_eq!(f.tool.as_deref(), Some("echo_one"));
        assert!(
            f.reason.contains("name_prefix") && f.reason.contains("aliases"),
            "failure reason must steer the operator at the fix: {}",
            f.reason
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn missing_upstream_is_recorded_not_panic() {
        // No server configured — server_names() is empty — result
        // is empty without panic. This mirrors `from_path`'s
        // "missing file is OK" stance and keeps daemon boot robust
        // against the operator running before they've configured
        // any upstreams.
        let svc = McpClientService::new();
        let mut reg = LocalAbilityRegistry::new();
        let result = reflect_all(&svc, &mut reg, "easynet:///r/r/agent/u.mcp").await;
        assert!(result.registered.is_empty());
        assert!(result.failed.is_empty());
    }

    /// HashMap import only used by one test; tag with allow to keep
    /// the no-unused-import lint clean in builds where the cfg gates
    /// take it out of scope.
    #[allow(dead_code)]
    fn _hm_marker(_: HashMap<String, String>) {}

    /// B4 — `refresh_server` reconciles registry state with a
    /// changed upstream tools catalogue. Diff classification:
    /// added / removed / unchanged.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refresh_server_diffs_added_and_removed_tools() {
        // Use two echo servers built from the same script template
        // but advertising DIFFERENT tool sets. First we register
        // against the "old" catalogue, then refresh against the
        // "new" one and assert the diff.
        fn write_script(dir: &std::path::Path, tools: &[&str], name: &str) -> std::path::PathBuf {
            let tools_json: String = tools
                .iter()
                .map(|t| format!(r#"{{"name":"{t}","inputSchema":{{"type":"object"}}}}"#))
                .collect::<Vec<_>>()
                .join(",");
            let script = dir.join(format!("{name}.sh"));
            std::fs::write(
                &script,
                format!(
                    r#"#!/bin/sh
exec python3 -u -c '
import sys, json
def read_msg():
    headers = {{}}
    while True:
        raw = sys.stdin.buffer.readline()
        if not raw:
            return None
        line = raw.decode().strip()
        if not line:
            break
        n, v = line.split(":", 1)
        headers[n.lower()] = v.strip()
    body = sys.stdin.buffer.read(int(headers["content-length"]))
    return json.loads(body)
def write_msg(m):
    b = json.dumps(m).encode()
    sys.stdout.buffer.write(f"Content-Length: {{len(b)}}\r\n\r\n".encode() + b)
    sys.stdout.buffer.flush()
while True:
    req = read_msg()
    if req is None:
        break
    rid = req.get("id")
    if rid is None:
        continue
    method = req.get("method")
    if method == "tools/list":
        result = {{"tools": [{tools_json}]}}
    elif method == "tools/call":
        result = {{"content": [{{"type": "text", "text": "ok"}}], "isError": False}}
    else:
        result = {{}}
    write_msg({{"jsonrpc": "2.0", "id": rid, "result": result}})
'
"#
                ),
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            script
        }

        let dir = tempfile::tempdir().unwrap();
        // The "before" upstream advertises [a, b].
        let script = write_script(dir.path(), &["a", "b"], "before");

        let svc =
            McpClientService::from_file(crate::runtime::execution::mcp_client::McpClientsFile {
                servers: vec![McpServerSpec {
                    name: "echo".into(),
                    command: script.to_string_lossy().to_string(),
                    stdio_framing: "content-length".into(),
                    ..Default::default()
                }],
            });
        let mut reg = LocalAbilityRegistry::new();
        let initial = reflect_all(&svc, &mut reg, "easynet:///r/test/agent/u.mcp").await;
        assert!(initial.failed.is_empty(), "{:?}", initial.failed);
        let prev_names: Vec<String> = initial
            .registered
            .iter()
            .map(|r| r.ability_name.clone())
            .collect();
        assert_eq!(prev_names, vec!["a".to_string(), "b".to_string()]);

        // Drop the old upstream connection (server still running
        // but we're done with it). For B4 we'd ideally have the
        // SAME upstream process change its tools list mid-flight —
        // simulating that in a test means swapping the underlying
        // process. Easiest reproduction: tear down the old service
        // and stand up a new one pointing at the SAME server name
        // but a new script that advertises [b, c].
        drop(reg);
        drop(svc);

        let script2 = write_script(dir.path(), &["b", "c"], "after");
        let svc2 =
            McpClientService::from_file(crate::runtime::execution::mcp_client::McpClientsFile {
                servers: vec![McpServerSpec {
                    name: "echo".into(),
                    command: script2.to_string_lossy().to_string(),
                    stdio_framing: "content-length".into(),
                    ..Default::default()
                }],
            });
        let mut reg2 = LocalAbilityRegistry::new();
        // Pre-seed reg2 with the "before" catalogue + a fake
        // descriptor source — refresh_server should keep `b`,
        // remove `a`, add `c`.
        let pre = reflect_all(&svc2, &mut reg2, "easynet:///r/test/agent/u.mcp").await;
        // svc2's script advertises [b, c]; but we want to start
        // from the [a, b] state. Manually register `a` so the
        // diff has something to remove.
        use std::sync::Arc;
        reg2.register_rpc(
            "a",
            Arc::new(|_: Value| Ok(serde_json::json!({"stale": "tool a"}))),
        );
        // After this seed, reg2 has [a, b, c]. previously_reflected
        // (from the operator's POV) is [a, b].
        let _ = pre;

        let diff = refresh_server(
            &svc2,
            &mut reg2,
            "easynet:///r/test/agent/u.mcp",
            "echo",
            &["a".into(), "b".into()],
        )
        .await;

        // `b` was already registered → unchanged.
        assert!(diff.unchanged.iter().any(|n| n == "b"));
        // `a` not in new catalogue, was in previously_reflected →
        // removed.
        assert_eq!(diff.removed, vec!["a".to_string()]);
        // `c` already registered by the earlier `reflect_all`
        // (pre) → unchanged (NOT added — refresh only adds tools
        // missing from the live registry).
        assert!(diff.unchanged.iter().any(|n| n == "c"));
        // Registry state: `a` removed (was the stale unary seed),
        // `b` + `c` still present. Note: reflective registration
        // produces STREAM abilities (B2b — so upstream progress
        // notifications flow through Axon's InvokeStream), hence
        // `has_stream` rather than `has_rpc` for the reflected
        // names. `a` was directly seeded with register_rpc so the
        // negative check stays on the rpc side too.
        assert!(!reg2.has_rpc("a"));
        assert!(!reg2.has_stream("a"));
        assert!(reg2.has_stream("b"));
        assert!(reg2.has_stream("c"));
    }

    /// B2b — when the upstream MCP server emits
    /// `notifications/progress` mid-call, those frames MUST flow
    /// through the reflected ability's stream as `{type:
    /// "progress", ...}` chunks, with the terminal `{type:
    /// "response", ...}` chunk carrying the final tools/call
    /// payload. This is what lets a caller `InvokeStream` against
    /// the reflected ability and watch upstream progress in real
    /// time — the whole point of B2b.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reflected_ability_streams_upstream_progress_chunks() {
        // Python upstream that emits TWO progress notifications
        // before the matching tools/call response. Same LSP-style
        // framing as the rest of the suite.
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("progress_upstream.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
exec python3 -u -c '
import sys, json
def read_msg():
    headers = {}
    while True:
        raw = sys.stdin.buffer.readline()
        if not raw:
            return None
        line = raw.decode().strip()
        if not line:
            break
        n, v = line.split(":", 1)
        headers[n.lower()] = v.strip()
    body = sys.stdin.buffer.read(int(headers["content-length"]))
    return json.loads(body)
def write_msg(m):
    b = json.dumps(m).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(b)}\r\n\r\n".encode() + b)
    sys.stdout.buffer.flush()
while True:
    req = read_msg()
    if req is None:
        break
    rid = req.get("id")
    if rid is None:
        continue
    method = req.get("method")
    if method == "initialize":
        write_msg({"jsonrpc":"2.0","id":rid,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"prg","version":"0"}}})
    elif method == "tools/list":
        write_msg({"jsonrpc":"2.0","id":rid,"result":{"tools":[{"name":"slow_op","inputSchema":{"type":"object"}}]}})
    elif method == "tools/call":
        token = (req.get("params") or {}).get("_meta", {}).get("progressToken")
        write_msg({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":token,"progress":0.25,"total":1.0,"message":"warming up"}})
        write_msg({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":token,"progress":0.75,"total":1.0,"message":"almost done"}})
        write_msg({"jsonrpc":"2.0","id":rid,"result":{"content":[{"type":"text","text":"finished"}],"isError":False}})
    else:
        write_msg({"jsonrpc":"2.0","id":rid,"result":{}})
'
"#,
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let svc = crate::runtime::execution::mcp_client::McpClientService::from_file(
            crate::runtime::execution::mcp_client::McpClientsFile {
                servers: vec![
                    crate::runtime::execution::mcp_client::McpServerSpec {
                        name: "prg".into(),
                        command: script.to_string_lossy().to_string(),
                        stdio_framing: "content-length".into(),
                        ..Default::default()
                    },
                ],
            },
        );
        let mut reg = LocalAbilityRegistry::new();
        let result = reflect_all(&svc, &mut reg, "easynet:///r/test/agent/u.mcp").await;
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert!(reg.has_stream("slow_op"));

        let handler = reg.get_stream("slow_op").expect("stream handler present");
        let source = handler(serde_json::json!({"input": "go"})).expect("handler ok");
        let mut rx = match source {
            crate::runtime::ability_dispatch::StreamSource::Live(rx) => rx,
            other => panic!("expected Live, got {other:?}"),
        };

        // Drain frames. Expect 2 progress + 1 response.
        let mut progress_frames = Vec::new();
        let mut terminal: Option<Value> = None;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(6);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
                Ok(Ok(frame)) => {
                    match frame.get("type").and_then(|v| v.as_str()) {
                        Some("progress") => progress_frames.push(frame),
                        Some("response") => {
                            terminal = Some(frame);
                            break;
                        }
                        Some("error") => panic!("got error frame: {frame}"),
                        other => panic!("unknown frame type {other:?}: {frame}"),
                    }
                }
                Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }

        assert_eq!(
            progress_frames.len(),
            2,
            "expected 2 upstream progress frames, got {}",
            progress_frames.len()
        );
        assert_eq!(progress_frames[0]["progress"], 0.25);
        assert_eq!(progress_frames[0]["message"], "warming up");
        assert_eq!(progress_frames[1]["progress"], 0.75);
        assert_eq!(progress_frames[1]["message"], "almost done");

        let term = terminal.expect("terminal response frame must arrive");
        assert_eq!(term["result"]["isError"], false);
        assert_eq!(term["result"]["content"][0]["text"], "finished");
    }
}

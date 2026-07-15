#![cfg(all(feature = "axon-pb", unix))]

//! End-to-end test for MCP hot-reload.
//!
//! Spins up a real stdio MCP child process that:
//!   1. First `tools/list` returns `[{name: "tool_a", ...}]`.
//!   2. After the first call sees `tools/list`, the next stdin read
//!      causes the child to PUSH a bare `notifications/tools/list_changed`
//!      frame on its own initiative (no `id` field — server-initiated
//!      notification per MCP spec).
//!   3. Second `tools/list` (driven by `RegistryRefreshSink` in
//!      reaction to the notification) returns `[{name: "tool_b", ...}]`.
//!
//! After step 3 the registry's dynamic side table must show `tool_b`
//! and not `tool_a` — that is the user-visible payoff of the whole
//! listener + dynamic_ext stack landing.

use std::sync::Arc;

use easynet_axon::invocation::LocalRuntime;
use easynet_cli::daemon::ability::builtins::integrations::mcp::reflective_registry::{
    refresh_server_dynamic, RegistryRefreshSink,
};
use easynet_cli::daemon::ability::dispatch::{
    AbilityAuthorityContext, AxonAbilityCatalog, OwnerKind,
};
use easynet_cli::daemon::execution::mcp::{McpClientService, McpClientsFile, McpServerSpec};

const MCP_OWNER_URA: &str = "easynet:///r/local/agent/local.mcp";

fn registry_for_mcp_owner() -> AxonAbilityCatalog {
    let owner = easynet_cli::core::ura::parse_ura(MCP_OWNER_URA).expect("canonical MCP owner URA");
    let device_ura = easynet_cli::core::ura::device_ura(&owner.realm, "mcp-hot-reload-device");
    let authority_context =
        AbilityAuthorityContext::for_combined_authority_roots_with_hosted_agents(
            device_ura,
            [MCP_OWNER_URA.to_string()],
        )
        .expect("MCP owner must be hosted by the test Device authority");
    AxonAbilityCatalog::new_with_runtime_and_authority_context(
        LocalRuntime::new(),
        authority_context,
    )
}

/// Build a Python stdio MCP server whose tools/list answer toggles
/// between two single-tool catalogues and pushes a list_changed
/// notification each time the client calls `tools/list`. The first
/// reply has `tool_a`, every subsequent reply (after the push) has
/// `tool_b` — exactly the shape `refresh_server_dynamic` needs to
/// detect a removed + added pair in its diff.
fn write_toggling_mcp_server(dir: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("toggling_mcp.sh");
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
        name, value = line.split(":", 1)
        headers[name.lower()] = value.strip()
    body = sys.stdin.buffer.read(int(headers["content-length"]))
    return json.loads(body)

def write_msg(resp):
    body = json.dumps(resp).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()

served_tools_list = 0
while True:
    req = read_msg()
    if req is None:
        break
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        write_msg({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": "2024-11-05"}})
    elif method == "notifications/initialized":
        # Notification — no response. (Server side never receives id
        # here in this flow.)
        pass
    elif method == "tools/list":
        served_tools_list += 1
        if served_tools_list == 1:
            tools = [{"name": "tool_a", "description": "Initial tool", "inputSchema": {"type": "object"}}]
            write_msg({"jsonrpc": "2.0", "id": rid, "result": {"tools": tools}})
            # Right after the first tools/list reply, push a
            # list_changed notification so the client'\''s
            # RegistryRefreshSink reacts and asks for the new list.
            write_msg({"jsonrpc": "2.0", "method": "notifications/tools/list_changed", "params": {}})
        else:
            tools = [{"name": "tool_b", "description": "Replacement tool", "inputSchema": {"type": "object"}}]
            write_msg({"jsonrpc": "2.0", "id": rid, "result": {"tools": tools}})
    else:
        write_msg({"jsonrpc": "2.0", "id": rid, "result": {}})
'
"#,
    )
    .expect("write toggling mcp script");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("chmod script");
    script
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_changed_push_triggers_dynamic_refresh() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = write_toggling_mcp_server(dir.path());

    let file = McpClientsFile {
        servers: vec![McpServerSpec {
            name: "toggling".into(),
            command: script.to_string_lossy().to_string(),
            stdio_framing: "content-length".into(),
            ..Default::default()
        }],
    };
    let svc = Arc::new(McpClientService::from_file(file));
    let registry = Arc::new(registry_for_mcp_owner());

    // Seed the dynamic side with `tool_a` so the refresh logic sees a
    // diff (removed=[tool_a], added=[tool_b]). In production this
    // mirrors what happens after the initial lazy reflection populates
    // the dynamic table; here we short-circuit that first reflection
    // and write tool_a directly so the test isolates the
    // list_changed → refresh path.
    {
        let manifest = easynet_cli::daemon::ability::manifest::AbilityManifest::new(
            "tool_a",
            "Initial tool",
            serde_json::json!({"type": "object"}),
        )
        .unwrap();
        // Boot-reflected tools register as stream handlers (mirrors
        // `register_one_tool_dynamic`). For the diff to detect the
        // unchanged-vs-removed split we don't actually need the
        // handler to run — just the registry membership.
        registry
            .hot_register_stream_with_spec(
                "tool_a",
                OwnerKind::Agent("mcp".to_string()),
                manifest,
                Arc::new(|_args| anyhow::bail!("tool_a handler not expected to run in this test")),
            )
            .expect("preload tool_a stream ability");
    }
    assert!(registry.has_stream("tool_a"));
    assert!(!registry.has_stream("tool_b"));

    // Attach the refresh sink with `tool_a` as the initially-reflected
    // set. The sink registers itself on the connection's
    // notification_sinks list; when the upstream pushes
    // tools/list_changed (right after the first tools/list reply),
    // the sink spawns refresh_server_dynamic, which calls
    // tools/list again, sees `[tool_b]`, and rewrites the dynamic
    // side: tool_a is hot_unregistered, tool_b is hot_registered.
    let sink = Box::new(RegistryRefreshSink::new(
        Arc::downgrade(&registry),
        Arc::downgrade(&svc),
        "toggling".to_string(),
        MCP_OWNER_URA.to_string(),
        vec!["tool_a".to_string()],
    ));
    svc.register_notification_sink("toggling", sink)
        .await
        .expect("attach refresh sink");

    // Trigger by calling tools/list ourselves. The Python script
    // sends the list_changed push as the second frame on its stdout,
    // so the listener task observes it and the sink spawns the
    // refresh task. We just need to await the refresh's settle.
    let listing = svc
        .rpc("toggling", "tools/list", serde_json::json!({}))
        .await
        .expect("first tools/list");
    let tools = listing
        .get("tools")
        .and_then(|v| v.as_array())
        .expect("tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "tool_a");

    // The sink's refresh task is detached from this future; poll the
    // registry state until the diff lands or we time out. 5s is
    // generous — the refresh does one extra tools/list RPC over a
    // local stdio child.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut converged = false;
    while std::time::Instant::now() < deadline {
        if registry.has_stream("tool_b") && !registry.has_stream("tool_a") {
            converged = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        converged,
        "registry never converged after list_changed push: \
         has tool_a={}, has tool_b={}",
        registry.has_stream("tool_a"),
        registry.has_stream("tool_b"),
    );
}

/// Sanity check: `refresh_server_dynamic` called directly (no sink,
/// no notification path) also produces the correct diff. Splits the
/// failure modes so a regression in the notification path doesn't
/// mask a regression in the diff math, or vice versa.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_server_dynamic_direct_call_diffs_correctly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = write_toggling_mcp_server(dir.path());

    let file = McpClientsFile {
        servers: vec![McpServerSpec {
            name: "toggling".into(),
            command: script.to_string_lossy().to_string(),
            stdio_framing: "content-length".into(),
            ..Default::default()
        }],
    };
    let svc = McpClientService::from_file(file);
    let registry = registry_for_mcp_owner();

    // First refresh — registry empty, prev=[], upstream replies
    // tool_a (script's first tools/list response).
    let diff1 = refresh_server_dynamic(&svc, &registry, MCP_OWNER_URA, "toggling", &[]).await;
    assert_eq!(
        diff1.added.len(),
        1,
        "first refresh adds tool_a; failures: {:?}",
        diff1.failed
    );
    assert_eq!(diff1.added[0].ability_name, "tool_a");
    assert!(diff1.removed.is_empty());

    // Second refresh — prev=[tool_a], upstream replies tool_b
    // (script's second tools/list response since it counted the
    // first call already). tool_a should be removed, tool_b added.
    let diff2 = refresh_server_dynamic(
        &svc,
        &registry,
        MCP_OWNER_URA,
        "toggling",
        &["tool_a".to_string()],
    )
    .await;
    assert_eq!(diff2.added.len(), 1);
    assert_eq!(diff2.added[0].ability_name, "tool_b");
    assert_eq!(diff2.removed, vec!["tool_a".to_string()]);
}

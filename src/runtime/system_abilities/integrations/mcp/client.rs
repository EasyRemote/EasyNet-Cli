// EasyNet CLI — mcp.client.{list,call} ability handlers (C-M9b)
// ===============================================================
//
// File: src/runtime/system_abilities/integrations/mcp/client.rs
//
// Edge-adapter abilities for OUTBOUND MCP. The mcp.bridge.* pair
// (C-M9a / C-M9a-ii) is INBOUND — external MCP clients see EasyNet
// as a server. mcp.client.* is the symmetric direction: EasyNet as
// a client, talking to other MCP servers (the operator's existing
// MCP servers, e.g. context7, filesystem MCP, etc.) so their tools
// become callable through the same in-process Invoke pipeline
// every other ability uses.
//
// Together with mcp.bridge.*, this completes the "ability-as-MCP"
// surface: anything that speaks MCP looks the same to the rest of
// the daemon as anything that speaks the local Invoke ABI.
//
// What lives here
// ---------------
//   * mcp.client.list  — aggregate `tools/list` across every
//                        configured upstream MCP server. Returns
//                        `{ servers: [{name, tools: [...]}], ... }`.
//   * mcp.client.call  — forward a `tools/call` to a chosen
//                        upstream by name. Returns the upstream's
//                        response verbatim (MCP `tools/call`
//                        shape: `{content, isError}`).
//
// Layer (per AXON-RFC-001-ability-layers.md)
//   * mcp.client.list  → Introspection (pure read of the catalogue)
//   * mcp.client.call  → Operational  (dispatches into an external
//                                       process; side effects come
//                                       from the upstream tool)
//
// Why aggregate vs. one-server-per-call for list?
// ----------------------------------------------
// The MCP-as-discovery use case is "show me every tool I can
// reach from this daemon." Forcing the caller to first list
// servers, then iterate, would burn round-trips. Aggregation in
// one ability matches what an operator running `mcp ls` would
// expect from a single command. A `server` filter argument on
// the input schema is the seam for "I only want one server's
// tools" if it ever matters.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::catalog::profiles::DEFAULT_MCP_AGENT_ID;
use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::execution::mcp_client::McpClientService;

pub const ABILITY_LIST: &str = crate::daemon::ability::names::integrations::MCP_CLIENT_LIST;
pub const ABILITY_CALL: &str = crate::daemon::ability::names::integrations::MCP_CLIENT_CALL;

pub fn register(reg: &mut AxonAbilityCatalog, svc: Arc<McpClientService>) {
    let svc_for_list = Arc::clone(&svc);
    reg.register_rpc_with_owner(
        ABILITY_LIST,
        OwnerKind::Agent(DEFAULT_MCP_AGENT_ID.to_string()),
        Arc::new(move |args: Value| {
            let svc = Arc::clone(&svc_for_list);
            // The ability-dispatch path is sync but McpClientService
            // is async (subprocess I/O). Spin up a small runtime
            // handle and block — abilities aren't on a hot path
            // here, and the tokio runtime the daemon already runs
            // gives us a current-thread handle to defer onto.
            block_on_async(async move { list_handler(&svc, args).await })
        }),
    );
    reg.register_rpc_with_owner(
        ABILITY_CALL,
        OwnerKind::Agent(DEFAULT_MCP_AGENT_ID.to_string()),
        Arc::new(move |args: Value| {
            let svc = Arc::clone(&svc);
            block_on_async(async move { call_handler(&svc, args).await })
        }),
    );
}

/// Bridge from sync ability dispatch to async service code. The
/// dispatcher's `LocalRpcHandler` signature is sync `Fn(Value) ->
/// Result<Value>` (matching the call modes pre-bidi) so we run
/// the async body on whatever runtime the caller already provides.
///
/// `tokio::runtime::Handle::current` will succeed when the daemon
/// dispatches through the IPC server (which runs on tokio).
/// `block_on` from inside a runtime is normally a deadlock risk;
/// `Handle::block_on` from a *non-runtime* thread is fine.
/// `Handle::current().block_on` from inside a runtime panics, so
/// we guard with `try_current` and fall back to a fresh runtime
/// for the test path that calls these handlers directly.
fn block_on_async<F: std::future::Future<Output = anyhow::Result<Value>>>(
    fut: F,
) -> anyhow::Result<Value> {
    match tokio::runtime::Handle::try_current() {
        Ok(_handle) => {
            // Inside a runtime — block_in_place + block_on is the
            // canonical bridge for "I'm in async land but need to
            // call this sync function that needs to call back into
            // async." Required because the LocalRpcHandler signature
            // is sync.
            tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(fut))
        }
        Err(_) => {
            // Test path: no runtime running. Build a single-threaded
            // one, block on it, drop it.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| anyhow::anyhow!("build test runtime: {e}"))?;
            rt.block_on(fut)
        }
    }
}

/// `mcp.client.list` handler. Aggregates `tools/list` across every
/// configured upstream MCP server. A single failed upstream does
/// NOT fail the whole call — its entry surfaces with `error` set
/// and `tools: []`, so the operator sees which one is broken
/// without losing the others.
async fn list_handler(svc: &Arc<McpClientService>, _args: Value) -> anyhow::Result<Value> {
    let names = svc.server_names().await;
    let mut servers = Vec::with_capacity(names.len());
    for name in names {
        let entry = match svc.rpc(&name, "tools/list", json!({})).await {
            Ok(result) => {
                let tools = result
                    .get("tools")
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new()));
                json!({
                    "name": name,
                    "tools": tools,
                })
            }
            Err(e) => json!({
                "name": name,
                "tools": [],
                "error": e.to_string(),
            }),
        };
        servers.push(entry);
    }
    Ok(json!({ "servers": servers }))
}

/// `mcp.client.call` handler. Forwards a single `tools/call` to a
/// named upstream and returns the response verbatim.
async fn call_handler(svc: &Arc<McpClientService>, args: Value) -> anyhow::Result<Value> {
    let server = match args.get("server").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return Ok(error_response(
                "`server` is required and must be a non-empty string",
            ));
        }
    };
    let tool_name = match args.get("name").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return Ok(error_response(
                "`name` is required and must be a non-empty string",
            ));
        }
    };
    let arguments = args
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    match svc
        .rpc(
            &server,
            "tools/call",
            json!({ "name": tool_name, "arguments": arguments }),
        )
        .await
    {
        Ok(value) => Ok(value),
        Err(e) => Ok(error_response(&format!(
            "upstream `{server}` rejected tools/call for `{tool_name}`: {e}"
        ))),
    }
}

fn error_response(message: &str) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": message,
        }],
        "isError": true,
    })
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn list_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

pub fn list_description() -> &'static str {
    "Aggregate tools/list across every configured upstream MCP \
     server (~/.easynet/mcp_clients.json). Returns one server entry \
     per configured upstream; an entry with `error` set indicates \
     that specific upstream failed without taking the others down."
}

pub fn call_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["server", "name"],
        "properties": {
            "server": {"type": "string", "minLength": 1},
            "name":   {"type": "string", "minLength": 1},
            "arguments": {
                "description": "Per-tool args; shape per the upstream tool's input schema."
            },
        },
        "additionalProperties": false,
    })
}

pub fn call_description() -> &'static str {
    "Forward a tools/call to a configured upstream MCP server. \
     `server` is the operator-chosen name from mcp_clients.json; \
     `name` is the upstream tool's name (as it appears in that \
     upstream's tools/list). Returns the upstream's tools/call \
     response verbatim (MCP {content, isError} shape)."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::execution::mcp_client::{McpClientsFile, McpServerSpec};
    use std::path::PathBuf;

    fn empty_svc() -> Arc<McpClientService> {
        Arc::new(McpClientService::new())
    }

    /// Spawn-time helper: write the same Python echo MCP script the
    /// service tests use, return a service pointing at it. The
    /// echo replies to every method with `{echoed: <params>}`, so
    /// list_handler will see `tools/list` returning that shape (no
    /// `tools` key) and exercise the missing-key tolerant path.
    fn echo_svc() -> (tempfile::TempDir, Arc<McpClientService>) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("mcp_echo.sh");
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
    if method == "tools/list":
        result = {"tools": [{"name": "echo", "description": "echoes the args", "inputSchema": {"type": "object"}}]}
    elif method == "tools/call":
        result = {"content": [{"type": "text", "text": json.dumps(req.get("params"))}], "isError": False}
    else:
        result = {"echoed": req.get("params")}
    write_msg({"jsonrpc": "2.0", "id": rid, "result": result})
'
"#,
        )
        .unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let svc = McpClientService::from_file(McpClientsFile {
            servers: vec![McpServerSpec {
                name: "echo".into(),
                command: script.to_string_lossy().to_string(),
                stdio_framing: "content-length".into(),
                ..Default::default()
            }],
        });
        (dir, Arc::new(svc))
    }

    #[test]
    fn registration_makes_both_dispatchable() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, empty_svc());
        assert!(reg.get_rpc(ABILITY_LIST).is_some());
        assert!(reg.get_rpc(ABILITY_CALL).is_some());
    }

    #[test]
    fn list_with_no_configured_servers_returns_empty_array() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, empty_svc());
        let handler = reg.get_rpc(ABILITY_LIST).unwrap();
        let resp = handler(json!({})).unwrap();
        let servers = resp["servers"].as_array().expect("servers is array");
        assert!(
            servers.is_empty(),
            "no config → empty array, not missing key"
        );
    }

    #[test]
    fn call_missing_server_field_returns_is_error() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, empty_svc());
        let handler = reg.get_rpc(ABILITY_CALL).unwrap();
        let resp = handler(json!({"name": "echo"})).unwrap();
        assert_eq!(resp["isError"], true);
        assert!(resp["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("`server`"));
    }

    #[test]
    fn call_missing_name_field_returns_is_error() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, empty_svc());
        let handler = reg.get_rpc(ABILITY_CALL).unwrap();
        let resp = handler(json!({"server": "echo"})).unwrap();
        assert_eq!(resp["isError"], true);
        assert!(resp["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("`name`"));
    }

    #[test]
    fn call_unknown_server_returns_is_error_naming_the_server() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, empty_svc());
        let handler = reg.get_rpc(ABILITY_CALL).unwrap();
        let resp = handler(json!({"server": "ghost", "name": "echo"})).unwrap();
        assert_eq!(resp["isError"], true);
        let text = resp["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("ghost"),
            "error must name the missing server; got {text:?}"
        );
    }

    #[test]
    fn list_input_schema_is_empty_object() {
        let s = list_input_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"].as_object().unwrap().is_empty());
    }

    #[test]
    fn call_input_schema_requires_server_and_name() {
        let s = call_input_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "server"));
        assert!(req.iter().any(|v| v == "name"));
    }

    #[test]
    #[cfg(unix)]
    fn list_round_trips_through_a_real_upstream() {
        // E2E: the echo upstream answers tools/list with one tool;
        // mcp.client.list aggregates and surfaces it in the
        // {servers:[{name, tools:[...]}]} envelope.
        let (_dir, svc) = echo_svc();
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, svc);
        let handler = reg.get_rpc(ABILITY_LIST).unwrap();
        let resp = handler(json!({})).unwrap();
        let servers = resp["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["name"], "echo");
        let tools = servers[0]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1, "echo upstream advertises one tool");
        assert_eq!(tools[0]["name"], "echo");
    }

    #[test]
    #[cfg(unix)]
    fn call_round_trips_through_a_real_upstream() {
        // E2E: tools/call to the echo upstream returns
        // {content:[{type:"text", text:<json-of-params>}], isError:false}.
        // We assert isError is false and the content carries our
        // arguments json.
        let (_dir, svc) = echo_svc();
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg, svc);
        let handler = reg.get_rpc(ABILITY_CALL).unwrap();
        let resp = handler(json!({
            "server": "echo",
            "name": "echo",
            "arguments": {"hello": "world"}
        }))
        .unwrap();
        assert_eq!(resp["isError"], false);
        let text = resp["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("hello"),
            "echoed text must carry our args; got {text:?}"
        );
        assert!(text.contains("world"));
    }

    #[test]
    fn from_path_with_configured_servers_indexes_them() {
        // Pin the on-disk file path the daemon will read so a future
        // refactor that renamed the config file would trip this.
        // We don't assert the exact path string — only the shape.
        let svc = McpClientService::from_path(&PathBuf::from(
            "/this/path/should/not/exist/__mcp_clients.json",
        ))
        .expect("missing file → empty service, not error");
        let names = futures::executor::block_on(svc.server_names());
        assert!(names.is_empty());
    }
}

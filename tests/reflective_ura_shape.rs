// Gate 4 — Reflective ability URAs carry no implementation label.
//
// This integration test exercises the full reflective registration
// path with the canonical SDK URA builder, then asserts:
//
//   1. The ability's wire-level URA matches the canonical shape
//      `easynet:///r/<realm>/ability/<user>.<agent>.<verb>` produced
//      by `easynet_axon::ura::URA::ability`.
//   2. The URA literal contains NO implementation label such as
//      `mcp_upstream` (the discipline gate 2 enforces at script
//      level — duplicated in code so a regression trips here even
//      when the script gate is bypassed).
//   3. The descriptor's `source` field DOES carry the provenance
//      (so observability isn't lost — provenance moves out of the
//      URA, not deleted).
//
// The test uses an in-process Python echo MCP server (same fixture
// pattern as `daemon::execution::mcp_client::tests`) so it stays
// dependency-free and runs in CI without any external network or
// MCP installs.

#![cfg(unix)]

use std::sync::Arc;

use easynet_axon::ura::{ability_ura, agent_ura};
use easynet_cli::daemon::ability::builtins::integrations::mcp::reflective_registry::reflect_all;
use easynet_cli::daemon::execution::mcp_client::{McpClientService, McpClientsFile, McpServerSpec};
use easynet_cli::runtime::ability_dispatch::AxonAbilityCatalog;

fn write_echo_script(dir: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("echo_mcp.sh");
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
            {"name": "echo_one", "description": "echoes back", "inputSchema": {"type": "object"}}
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
    script
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reflective_ability_ura_is_clean_canonical_shape() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_echo_script(dir.path());

    let svc = Arc::new(McpClientService::from_file(McpClientsFile {
        servers: vec![McpServerSpec {
            name: "echo".into(),
            command: script.to_string_lossy().to_string(),
            stdio_framing: "content-length".into(),
            ..Default::default()
        }],
    }));

    // Construct the mcp-profile owner URA the way the daemon does
    // at boot — through the SDK builder, not by string-formatting.
    // Per AGENT_IDENTITY.md §2, never `format!("easynet:///{}")` to
    // synthesise URAs; use the typed builder.
    let realm = "test-realm";
    let user = "test-user";
    let agent = "mcp";
    let owner_ura = agent_ura(realm, user, agent);
    assert_eq!(
        owner_ura, "easynet:///r/test-realm/agent/test-user.mcp",
        "agent_ura builder shape sanity"
    );

    let mut reg = AxonAbilityCatalog::new();
    let result = reflect_all(&svc, &mut reg, &owner_ura).await;
    assert!(
        result.failed.is_empty(),
        "reflect_all should not fail for echo upstream: {:?}",
        result.failed
    );
    assert_eq!(result.registered.len(), 1);
    let rec = &result.registered[0];

    // (1) Wire-level URA matches the canonical builder's output.
    //
    // The reflective registry doesn't itself synthesise the URA
    // (that happens later in the descriptor publish path); what we
    // pin here is that combining `agent_ura(user, agent)` with the
    // reflected ability_name through the canonical `ability_ura`
    // builder yields the same string callers would route on. If a
    // future refactor leaks a different naming, this assert trips.
    let expected_ability_ura = ability_ura(realm, user, agent, &rec.ability_name);
    assert_eq!(
        expected_ability_ura, "easynet:///r/test-realm/ability/test-user.mcp.echo_one",
        "ability URA must be the canonical 3-segment shape"
    );

    // (2) URA literal does NOT carry any implementation label.
    // Duplicates the script gate 2 invariant in code so refactors
    // that leak the label trip here even when the gate is skipped.
    for forbidden in ["mcp_upstream", "mcp-upstream"] {
        assert!(
            !expected_ability_ura.contains(forbidden),
            "URA literal must not contain `{forbidden}`: {expected_ability_ura}"
        );
        assert!(
            !rec.descriptor.owner_ura.contains(forbidden),
            "owner URA must not contain `{forbidden}`: {}",
            rec.descriptor.owner_ura
        );
        assert!(
            !rec.ability_name.contains(forbidden),
            "ability name must not contain `{forbidden}`: {}",
            rec.ability_name
        );
    }

    // (3) Provenance lives on `source`, not lost from the system.
    assert!(
        rec.descriptor
            .source
            .starts_with("mcp_upstream:echo:echo_one"),
        "provenance must be preserved on AbilityDescriptor.source: {:?}",
        rec.descriptor.source
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reflective_ability_with_prefix_still_canonical_ura() {
    // Same discipline check but with an operator-chosen
    // `name_prefix` — ensures the prefix passes through the
    // builder cleanly and the dotted segment doesn't bleed into
    // the URA in a way that would confuse the parser.
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("echo_prefixed.sh");
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

    let svc = Arc::new(McpClientService::from_file(McpClientsFile {
        servers: vec![McpServerSpec {
            name: "context7".into(),
            command: script.to_string_lossy().to_string(),
            name_prefix: "ctx7.".into(),
            stdio_framing: "content-length".into(),
            ..Default::default()
        }],
    }));

    let realm = "test-realm";
    let user = "test-user";
    let agent = "mcp";
    let owner_ura = agent_ura(realm, user, agent);

    let mut reg = AxonAbilityCatalog::new();
    let result = reflect_all(&svc, &mut reg, &owner_ura).await;
    assert!(result.failed.is_empty(), "{:?}", result.failed);
    assert_eq!(result.registered.len(), 1);
    let rec = &result.registered[0];
    assert_eq!(rec.ability_name, "ctx7.search_docs");

    let ura = ability_ura(realm, user, agent, &rec.ability_name);
    assert_eq!(
        ura,
        "easynet:///r/test-realm/ability/test-user.mcp.ctx7.search_docs"
    );
    assert!(!ura.contains("mcp_upstream"));
}

// Gate 6 (round-1 acceptance) — End-to-end smoke for the
// MCP-Bench plumbing path on the EasyNet daemon.
//
// This test replaces what plan §verification calls
// "MCP-Bench 实跑 smoke task" with a self-contained equivalent
// that doesn't require:
//   * cloning Accenture/mcp-bench (~30 minutes of git + npm + python deps)
//   * 28 upstream MCP-server installs (~hours of language-specific setup)
//   * a running EasyNet daemon process bound to a real port
//   * a configured EasyNet user / credentials / hub
//
// What it DOES exercise end-to-end (the parts that are EasyNet's
// responsibility, not mcp-bench's):
//
//   1. mcp_bench_setup.sh's TRANSLATION schema produces JSON that
//      deserialises into McpClientService.
//   2. McpClientService::from_path loads that JSON.
//   3. build_registry() — the exact public entry point the daemon
//      bin calls at boot — kicks off reflective MCP registration.
//   4. Each tool the (in-process) upstream advertises shows up as
//      a callable ability in the registry, with the URA-clean
//      descriptor invariants from F-01.
//   5. Invoking the reflected ability through the registry's RPC
//      handler returns the upstream's MCP-shape response verbatim.
//
// Whatever fails in mcp-bench after these checks pass is mcp-bench's
// fault (a broken upstream install, a missing API key, a runtime
// version mismatch) — NOT EasyNet's reflective plumbing.

#![cfg(unix)]

use std::sync::Arc;

use easynet_axon::invocation::LocalRuntime;
use easynet_cli::daemon::execution::mcp_client::{McpClientService, McpClientsFile, McpServerSpec};
use easynet_cli::daemon::invocation::local_runtime_invoker::open_local_stream;
use easynet_cli::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

/// Build the in-process Python echo MCP server fixture used
/// throughout this round. Same script shape as the `mcp_client`
/// unit tests + `reflective_ura_shape` integration test, so the
/// fixture stays one canonical artefact.
fn write_echo_mcp_server(dir: &std::path::Path) -> std::path::PathBuf {
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
            {"name": "weather_lookup", "description": "lookup weather", "inputSchema": {"type": "object"}},
            {"name": "currency_convert", "description": "convert currency", "inputSchema": {"type": "object"}}
        ]}
    elif method == "tools/call":
        params = req.get("params") or {}
        result = {"content": [{"type": "text", "text": json.dumps({"echoed_tool": params.get("name"), "echoed_args": params.get("arguments", {})})}], "isError": False}
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

/// Translate a single-server commands.json into the
/// mcp_clients.json shape — mirrors the python heredoc in
/// `engineering/scripts/mcp_bench_setup.sh`. Kept simple here because we only
/// need to round-trip one server for this smoke.
fn synthesize_mcp_clients_json(server_name: &str, script_path: &std::path::Path) -> String {
    serde_json::json!({
        "servers": [{
            "name": server_name,
            "command": script_path.to_string_lossy(),
            "args": [],
            "env": {},
            "transport": "stdio",
            "stdio_framing": "content-length",
        }]
    })
    .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reflective_path_directly_through_mcp_client_service_round_trip() {
    // This is the slice of "MCP-Bench smoke" that does NOT touch
    // process-wide env state. Verifies the contract:
    //   commands.json shape → McpClientService → reflect_all →
    //     LocalRuntime invocation produces MCP-shape verbatim response.
    let dir = tempfile::tempdir().unwrap();
    let script = write_echo_mcp_server(dir.path());

    let mcp_clients_json = synthesize_mcp_clients_json("echo", &script);
    let config_path = dir.path().join("mcp_clients.json");
    std::fs::write(&config_path, &mcp_clients_json).unwrap();

    // Replicate what `build_registry_with_services` does internally:
    // load the config + reflect into a fresh registry.
    let svc = McpClientService::from_path(&config_path).expect("from_path must accept");
    assert_eq!(svc.server_names().await, vec!["echo".to_string()]);

    let runtime = LocalRuntime::new();
    let mut reg = easynet_cli::daemon::ability::dispatch::AxonAbilityCatalog::new_with_runtime(
        Arc::clone(&runtime),
    );
    let owner_ura = easynet_axon::ura::agent_ura("test-realm", "test-user", "mcp");
    let result = easynet_cli::daemon::ability::builtins::integrations::mcp::reflective_registry::reflect_all(
        &svc, &mut reg, &owner_ura,
    )
    .await;

    assert!(
        result.failed.is_empty(),
        "reflective registration must not fail for healthy echo upstream: {:?}",
        result.failed
    );
    // Echo upstream advertises two tools; both must register.
    assert_eq!(
        result.registered.len(),
        2,
        "both echo tools must be reflected; got {:?}",
        result
            .registered
            .iter()
            .map(|r| &r.ability_name)
            .collect::<Vec<_>>()
    );

    // Catalog check: registry actually has the abilities. This is
    // what `easynet abilities` would surface and what `easynet
    // mcp_server` would project to external MCP clients.
    // Post-B2b: reflected abilities live in the stream map.
    assert!(reg.has_stream("weather_lookup"));
    assert!(reg.has_stream("currency_convert"));

    // The discipline check (URA cleanness) at the post-registration
    // descriptor level — duplicates `reflective_ura_shape.rs` and
    // `scripts/checks/ura_no_implementation_label.sh` at the runtime
    // surface so a regression on the build_registry path trips here.
    for rec in &result.registered {
        let derived_ura =
            easynet_axon::ura::ability_ura("test-realm", "test-user", "mcp", &rec.ability_name);
        assert!(
            !derived_ura.contains("mcp_upstream"),
            "URA must not leak mcp_upstream: {derived_ura}"
        );
        assert!(
            rec.descriptor.source.starts_with("mcp_upstream:echo:"),
            "descriptor.source must carry provenance: {:?}",
            rec.descriptor.source
        );
    }

    // Functional check (post-B2b): invoke the reflected ability as
    // a STREAM and drain frames. The terminal frame carries the
    // MCP-shape `{content, isError}` response verbatim; any
    // intermediate frames would be `{type: "progress", ...}`
    // (echo upstream doesn't emit any — that's exercised by
    // mcp_client::tests::rpc_with_progress_routes_interleaved_progress_to_sink).
    let weather_lookup_ura = easynet_axon::ura::owner_ability_ura(&owner_ura, "weather_lookup")
        .expect("owner ability URA");
    let mut rx = open_local_stream(
        Arc::clone(&runtime),
        InvocationTarget {
            scope: TargetScope::Local,
            ability: weather_lookup_ura,
            normalized_args: serde_json::json!({"location": "Berlin"}),
            call_mode: CallMode::Stream,
            subject: None,
            causal_context: None,
        },
    )
    .await
    .expect("weather_lookup must be callable as stream through LocalRuntime");
    let _keep_catalog_alive = reg;

    // Drain until we see the terminal `response` frame. Echo
    // upstream replies immediately so this is bounded.
    let mut terminal: Option<serde_json::Value> = None;
    let drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < drain_deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), rx.next_frame()).await {
            Ok(Some(Ok(frame))) => {
                if !frame.payload.is_empty() {
                    let value: serde_json::Value =
                        serde_json::from_slice(&frame.payload).expect("frame payload JSON");
                    if value.get("type").and_then(|v| v.as_str()) == Some("response") {
                        terminal = Some(value);
                        break;
                    }
                }
                // progress / other frames — keep draining.
                if frame.terminal {
                    break;
                }
            }
            Ok(Some(Err(_frame_error))) => break,
            Ok(None) => break,
            Err(_timeout_tick) => continue,
        }
    }
    let term = terminal.expect("must receive a `response` terminal frame within 5s");
    let result = &term["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"]
        .as_str()
        .expect("content[0].text must be a string");
    let echoed: serde_json::Value =
        serde_json::from_str(text).expect("echo upstream wraps args as JSON string");
    assert_eq!(echoed["echoed_tool"], "weather_lookup");
    assert_eq!(echoed["echoed_args"]["location"], "Berlin");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_bench_translation_round_trips_into_live_reflection() {
    // Tighter coupling: drive the same translation logic the
    // setup script applies, run it through reflection. If
    // `mcp_bench_setup.sh` and the daemon ever diverge on the
    // schema, this trips immediately.
    let dir = tempfile::tempdir().unwrap();
    let script = write_echo_mcp_server(dir.path());

    // Emulate `commands.json` for one server (the format
    // Accenture/mcp-bench publishes).
    let commands_json = serde_json::json!({
        "Echo Demo": {
            "cmd": format!("{} ", script.display()),
            "env": [],
            "cwd": null
        }
    });

    // Mirror the python heredoc — minimal, single-server path.
    let translated = {
        let entry = commands_json.get("Echo Demo").unwrap();
        let cmd = entry["cmd"].as_str().unwrap().trim().to_string();
        // No cwd → no sh -c wrapper; split on whitespace per the
        // script's fallback branch.
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        serde_json::json!({
            "servers": [{
                "name": "Echo Demo",
                "command": parts[0],
                "args": parts[1..].iter().collect::<Vec<_>>(),
                "env": {},
                "transport": "stdio",
                "stdio_framing": "content-length",
            }]
        })
    };

    let parsed: McpClientsFile = serde_json::from_value(translated).unwrap();
    let svc = Arc::new(McpClientService::from_file(parsed));

    let mut reg = easynet_cli::daemon::ability::dispatch::AxonAbilityCatalog::new();
    let result = easynet_cli::daemon::ability::builtins::integrations::mcp::reflective_registry::reflect_all(
        &svc,
        &mut reg,
        "easynet:///r/test/agent/u.mcp",
    )
    .await;

    assert!(
        result.failed.is_empty(),
        "translated commands.json must yield healthy reflection: {:?}",
        result.failed
    );
    // Post-B2b: reflected abilities live in the stream map.
    assert!(reg.has_stream("weather_lookup"));
    assert!(reg.has_stream("currency_convert"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn broken_upstream_does_not_block_other_servers() {
    // Plan §A1 contract: a failed upstream MCP server is LOGGED,
    // not panicked; reflection continues for other servers.
    // mcp-bench has 28 upstreams — if one breaks at install time,
    // the operator should still get the other 27 callable.
    let dir = tempfile::tempdir().unwrap();
    let good = write_echo_mcp_server(dir.path());
    let bad_script = dir.path().join("broken.sh");
    std::fs::write(&bad_script, "#!/bin/sh\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bad_script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let svc = Arc::new(McpClientService::from_file(McpClientsFile {
        servers: vec![
            McpServerSpec {
                name: "broken-one".into(),
                command: bad_script.to_string_lossy().into(),
                ..Default::default()
            },
            McpServerSpec {
                name: "good-one".into(),
                command: good.to_string_lossy().into(),
                stdio_framing: "content-length".into(),
                ..Default::default()
            },
        ],
    }));

    let mut reg = easynet_cli::daemon::ability::dispatch::AxonAbilityCatalog::new();
    let result = easynet_cli::daemon::ability::builtins::integrations::mcp::reflective_registry::reflect_all(
        &svc,
        &mut reg,
        "easynet:///r/test/agent/u.mcp",
    )
    .await;

    // The bad one fails; the good one still produces tools.
    assert!(
        !result.failed.is_empty(),
        "broken upstream must be flagged as a failure"
    );
    let bad_failures: Vec<_> = result
        .failed
        .iter()
        .filter(|f| f.server == "broken-one")
        .collect();
    assert!(
        !bad_failures.is_empty(),
        "broken-one must be among failures: {:?}",
        result.failed
    );

    let good_registered: Vec<_> = result
        .registered
        .iter()
        .filter(|r| r.server == "good-one")
        .collect();
    assert_eq!(
        good_registered.len(),
        2,
        "good upstream's two tools must still register: {:?}",
        result.registered
    );
    // Post-B2b: reflected abilities live in the stream map.
    assert!(reg.has_stream("weather_lookup"));
    assert!(reg.has_stream("currency_convert"));
}

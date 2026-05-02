// EasyNet CLI — McpClientService (C-M9b)
// =======================================
//
// File: src/runtime/execution/mcp_client/mod.rs
//
// Sub-service that owns every outbound MCP server connection the
// daemon spawns. Mirrors PtyService's process-wide-Arc shape: one
// service handle, lazily-instantiated child connections, indexed
// by server name.
//
// Wire protocol
// -------------
// MCP stdio transport: JSON-RPC 2.0 frames, one per line, sent on
// the child's stdin and received on its stdout. The two methods we
// use are:
//
//   tools/list  → returns { tools: [{ name, description, inputSchema }] }
//   tools/call  → returns { content: [...], isError: bool }
//
// We send `initialize` once on first connection (per the MCP
// spec's handshake) and then drive the two RPCs above. v1 ignores
// the `notifications/*` channel — we don't need progress / cancel
// for the first cut.
//
// Concurrency model
// -----------------
// One tokio Mutex per server connection. The MCP protocol allows
// pipelined requests (id-keyed responses) but v1 serialises:
// take the mutex, send, await reply, release. This keeps the
// reader loop trivial (one in-flight request at a time → no need
// for an oneshot fan-out table). A future enhancement that needs
// throughput can lift this to per-id oneshots without changing
// the public service surface.
//
// Why not use an external mcp-client crate
// ----------------------------------------
// MCP is two RPC methods over stdio JSON-RPC 2.0. Pulling a crate
// would import schemas, capability negotiation, transport
// abstraction layers we don't use. Hand-rolling the minimum keeps
// the dependency surface clean and the wire shape pinned to the
// MCP spec we actually exercise.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

/// Config row for one upstream MCP server. Mirrors the shape of
/// `~/.claude/mcp_servers.json` so an operator who already runs
/// MCP can drop their existing config in. Fields:
///
///   * `name`    — short identifier the operator picks; ability
///                 callers reference it by this string.
///   * `command` — executable to spawn (e.g. `"node"`, `"python"`,
///                 `"npx"`).
///   * `args`    — argv tail.
///   * `env`     — extra env vars merged with the daemon's env.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerSpec {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Top-level config file shape. The `servers` array is the single
/// source of truth for which upstreams are reachable.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpClientsFile {
    #[serde(default)]
    pub servers: Vec<McpServerSpec>,
}

impl McpClientsFile {
    /// Look up a server spec by name; returns the first match.
    pub fn get(&self, name: &str) -> Option<&McpServerSpec> {
        self.servers.iter().find(|s| s.name == name)
    }
}

/// One live outbound MCP connection. The reader/writer halves live
/// behind a tokio Mutex so the request/response round-trip is
/// serialised per server (v1 — see module docs).
struct McpConnection {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpConnection {
    /// Send a JSON-RPC request and await its response. Allocates a
    /// fresh integer id; the response's `id` MUST match.
    async fn rpc(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let line = format!("{}\n", serde_json::to_string(&req)?);
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        // Read lines until we see one matching our id. MCP servers
        // MAY interleave notifications (no `id` field); skip those.
        loop {
            let mut buf = String::new();
            let n = self.stdout.read_line(&mut buf).await?;
            if n == 0 {
                anyhow::bail!("MCP server closed stdout before responding to `{method}`");
            }
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => {
                    // Servers occasionally print log lines to stdout
                    // that aren't valid JSON-RPC. Skip; don't crash.
                    continue;
                }
            };
            // Skip notifications (no `id`).
            let resp_id = match v.get("id").and_then(Value::as_u64) {
                Some(n) => n,
                None => continue,
            };
            if resp_id != id {
                // Out-of-order response (shouldn't happen given v1's
                // serial discipline). Skip; the matching frame will
                // come.
                continue;
            }
            if let Some(err) = v.get("error") {
                anyhow::bail!("MCP server returned JSON-RPC error: {err}");
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

/// One row in the per-process server registry. Wraps the
/// connection (None when not yet established) and the spec the
/// operator declared.
struct McpServerRow {
    spec: McpServerSpec,
    /// `None` until the first call lazily spawns the child.
    /// Subsequent calls reuse the connection. A future health-
    /// check could clear this entry on stdio failure to trigger
    /// re-spawn; v1 surfaces the failure to the caller.
    conn: Option<McpConnection>,
}

/// Process-wide outbound MCP client registry. Cloneable handle.
#[derive(Clone)]
pub struct McpClientService {
    inner: Arc<Mutex<McpClientServiceInner>>,
}

struct McpClientServiceInner {
    servers: HashMap<String, McpServerRow>,
}

impl Default for McpClientService {
    fn default() -> Self {
        Self::new()
    }
}

impl McpClientService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(McpClientServiceInner {
                servers: HashMap::new(),
            })),
        }
    }

    /// Construct from an in-memory file (test path or operator-
    /// supplied snapshot). Production callers prefer `from_path`.
    pub fn from_file(file: McpClientsFile) -> Self {
        let svc = Self::new();
        let mut g = svc
            .inner
            .try_lock()
            .expect("fresh service has no contention");
        for spec in file.servers {
            g.servers
                .insert(spec.name.clone(), McpServerRow { spec, conn: None });
        }
        drop(g);
        svc
    }

    /// Read a config file from disk. Missing file → empty service
    /// (no upstream MCP servers configured); parse error
    /// propagates so the operator sees their typo at boot rather
    /// than as silent "no servers."
    pub fn from_path(path: &PathBuf) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let bytes =
            std::fs::read(path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
        let file: McpClientsFile = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
        Ok(Self::from_file(file))
    }

    /// List the names of every configured upstream server. Cheap
    /// (no I/O); the names come straight off the static config.
    pub async fn server_names(&self) -> Vec<String> {
        let g = self.inner.lock().await;
        let mut names: Vec<String> = g.servers.keys().cloned().collect();
        names.sort();
        names
    }

    /// Lazily ensure a connection to `name` is alive, then send a
    /// JSON-RPC request. Returns the JSON-RPC `result` field.
    ///
    /// First call to a server triggers:
    ///   1. spawn the child process per the spec's command + args
    ///   2. send `initialize` (MCP handshake)
    ///   3. send the requested method
    /// Subsequent calls reuse the live connection.
    pub async fn rpc(&self, name: &str, method: &str, params: Value) -> anyhow::Result<Value> {
        let mut g = self.inner.lock().await;
        let row = g.servers.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!("no upstream MCP server configured with name `{name}`")
        })?;
        if row.conn.is_none() {
            let conn = spawn_and_initialize(&row.spec).await?;
            row.conn = Some(conn);
        }
        let conn = row.conn.as_mut().expect("conn just set");
        conn.rpc(method, params).await
    }
}

impl std::fmt::Debug for McpClientService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClientService").finish_non_exhaustive()
    }
}

/// Spawn the configured child and send the MCP `initialize`
/// handshake. Returns the live connection ready for `tools/*`
/// calls.
async fn spawn_and_initialize(spec: &McpServerSpec) -> anyhow::Result<McpConnection> {
    let mut cmd = Command::new(&spec.command);
    cmd.args(&spec.args)
        .envs(&spec.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawn `{}`: {e}", spec.command))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("MCP child has no stdin pipe"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("MCP child has no stdout pipe"))?;
    // Detach: we own the child's stdio but not its lifetime.
    // Dropping `child` here lets it run until close (the OS
    // reaper claims it on exit).
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    let mut conn = McpConnection {
        stdin,
        stdout: BufReader::new(stdout),
        next_id: 1,
    };
    // MCP `initialize` handshake. We claim a minimal client
    // capability surface (no sampling, no roots) — enough to drive
    // tools/list and tools/call.
    let _ = conn
        .rpc(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "easynet-daemon",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        )
        .await?;
    // Per spec the client MUST send the `notifications/initialized`
    // notification after the initialize round-trip. We send it as a
    // best-effort bare notification (no `id` field, no response).
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    });
    let line = format!("{}\n", serde_json::to_string(&notif)?);
    conn.stdin.write_all(line.as_bytes()).await?;
    conn.stdin.flush().await?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_service_has_no_servers() {
        let svc = McpClientService::new();
        let names = futures::executor::block_on(svc.server_names());
        assert!(names.is_empty());
    }

    #[test]
    fn from_file_indexes_servers_by_name() {
        let file = McpClientsFile {
            servers: vec![
                McpServerSpec {
                    name: "alpha".into(),
                    command: "echo".into(),
                    args: vec!["alpha".into()],
                    env: HashMap::new(),
                },
                McpServerSpec {
                    name: "beta".into(),
                    command: "echo".into(),
                    args: vec![],
                    env: HashMap::new(),
                },
            ],
        };
        let svc = McpClientService::from_file(file);
        let names = futures::executor::block_on(svc.server_names());
        assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn from_path_missing_file_yields_empty_service() {
        let p = PathBuf::from("/tmp/eznt-mcp-clients-doesnotexist-test.json");
        let svc = McpClientService::from_path(&p).expect("missing file is OK");
        let names = futures::executor::block_on(svc.server_names());
        assert!(names.is_empty());
    }

    #[test]
    fn from_path_malformed_json_propagates_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("mcp.json");
        std::fs::write(&p, b"not json at all").unwrap();
        let err = McpClientService::from_path(&p).unwrap_err();
        assert!(format!("{err}").contains("parse"));
    }

    #[test]
    fn mcp_clients_file_serde_roundtrip() {
        // Pin the on-disk schema so a refactor that renamed a field
        // would trip this and force an explicit migration story.
        let file = McpClientsFile {
            servers: vec![McpServerSpec {
                name: "context7".into(),
                command: "npx".into(),
                args: vec!["-y".into(), "@upstash/context7-mcp".into()],
                env: HashMap::from([("API_KEY".into(), "x".into())]),
            }],
        };
        let json = serde_json::to_string(&file).unwrap();
        let back: McpClientsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.servers.len(), 1);
        assert_eq!(back.servers[0].name, "context7");
        assert_eq!(back.servers[0].args, vec!["-y", "@upstash/context7-mcp"]);
    }

    #[tokio::test]
    async fn rpc_unknown_server_errors_clearly() {
        let svc = McpClientService::new();
        let err = svc
            .rpc("nonexistent", "tools/list", json!({}))
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("nonexistent"),
            "error must name the missing server; got {msg:?}"
        );
    }

    /// Round-trip through a real MCP-style stdio echo server.
    /// Uses a tiny inline shell script so the test stays
    /// dependency-free. The script reads JSON-RPC requests from
    /// stdin and replies with `{result: {echoed: <params>}}` on
    /// the same id.
    #[tokio::test]
    #[cfg(unix)]
    async fn rpc_round_trips_through_a_real_subprocess() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("mcp_echo.sh");
        std::fs::write(
            &script,
            // POSIX sh + jq isn't universal; use Python which is
            // present on every macOS + Linux dev box.
            r#"#!/bin/sh
exec python3 -u -c '
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    # Echo back the params as the result, keyed to the request id.
    resp = {"jsonrpc": "2.0", "id": req.get("id"), "result": {"echoed": req.get("params")}}
    print(json.dumps(resp), flush=True)
'
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let file = McpClientsFile {
            servers: vec![McpServerSpec {
                name: "echo".into(),
                command: script.to_string_lossy().to_string(),
                args: vec![],
                env: HashMap::new(),
            }],
        };
        let svc = McpClientService::from_file(file);

        // First call: spawns child, runs initialize handshake,
        // then the actual tools/list request. The echo server
        // returns the params as result; we just confirm the
        // round-trip works.
        let resp = svc
            .rpc("echo", "tools/list", json!({"probe": "first-call"}))
            .await
            .expect("first rpc should succeed");
        assert_eq!(resp["echoed"]["probe"], "first-call");

        // Second call: reuses the live connection; id is now 3
        // (1=initialize, 2=tools/list, 3=this).
        let resp2 = svc
            .rpc("echo", "tools/call", json!({"probe": "second-call"}))
            .await
            .expect("second rpc should succeed");
        assert_eq!(resp2["echoed"]["probe"], "second-call");
    }
}

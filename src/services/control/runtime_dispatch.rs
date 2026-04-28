// EasyNet CLI — runtime-dispatch UDS responder (Step 3, daemon side)
// =====================================================================
//
// File: src/services/control/runtime_dispatch.rs
//
// Companion to EasyNet-Axon's `runtime_local_tools` registry +
// `try_dispatch_runtime_local_tool` hook (axon-runtime). When an
// external Invoke arrives at axon-runtime for an ability the daemon
// registered via `runtime.register_local_tool`, axon-runtime opens
// a Unix domain socket connection to the registered
// `dispatch_endpoint` and sends a single-line JSON request. This
// module is the daemon-side acceptor for that socket.
//
// Wire shape (mirrors core/runtime-rs/src/interop_native/execution.rs)
// --------------------------------------------------------------------
//
//   request line  (terminated by \n):
//     {"tool_name":"<x>","function_name":"<y>","arguments_b64":"<base64>"}
//
//   response line (terminated by \n):
//     {"ok":true,  "result_b64":"<base64>", "content_type":"application/json"}
//   OR:
//     {"ok":false, "code":"<typed>",        "message":"<human>"}
//
// One request per accepted connection, then close. Matches the
// runtime side's per-request UDS connection (no pooling) — keeps
// both halves simple at the cost of a syscall per call. Future
// optimisation: keepalive + multiplexing, but only if profiling
// shows a hot path here.
//
// Why a separate UDS from `control.sock`
// --------------------------------------
// `control.sock` already serves CLI subcommands + the local stdio
// MCP server with a length-delimited JSON IPC framing (see
// `services/control/server.rs::serve_connection`). Multiplexing
// runtime-dispatch's newline-delimited single-line shape onto the
// same socket would require either peek-and-classify framing
// detection or a magic byte — both fragile. Two sockets cost one
// extra `bind` syscall at boot and zero ambiguity.
//
// Default path: `~/.easynet/runtime-dispatch.sock`. Override via
// `EASYNET_RUNTIME_DISPATCH_SOCK` env var. The path is what the
// daemon also passes as `dispatch_endpoint` (with `ipc://` prefix)
// in `runtime.register_local_tool`, so the runtime's lookup hits
// exactly the file this module owns.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::services::control::ability_proxy::AbilityProxy;

/// One incoming request on the runtime-dispatch UDS. Mirrors the
/// shape `axon-runtime/.../execution.rs::try_dispatch_runtime_local_tool`
/// emits — adding fields here without coordinating axon-runtime
/// would silently break dispatch. `#[serde(default)]` on every
/// field keeps an older daemon tolerant of a future axon adding
/// trailing arguments.
#[derive(Debug, Deserialize)]
struct DispatchRequest {
    #[serde(default)]
    tool_name: String,
    /// MCP-shape function-name. Today the daemon's abilities are
    /// single-method (the ability name IS the call), so this is
    /// usually empty. Logged for observability only.
    #[serde(default)]
    function_name: String,
    /// Base64-encoded raw bytes of the call arguments. The daemon
    /// decodes once, parses as JSON (today every ability accepts
    /// JSON args), invokes, and re-encodes the result.
    #[serde(default)]
    arguments_b64: String,
}

/// Success response shape.
#[derive(Debug, Serialize)]
struct DispatchOk {
    ok: bool,
    result_b64: String,
    content_type: String,
}

/// Failure response shape. `ok=false` is the discriminator.
#[derive(Debug, Serialize)]
struct DispatchErr {
    ok: bool,
    code: String,
    message: String,
}

/// Default UDS path under `~/.easynet/`. Picked to be visually
/// distinct from `control.sock` so an operator doing `lsof` can
/// see "this is the runtime-dispatch socket" without referring
/// to a doc.
pub const DEFAULT_RUNTIME_DISPATCH_SOCK_NAME: &str = "runtime-dispatch.sock";

/// Compute the socket path the daemon binds. The override env var
/// is the integration-test seam: a fixture can write to a temp
/// path, and exposes the socket location to a fake axon-runtime
/// without touching `~/.easynet/`.
pub fn dispatch_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("EASYNET_RUNTIME_DISPATCH_SOCK") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    crate::persistence::config::state_dir().join(DEFAULT_RUNTIME_DISPATCH_SOCK_NAME)
}

/// Build the matching `ipc://...` URI the daemon registers as the
/// `dispatch_endpoint`. Always uses the resolved socket path; an
/// integration test can predict-then-verify without re-deriving the
/// override logic.
pub fn dispatch_endpoint_uri() -> String {
    format!("ipc://{}", dispatch_socket_path().display())
}

/// Bind the runtime-dispatch UDS, advertise its location for
/// debugging, and run the accept loop until the listener is
/// closed. Mirrors `services/control/server.rs::run` shape so the
/// daemon bin can wire both servers symmetrically.
///
/// Idempotent against a stale socket file: an `EADDRINUSE` from a
/// previous daemon process that died without unlinking gets
/// removed and retried once. A genuine collision (another live
/// process holding the socket) surfaces on the second attempt and
/// aborts boot — operators see the conflict cleanly rather than
/// running two daemons that disagree about who serves invokes.
pub async fn run(proxy: AbilityProxy) -> anyhow::Result<()> {
    let path = dispatch_socket_path();
    let listener = bind_socket(&path).await?;
    eprintln!(
        "[runtime-dispatch] listening at {} (Step 3 wire to axon-runtime)",
        path.display()
    );
    accept_loop(listener, proxy).await
}

/// Bind the socket, recovering from a stale file left by a prior
/// daemon crash. Async because the liveness probe (UnixStream
/// connect) is async.
async fn bind_socket(path: &Path) -> anyhow::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    match UnixListener::bind(path) {
        Ok(l) => Ok(l),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            // Stale socket from a prior daemon crash. Probe — if a
            // live process accepts, we abort; otherwise we unlink
            // and retry.
            if UnixStream::connect(path).await.is_ok() {
                anyhow::bail!(
                    "another process already accepts on {} — refusing to overwrite",
                    path.display()
                );
            }
            let _ = std::fs::remove_file(path);
            UnixListener::bind(path).map_err(|e| {
                anyhow::anyhow!("rebind {} after stale unlink: {e}", path.display())
            })
        }
        Err(e) => Err(anyhow::anyhow!("bind {}: {e}", path.display())),
    }
}

/// Accept connections forever, spawn one task per connection.
/// One request per connection; the task ends after writing the
/// response.
pub async fn accept_loop(
    listener: UnixListener,
    proxy: AbilityProxy,
) -> anyhow::Result<()> {
    loop {
        let (stream, _peer) = listener.accept().await?;
        let proxy = proxy.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_one(stream, proxy).await {
                // Per-connection failures never crash the loop. We
                // log via eprintln (mirrors server.rs); a future
                // structured-logging pass routes both modules
                // through `tracing`.
                eprintln!("[runtime-dispatch] connection error: {e:#}");
            }
        });
    }
}

/// Drive a single accepted connection: read one line, dispatch,
/// write one line, close.
async fn serve_one(stream: UnixStream, proxy: AbilityProxy) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(()); // peer closed without sending
    }

    let response_line = build_response_line(&line, &proxy);
    write_half.write_all(response_line.as_bytes()).await?;
    write_half.flush().await?;
    Ok(())
}

/// Pure decision function — what response should we send for a
/// given request line + proxy state. Pulled out as a free
/// function so it's exercised by unit tests without needing a real
/// socket. ALWAYS returns a single line ending in `\n` — the
/// runtime side reads exactly one line and tolerates no other
/// shape.
fn build_response_line(request_line: &str, proxy: &AbilityProxy) -> String {
    let req: DispatchRequest = match serde_json::from_str(request_line.trim()) {
        Ok(r) => r,
        Err(e) => {
            return error_line("BAD_REQUEST", format!("malformed request: {e}"));
        }
    };
    if req.tool_name.trim().is_empty() {
        return error_line("BAD_REQUEST", "tool_name must be non-empty".into());
    }
    let _ = req.function_name; // logged at trace level in a future PR

    let args_bytes = match base64::engine::general_purpose::STANDARD
        .decode(req.arguments_b64.as_bytes())
    {
        Ok(b) => b,
        Err(e) => {
            return error_line("BAD_REQUEST", format!("arguments_b64 decode: {e}"));
        }
    };
    let args_value: Value = if args_bytes.is_empty() {
        Value::Object(Default::default())
    } else {
        match serde_json::from_slice(&args_bytes) {
            Ok(v) => v,
            Err(e) => {
                return error_line(
                    "BAD_REQUEST",
                    format!("decoded arguments_b64 is not valid JSON: {e}"),
                );
            }
        }
    };

    match proxy.execute_runtime_dispatch(&req.tool_name, args_value) {
        Ok(value) => {
            let bytes = match serde_json::to_vec(&value) {
                Ok(b) => b,
                Err(e) => {
                    return error_line(
                        "INTERNAL",
                        format!("serialise result: {e}"),
                    );
                }
            };
            let body = DispatchOk {
                ok: true,
                result_b64: base64::engine::general_purpose::STANDARD.encode(&bytes),
                content_type: "application/json".to_string(),
            };
            let mut s = serde_json::to_string(&body).expect("serialise ok");
            s.push('\n');
            s
        }
        Err(msg) => {
            // Translate "no local handler registered" into NOT_FOUND
            // so the runtime side surfaces a precise typed error;
            // anything else is generic ABILITY_FAILED.
            let code = if msg.contains("no local handler registered")
                || msg.contains("no local stream handler registered")
                || msg.contains("no local bidi handler registered")
            {
                "NOT_FOUND"
            } else {
                "ABILITY_FAILED"
            };
            error_line(code, msg)
        }
    }
}

fn error_line(code: &str, message: String) -> String {
    let body = DispatchErr {
        ok: false,
        code: code.to_string(),
        message,
    };
    let mut s = serde_json::to_string(&body).expect("serialise err");
    s.push('\n');
    s
}

// keep async wrapper public-ish for tests
#[cfg(test)]
pub(crate) async fn build_response_line_for_test(
    request_line: &str,
    proxy: &AbilityProxy,
) -> String {
    build_response_line(request_line, proxy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::control::ability_proxy::AbilityProxy;

    /// Bare proxy used by every test. The dispatcher under it is
    /// the live system-ability registry — `observe.health` is the
    /// canonical "always-registered, no fixture needed" probe.
    fn fresh_proxy() -> AbilityProxy {
        use crate::runtime::gateway::NoopGateway;
        use crate::runtime::kernel::Kernel;
        use crate::runtime::kernel_api::KernelApi;
        use std::sync::Arc;
        let kernel: Arc<dyn KernelApi> =
            Arc::new(Kernel::new(Arc::new(NoopGateway::new())));
        AbilityProxy::new(kernel)
    }

    /// Both env-var tests serialize through this mutex because
    /// they mutate the process-global env. cargo test runs
    /// integration tests in parallel by default; without the
    /// mutex the two would race and one would observe the
    /// other's set_var clobbered before the assertion runs.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn dispatch_socket_path_uses_env_override_when_set() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("EASYNET_RUNTIME_DISPATCH_SOCK").ok();
        std::env::set_var(
            "EASYNET_RUNTIME_DISPATCH_SOCK",
            "/tmp/test-override.sock",
        );
        let p = dispatch_socket_path();
        assert_eq!(p.to_string_lossy(), "/tmp/test-override.sock");
        match prev {
            Some(v) => std::env::set_var("EASYNET_RUNTIME_DISPATCH_SOCK", v),
            None => std::env::remove_var("EASYNET_RUNTIME_DISPATCH_SOCK"),
        }
    }

    #[test]
    fn dispatch_endpoint_uri_uses_ipc_prefix() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("EASYNET_RUNTIME_DISPATCH_SOCK").ok();
        std::env::set_var("EASYNET_RUNTIME_DISPATCH_SOCK", "/tmp/x.sock");
        let uri = dispatch_endpoint_uri();
        assert_eq!(uri, "ipc:///tmp/x.sock");
        match prev {
            Some(v) => std::env::set_var("EASYNET_RUNTIME_DISPATCH_SOCK", v),
            None => std::env::remove_var("EASYNET_RUNTIME_DISPATCH_SOCK"),
        }
    }

    #[test]
    fn malformed_request_line_returns_bad_request() {
        let proxy = fresh_proxy();
        let resp = build_response_line("not a json", &proxy);
        let v: Value = serde_json::from_str(resp.trim()).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["code"], "BAD_REQUEST");
        assert!(v["message"].as_str().unwrap().contains("malformed"));
    }

    #[test]
    fn empty_tool_name_returns_bad_request() {
        let proxy = fresh_proxy();
        let resp = build_response_line(
            r#"{"tool_name":"","arguments_b64":""}"#,
            &proxy,
        );
        let v: Value = serde_json::from_str(resp.trim()).unwrap();
        assert_eq!(v["code"], "BAD_REQUEST");
        assert!(v["message"].as_str().unwrap().contains("tool_name"));
    }

    #[test]
    fn bad_base64_arguments_returns_bad_request() {
        // `!` is outside the base64 alphabet (URL or standard). A
        // bare-`@` string actually decodes (some base64 libs accept
        // it as no-op padding) so we use a guaranteed-invalid char.
        let proxy = fresh_proxy();
        let resp = build_response_line(
            r#"{"tool_name":"observe.health","arguments_b64":"!!!"}"#,
            &proxy,
        );
        let v: Value = serde_json::from_str(resp.trim()).unwrap();
        assert_eq!(v["code"], "BAD_REQUEST");
        assert!(
            v["message"].as_str().unwrap().to_ascii_lowercase().contains("base64")
                || v["message"].as_str().unwrap().contains("decode")
                || v["message"].as_str().unwrap().contains("not valid JSON"),
            "expected a base64-related error; got {:?}",
            v["message"]
        );
    }

    #[test]
    fn empty_arguments_default_to_empty_object() {
        // observe.health accepts {} — empty arguments_b64 must be
        // treated as {} not as a bad-request.
        let proxy = fresh_proxy();
        let resp = build_response_line(
            r#"{"tool_name":"observe.health","arguments_b64":""}"#,
            &proxy,
        );
        let v: Value = serde_json::from_str(resp.trim()).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v["result_b64"].as_str().is_some());
        assert_eq!(v["content_type"], "application/json");
    }

    #[test]
    fn unknown_ability_returns_not_found() {
        let proxy = fresh_proxy();
        let resp = build_response_line(
            r#"{"tool_name":"nope.does_not_exist","arguments_b64":""}"#,
            &proxy,
        );
        let v: Value = serde_json::from_str(resp.trim()).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["code"], "NOT_FOUND");
    }

    #[test]
    fn observe_health_round_trip_ok() {
        // Real RPC through the dispatcher — observe.health has a
        // built-in handler that returns a structured object. We
        // verify the result_b64 decodes to JSON and isn't empty.
        let proxy = fresh_proxy();
        let resp = build_response_line(
            r#"{"tool_name":"observe.health","arguments_b64":""}"#,
            &proxy,
        );
        let v: Value = serde_json::from_str(resp.trim()).unwrap();
        assert_eq!(v["ok"], true);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(v["result_b64"].as_str().unwrap())
            .unwrap();
        let inner: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(inner.is_object(), "observe.health returns an object");
    }

    #[test]
    fn json_args_decoded_then_passed_to_dispatcher() {
        // observe.health echoes its args through `ts`. We send a
        // marker arg and verify the dispatch path didn't drop it.
        let proxy = fresh_proxy();
        let args = serde_json::json!({"client_marker":"e2e-step3"}).to_string();
        let args_b64 = base64::engine::general_purpose::STANDARD.encode(args.as_bytes());
        let req = format!(
            r#"{{"tool_name":"observe.health","arguments_b64":"{args_b64}"}}"#
        );
        let resp = build_response_line(&req, &proxy);
        let v: Value = serde_json::from_str(resp.trim()).unwrap();
        assert_eq!(v["ok"], true);
        // We don't pin the inner shape — observe.health may or may
        // not echo client_marker depending on its implementation.
        // The pin is "this didn't crash + ok=true," meaning args
        // decode + JSON parse + dispatch all worked.
    }

    /// End-to-end UDS round trip: bind a real socket, write the
    /// request, read the response. Pins the wire shape against an
    /// actual socket — the runtime side will hit exactly this
    /// listener at production time.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_uds_round_trip_observe_health() {
        let socket_path = std::path::PathBuf::from(format!(
            "/tmp/erld-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).unwrap();
        let proxy = fresh_proxy();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            serve_one(stream, proxy).await.unwrap();
        });

        // Client side: open, send request, read response, close.
        let mut client = UnixStream::connect(&socket_path).await.unwrap();
        let req =
            "{\"tool_name\":\"observe.health\",\"arguments_b64\":\"\"}\n".as_bytes();
        client.write_all(req).await.unwrap();
        client.flush().await.unwrap();
        let (read_half, _) = client.into_split();
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        server.await.unwrap();

        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["content_type"], "application/json");

        let _ = std::fs::remove_file(&socket_path);
    }

    /// `bind_socket` cleans up a stale (no live listener) socket
    /// file. Pins the recovery path so a daemon that crashed
    /// without removing the file can re-bind on next boot.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bind_recovers_from_stale_socket_file() {
        let socket_path = std::path::PathBuf::from(format!(
            "/tmp/erld-stale-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        // Touch a stale file with no listener.
        std::fs::write(&socket_path, b"").unwrap();
        let listener = bind_socket(&socket_path).await.unwrap();
        drop(listener);
        let _ = std::fs::remove_file(&socket_path);
    }

    /// `bind_socket` aborts when a live listener is already on the
    /// path. Pins the safety property — two daemons on the same
    /// host cannot accidentally split-brain over the dispatch
    /// socket.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bind_refuses_when_live_listener_holds_path() {
        let socket_path = std::path::PathBuf::from(format!(
            "/tmp/erld-live-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let _ = std::fs::remove_file(&socket_path);
        let _holder = UnixListener::bind(&socket_path).unwrap();
        let err = bind_socket(&socket_path).await.unwrap_err();
        assert!(format!("{err:#}").contains("another process already accepts"));
        let _ = std::fs::remove_file(&socket_path);
    }
}

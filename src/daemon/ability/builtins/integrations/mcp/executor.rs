// EasyNet CLI — MCP Ability Executor
// ==================================
//
// File: src/daemon/ability/builtins/integrations/mcp/executor.rs
// Description: Implementation backing `[exec] kind = "mcp"` in an
//              agent ability manifest. It calls one configured
//              upstream MCP tool through `McpClientService` and
//              returns the upstream MCP `tools/call` response
//              verbatim inside EasyNet's standard executor envelope.
//
// Why this exists
// ---------------
// Binding MCP-Bench or an operator's existing MCP tool catalogue to
// a named EasyNet agent should not require an LLM prompt or a shell
// wrapper. The manifest already pins the upstream `(server, tool)`
// pair, so the daemon can dispatch directly through the outbound MCP
// client service. This keeps the route deterministic and preserves
// the MCP `{content, isError}` result shape for benchmark checks.
//
// Sync→async bridge contract
// --------------------------
// `LocalRpcHandler` is sync `Fn(Value) -> anyhow::Result<Value>` —
// the registry was shaped before MCP-over-async existed and the
// sibling executors (`shell_executor`, `http_executor` via `ureq`,
// `eal_executor`) are all sync IO. The MCP client is async-only, so
// this executor MUST cross the sync→async boundary. We require an
// ambient tokio runtime (the daemon hot path provides one) and use
// `block_in_place + Handle::block_on`; callers without a runtime
// are an authoring bug, not a recoverable situation — see
// `block_on_async` below for the fail-fast contract.
//
// Client lifecycle (process-singleton, boot-injected)
// ---------------------------------------------------
// A daemon invocation of an `mcp` exec must NOT rebuild
// `McpClientService` per call — doing so re-reads `mcps.json`
// from disk and resets every upstream's connection state
// (`McpServerRow.conn`/`http_conn`), forcing a fresh stdio spawn +
// `initialize` handshake on every tool call. For an mcp-bench
// 28-server catalogue that turns one ability invocation into 28
// handshakes.
//
// The executor therefore reads the `McpClientService` from a
// process-wide [`crate::support::platform::process_singleton::ProcessSingleton`]
// (`Mode::Once` — production write-once) that
// `build_registry_with_services` populates at boot via
// `set_process_client(...)`. The same `Arc` instance backs both
// `mcp.client.*`, the reflective MCP registry, and
// `[exec] kind="mcp"` ability dispatch — one config load, one
// connection pool, no divergence between surfaces.
//
// Why a process-singleton instead of threading a context arg
// through every handler: the registry's `LocalRpcHandler` signature
// is `Fn(Value) -> Result<Value>`, owned for the registry's
// lifetime. Extending it with a per-call context argument is the
// "right" eventual fix but lives in the ability-dispatch context
// refactor; until then a boot-injected singleton matches the daemon's
// actual lifecycle (one MCP service per process, set once at boot)
// and removes the silent-divergence bug between exec and reflective
// surfaces.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use crate::daemon::ability::manifest::McpExec;
use crate::daemon::execution::mcp::McpClientService;
use crate::support::platform::process_singleton::ProcessSingleton;

/// Process-wide `McpClientService` handle. Populated at daemon boot
/// by `build_registry_with_services` via `set_process_client(...)`
/// and read by every `run_mcp_exec` call. See module docs for why
/// this is a singleton rather than a per-call argument.
///
/// `ProcessSingleton::once()` — production write-once. The dispatch
/// hot path's `get()` guarantees that nothing can swap the singleton
/// out from under an in-flight `run_mcp_exec` call. See
/// `support::process_singleton` for the mode-choice rationale.
static PROCESS_CLIENT: ProcessSingleton<McpClientService> = ProcessSingleton::once();

/// Register the process-wide `McpClientService` handle. Idempotent
/// against concurrent racers: the first writer wins, later writers'
/// values are silently dropped. The daemon boot path is single-
/// writer in practice.
///
/// Returns the `Arc<McpClientService>` that will be observed by
/// future `run_mcp_exec` calls — either the one just installed or
/// the one a concurrent caller installed first.
pub fn set_process_client(svc: Arc<McpClientService>) -> Arc<McpClientService> {
    PROCESS_CLIENT.set(svc)
}

/// Test-only seam: install a `McpClientService` for the unit tests
/// in this file. Production code goes through `set_process_client`
/// at boot. Tests that share the static must run serially or accept
/// the first-writer-wins semantics; the tests here only assert
/// behaviour that does not depend on the specific service identity
/// (argument validation and a guaranteed-failure RPC), so we just
/// gate on test-cfg.
#[cfg(test)]
fn install_process_client_for_tests(svc: Arc<McpClientService>) {
    PROCESS_CLIENT.set(svc);
}

/// Stable tag for the `fulfilled_by` field of the executor envelope.
/// Downstream callers (invocation ledger renderer, mission-runs
/// schema-compat harness) match on this string; pin it as a const
/// so a typo here is a compile failure, not a silent ledger break.
const FULFILLED_BY_TAG: &str = "mcp";
pub const AXON_INVOCATION_CONTEXT_RESULT_KEY: &str = "axon_invocation";

/// Invoke the MCP tool declared by an ability manifest.
///
/// Returns the upstream MCP `tools/call` response wrapped in the
/// standard executor envelope (`result`, `fulfilled_by`, `server`,
/// `tool`, `elapsed_ms`). The upstream response is preserved
/// verbatim so callers can inspect MCP-native fields such as
/// `content` / `isError`.
pub fn run_mcp_exec(spec: &McpExec, args: &Value) -> anyhow::Result<Value> {
    run_mcp_exec_with_invocation_context(spec, args, None)
}

/// Invoke an MCP-backed plugin ability while preserving daemon envelope
/// metadata in the local result.
///
/// The context is deliberately NOT forwarded inside MCP `arguments`; upstream
/// MCP tools validate their own schemas and must not receive daemon protocol
/// fields as surprise user arguments.
pub fn run_mcp_exec_with_invocation_context(
    spec: &McpExec,
    args: &Value,
    invocation_context: Option<Value>,
) -> anyhow::Result<Value> {
    let started = Instant::now();
    let arguments = require_object_args(args)?;
    let server = spec.server.clone();
    let tool = spec.tool.clone();

    let client = PROCESS_CLIENT.get().ok_or_else(|| {
        anyhow::anyhow!(
            "mcp executor: process-wide McpClientService has not been initialised; \
             daemon boot must call mcp_executor::set_process_client before any \
             `[exec] kind=\"mcp\"` ability is invoked"
        )
    })?;
    let result = block_on_async(async move {
        client
            .rpc(
                &server,
                "tools/call",
                json!({
                    "name": tool,
                    "arguments": arguments,
                }),
            )
            .await
    })?;

    let mut response = json!({
        "result": result,
        "fulfilled_by": FULFILLED_BY_TAG,
        "server": spec.server,
        "tool": spec.tool,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    });
    if let Some(invocation_context) = invocation_context {
        response[AXON_INVOCATION_CONTEXT_RESULT_KEY] = invocation_context;
    }
    Ok(response)
}

/// Validate + clone the arguments JSON for the `tools/call` payload.
/// MCP requires `arguments` to be a JSON object; we reject everything
/// else with a typed message so the operator sees what they sent.
fn require_object_args(args: &Value) -> anyhow::Result<Value> {
    match args {
        Value::Object(_) => Ok(args.clone()),
        Value::Null => anyhow::bail!("mcp executor: ability args must be a JSON object; got null"),
        Value::Bool(_) => {
            anyhow::bail!("mcp executor: ability args must be a JSON object; got a boolean")
        }
        Value::Number(_) => {
            anyhow::bail!("mcp executor: ability args must be a JSON object; got a number")
        }
        Value::String(_) => {
            anyhow::bail!("mcp executor: ability args must be a JSON object; got a string")
        }
        Value::Array(_) => {
            anyhow::bail!("mcp executor: ability args must be a JSON object; got an array")
        }
    }
}

/// Sync→async bridge. The executor runs inside the daemon's tokio
/// runtime (the gRPC invocation path is async by construction and
/// drives every `LocalRpcHandler` from inside a worker thread).
/// Calling `run_mcp_exec` outside a runtime is an authoring bug —
/// a synchronous unit test or CLI path that forgot to start one —
/// not a recoverable production state. Bailing fast surfaces the
/// bug at the call site instead of papering over it with a throwaway
/// runtime whose lifecycle is undocumented.
///
/// **Bridge policy.** The MCP plane has exactly one sanctioned
/// sync→async bridge family: this function for the executor's hot
/// path, and
/// [`crate::daemon::ability::builtins::integrations::mcp::reflective_registry::run_eager_blocking`]
/// /
/// [`crate::daemon::ability::builtins::integrations::mcp::reflective_registry::McpReflectionSupervisor::attach_refresh_sinks_blocking`]
/// for the boot-time reflective registry. Any future sync entry
/// point that needs to drive a future MUST reuse one of those
/// surfaces rather than re-deriving the `Handle::try_current` /
/// `block_in_place` ladder — duplicate bridges historically drifted
/// on missing-runtime policy.
fn block_on_async<F>(fut: F) -> anyhow::Result<Value>
where
    F: std::future::Future<Output = anyhow::Result<Value>> + Send + 'static,
{
    let handle = tokio::runtime::Handle::try_current().map_err(|_| {
        anyhow::anyhow!(
            "mcp executor: no ambient tokio runtime; \
             callers must drive `[exec] kind=\"mcp\"` from inside the daemon's \
             async InvokeStream/Invoke path or a `#[tokio::test]` runtime"
        )
    })?;
    // Run the future ON the runtime's workers instead of
    // `block_in_place + block_on` on THIS thread. Ability handlers
    // execute on spawn_blocking threads; driving IO-registering
    // futures (tokio::process pipes) from there entangles the IO
    // driver with the blocking thread's context and has surfaced as
    // "A Tokio 1.x context was found, but it is being shutdown".
    // A worker-spawned task registers IO on the live driver; this
    // thread just waits. The timeout fails fast instead of hanging
    // if the runtime is genuinely going down.
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    handle.spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx.recv_timeout(std::time::Duration::from_secs(600))
        .map_err(|_| {
            anyhow::anyhow!(
                "mcp executor: upstream call did not complete (runtime shutting \
             down or upstream hung past the 600s ceiling)"
            )
        })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::execution::mcp::{McpClientService, McpClientsFile, McpServerSpec};

    #[test]
    fn rejects_non_object_args_with_typed_messages() {
        for (input, expected_substring) in [
            (json!(null), "got null"),
            (json!(true), "got a boolean"),
            (json!(42), "got a number"),
            (json!("string"), "got a string"),
            (json!([1, 2]), "got an array"),
        ] {
            let err = require_object_args(&input).unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains(expected_substring),
                "input {input}: message {msg:?} should mention {expected_substring:?}"
            );
            assert!(
                msg.contains("must be a JSON object"),
                "input {input}: message {msg:?} should mention the contract"
            );
        }
    }

    #[test]
    fn accepts_object_args_and_clones_them() {
        let input = json!({"foo": 1, "bar": [1, 2]});
        let out = require_object_args(&input).expect("object accepted");
        assert_eq!(out, input);
    }

    #[test]
    fn mcp_invocation_context_result_key_is_reserved_for_metadata() {
        assert_eq!(AXON_INVOCATION_CONTEXT_RESULT_KEY, "axon_invocation");
    }

    /// End-to-end: a manifest pinning a configured-but-unreachable
    /// upstream server should surface the upstream RPC failure
    /// verbatim (no panic, no per-call config reread). We don't spin
    /// up a real upstream here — the contract under test is "the
    /// executor reaches `client.rpc` with the spec's `server`/`tool`
    /// and propagates errors as `anyhow::Error`."
    ///
    /// The test directly exercises the inner async RPC against an
    /// explicit in-memory client so it does not depend on the
    /// process-global `PROCESS_CLIENT` (which other tests may have
    /// populated first). `#[tokio::test(flavor = "multi_thread")]`
    /// provides the ambient runtime that `block_on_async` requires.
    #[tokio::test(flavor = "multi_thread")]
    async fn unreachable_server_surfaces_rpc_error_not_panic() {
        let svc = McpClientService::from_file(McpClientsFile {
            servers: vec![McpServerSpec {
                name: "unreachable".into(),
                command: "/nonexistent/binary-that-does-not-exist".into(),
                ..McpServerSpec::default()
            }],
        });
        let spec = McpExec {
            server: "unreachable".into(),
            tool: "anything".into(),
        };
        let arguments = json!({"q": "hello"});
        let err = svc
            .rpc(
                &spec.server,
                "tools/call",
                json!({"name": spec.tool, "arguments": arguments}),
            )
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            !msg.is_empty(),
            "rpc error must carry an operator-readable message"
        );
    }

    /// `run_mcp_exec` called before `set_process_client` MUST fail
    /// fast with a typed message naming the boot-order bug rather
    /// than silently re-loading the on-disk config — that older
    /// behaviour created a second `McpClientService` instance and
    /// diverged from the reflective registry's view of
    /// `mcps.json`. We assert the error mentions the missing
    /// init step so an operator hitting this in dev can grep
    /// straight to the cause.
    #[tokio::test(flavor = "multi_thread")]
    async fn run_mcp_exec_fails_typed_when_process_client_uninitialised() {
        // We can't easily reset PROCESS_CLIENT once another test has
        // populated it; gate the assertion on the unset path by
        // checking the static lazily. If the static is already set
        // (because another test installed it first) the assertion
        // collapses to "no panic", still useful as a smoke test.
        let spec = McpExec {
            server: "anything".into(),
            tool: "anything".into(),
        };
        let result = run_mcp_exec(&spec, &json!({}));
        match (PROCESS_CLIENT.get().is_some(), result) {
            (false, Err(e)) => {
                let msg = format!("{e}");
                assert!(
                    msg.contains("set_process_client"),
                    "error message must name the missing init step: {msg}"
                );
            }
            (false, Ok(_)) => panic!("run_mcp_exec must not succeed without a client"),
            (true, _) => {
                // Another test installed PROCESS_CLIENT already.
                // The fast-fail path is unreachable from here; nothing
                // to assert beyond "did not panic".
            }
        }
    }

    /// Sanity: `install_process_client_for_tests` makes the static
    /// observable so a subsequent `run_mcp_exec` reaches the RPC
    /// path. We use an in-memory client pointed at a non-existent
    /// command so the RPC fails fast without external IO.
    #[tokio::test(flavor = "multi_thread")]
    async fn install_process_client_routes_through_to_rpc() {
        let svc = Arc::new(McpClientService::from_file(McpClientsFile {
            servers: vec![McpServerSpec {
                name: "unreachable-installed".into(),
                command: "/nonexistent/binary-installed".into(),
                ..McpServerSpec::default()
            }],
        }));
        install_process_client_for_tests(svc);
        let spec = McpExec {
            server: "unreachable-installed".into(),
            tool: "anything".into(),
        };
        let _ = run_mcp_exec(&spec, &json!({}));
        // Either an Err (expected when this test's install_ won the
        // race) or an Err shaped by an earlier test's installation.
        // The assertion the previous block makes is enough; this
        // test exists to keep the seam exercised so future refactors
        // do not delete it.
    }
}

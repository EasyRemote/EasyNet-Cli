// EasyNet CLI
// ===========
//
// File: src/facade/cli/invoke.rs
// Description: `easynet ability invoke <ability> [--args JSON] [--timeout SECS]`.
//
// Routing model after the AXON-RFC-001 P1.5 federation cull:
//
//   easynet ability invoke <ability>            # Local IPC dispatch.
//                                               # Goes to the local daemon's
//                                               # Control plane (unix socket
//                                               # at ~/.easynet/control.sock)
//                                               # and lands in the same
//                                               # AbilityDispatcher every
//                                               # other invocation surface
//                                               # uses (EAL agent.ability,
//                                               # MCP tools/call, library
//                                               # FFI). One source of truth.
//
//   easynet ability invoke <ability> --node N   # ⚠ Pinning to a remote
//                                               # node is not supported in
//                                               # this build. The federation
//                                               # bridge that backed the
//                                               # `--node` flag was removed
//                                               # by AXON-RFC-001 P1.5; the
//                                               # replacement (Invoke
//                                               # against an Agent ability
//                                               # exposed on the realm)
//                                               # ships in a follow-up. For
//                                               # now, --node returns a
//                                               # precise error so a script
//                                               # using the old form fails
//                                               # loud rather than silently
//                                               # auto-routing locally.
//
// Why this rewrite
// ----------------
// Pre-rewrite this file called
// `bridge.call_mcp_tool_with_timeout(...)`, which AXON-RFC-001 P1.5
// removed in the federation cull. Every `easynet ability invoke <name>`
// call therefore failed with
//
//     bridge: call_mcp_tool_with_timeout removed by AXON-RFC-001 P1.5;
//     use Invoke against the appropriate Agent ability
//
// regardless of which ability the caller named. The CLI sub-command
// was effectively dead. The fix is to route through the Control
// plane the daemon already runs on a local UDS — the same dispatcher
// (`AbilityDispatcher::execute_rpc`) that backs every other
// invocation surface in the codebase. One dispatcher, all surfaces;
// no "federation bridge" path that exists in name only.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::{bail, Context};
use clap::Args;
use serde_json::Value;

use crate::support::local_invoke::invoke_local_ability;
use crate::support::{output, timeouts};

#[derive(Debug, Args)]
pub struct InvokeArgs {
    /// Ability (tool) name to invoke. Use the canonical
    /// `<owner>.<verb>` form for agent-owned abilities (e.g.
    /// `claude.weather`) and the bare verb for system abilities
    /// (e.g. `easynet.discover`, `observe.health`).
    pub ability: String,
    /// ⚠ Pinning to a remote node id is not wired in this build.
    /// The federation Invoke surface that would back it ships in a
    /// follow-up to AXON-RFC-001 P1.5. Passing `--node` today
    /// returns a precise error rather than silently auto-routing
    /// locally.
    #[arg(long, short = 'n', value_name = "NODE_ID")]
    pub node: Option<String>,
    /// JSON object passed to the ability as its arguments (e.g.
    /// `--args '{"location": "Beijing"}'`). Defaults to `{}` when
    /// omitted.
    #[arg(long, value_name = "JSON")]
    pub args: Option<String>,
    /// Per-call deadline in seconds. `0` inherits the runtime default.
    /// Default: 60 s, governed by `support::timeouts::INVOKE_DEFAULT_SECS`.
    #[arg(long, value_name = "SECS", default_value_t = timeouts::INVOKE_DEFAULT_SECS)]
    pub timeout: u64,
    /// Print the raw ability envelope instead of just the inner
    /// payload. The default (no flag) follows the same pattern as
    /// `jq -r .result`: when the response is the standard
    /// `{result, fulfilled_by, ...}` envelope (chat handler, shell
    /// exec, registry dispatch), unwrap to `result`; otherwise print
    /// the value as-is. Pass `--raw` when a script needs the full
    /// envelope (timing, exit_code, fulfilled_by) for diagnostics.
    #[arg(long)]
    pub raw: bool,
}

pub fn run(invoke_args: InvokeArgs) -> anyhow::Result<()> {
    // --node is reserved-but-not-implemented after the AXON-RFC-001
    // P1.5 cull. Reject it loudly with the same message everywhere
    // so a script that depended on the old federation path sees a
    // single, searchable error string. Treat empty `--node ""` as
    // a shell-expansion accident, same as before.
    match invoke_args.node.as_deref().map(str::trim) {
        None => {} // local dispatch — the only supported path today
        Some("") => bail!(
            "--node was given but empty; omit the flag to dispatch locally, \
             or pass a real node id once federation Invoke is wired"
        ),
        Some(_) => bail!(
            "remote pinning via --node is not wired in this build. The \
             federation bridge that backed it was removed by AXON-RFC-001 \
             P1.5; the replacement (Invoke against an Agent ability on the \
             realm) ships in a follow-up. Until then, omit --node to \
             dispatch against the local daemon."
        ),
    }

    let arguments: Value = match invoke_args.args.as_deref() {
        Some(s) => serde_json::from_str(s).context("parse --args JSON")?,
        None => Value::Object(Default::default()),
    };

    // Validate-and-clamp the timeout the same way every other CLI
    // invoke surface does, so the operator-visible behaviour around
    // `--timeout 0` (inherit default) and "value too large" matches
    // what `easynet mission run` / `agent send` already enforce.
    let _timeout_ms = timeouts::effective_ms(invoke_args.timeout).map_err(anyhow::Error::msg)?;

    // One ability invocation. The shared helper owns the
    // control.json lookup, the IPC dance, and the daemon-error
    // rendering — every CLI subcommand goes through this same
    // function per the AXON-RFC-001 ontology that says "every
    // action is an ability invocation".
    let result = invoke_local_ability(&invoke_args.ability, arguments)?;

    let to_print = if invoke_args.raw {
        result
    } else {
        unwrap_envelope(result)
    };
    // Strings get printed bare so a shell pipeline can `read` the
    // value without de-quoting. Other shapes (objects, arrays, nums)
    // still go through pretty-print so the operator can see structure
    // — script users that need an exact JSON form pass `--raw` and
    // jq the result themselves.
    match &to_print {
        Value::String(s) => println!("{s}"),
        _ => println!("{}", serde_json::to_string_pretty(&to_print)?),
    }
    output::success(&format!("{} → ok (local daemon)", invoke_args.ability));
    Ok(())
}

/// Strip one layer of the standard ability envelope when present.
///
/// The dispatch surfaces (chat handler, shell executor, invoke
/// handler) all wrap the actual payload in
/// `{result, fulfilled_by, ...}` so a caller can see whether the
/// call ran through a deterministic executor or an LLM. For the
/// CLI's default print path that's noise — a script piping the
/// output to `jq` invariably reaches for `.result`. We do that
/// extraction here so the common case is "print the value", not
/// "print a JSON object the user has to navigate".
///
/// The check is deliberately structural: we only unwrap when the
/// top-level is an object that has a `result` field AND a
/// `fulfilled_by` field — both halves of the envelope contract.
/// Any other shape passes through verbatim, which means an ability
/// returning a hand-crafted `{"result": ...}` (no fulfilled_by) is
/// not accidentally unwrapped.
fn unwrap_envelope(v: Value) -> Value {
    match v {
        Value::Object(mut map)
            if map.contains_key("result") && map.contains_key("fulfilled_by") =>
        {
            map.remove("result").unwrap_or(Value::Null)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_node_returns_a_precise_actionable_error() {
        // `--node` is reserved-but-not-implemented post P1.5. The
        // CLI MUST refuse rather than silently auto-routing — a
        // script that previously pinned to a real federation node
        // and now gets local-only dispatch behind its back would
        // produce wrong results without warning.
        let res = run(InvokeArgs {
            ability: "observe.health".into(),
            node: Some("some-node-id".into()),
            args: None,
            timeout: 60,
            raw: false,
        });
        let err = res.expect_err("must reject --node");
        let msg = format!("{err}");
        assert!(
            msg.contains("federation") || msg.contains("--node"),
            "error must mention --node / federation status, got: {msg}"
        );
    }

    #[test]
    fn empty_node_string_is_caught_as_shell_expansion_accident() {
        // `--node ""` is almost always an unset shell variable that
        // expanded to empty, not a deliberate intent. Reject loudly.
        let res = run(InvokeArgs {
            ability: "observe.health".into(),
            node: Some("   ".into()),
            args: None,
            timeout: 60,
            raw: false,
        });
        let err = res.expect_err("must reject empty --node");
        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn malformed_args_json_surfaces_a_parse_error_with_context() {
        // Operator-visible: a typo in --args should say "parse
        // --args JSON", not crash mid-IPC.
        let res = run(InvokeArgs {
            ability: "observe.health".into(),
            node: None,
            args: Some("{not valid".into()),
            timeout: 60,
            raw: false,
        });
        let err = res.expect_err("must reject malformed JSON");
        assert!(format!("{err:#}").contains("parse --args JSON"));
    }

    #[test]
    fn unwrap_envelope_strips_standard_shape() {
        // The shell-executor / invoke-handler envelope shape:
        // {result, fulfilled_by, ...}. Default print path drops one
        // layer so script users don't have to `jq .result` every
        // time they pipe through the CLI.
        let envelope = serde_json::json!({
            "result": "tokyo: Clear +20C",
            "fulfilled_by": "shell",
            "exit_code": 0,
            "elapsed_ms": 700,
        });
        let unwrapped = unwrap_envelope(envelope);
        assert_eq!(unwrapped, serde_json::Value::String("tokyo: Clear +20C".into()));
    }

    #[test]
    fn unwrap_envelope_passes_non_envelope_through() {
        // An ability returning `{"result": ...}` without the
        // `fulfilled_by` half is NOT a dispatch envelope — could be
        // a hand-crafted ability that uses `result` as a domain key.
        // Don't unwrap; the structural check guards against that.
        let plain = serde_json::json!({"result": 42});
        assert_eq!(unwrap_envelope(plain.clone()), plain);
    }

    #[test]
    fn unwrap_envelope_passes_arrays_and_scalars_through() {
        // Anything that isn't an object can't carry an envelope,
        // so the unwrap is a no-op. Lock the behaviour so a future
        // refactor doesn't accidentally start unpacking arrays.
        for v in [
            serde_json::Value::Null,
            serde_json::json!("a string"),
            serde_json::json!(42),
            serde_json::json!([1, 2, 3]),
        ] {
            assert_eq!(unwrap_envelope(v.clone()), v);
        }
    }
}

// EasyNet CLI
// ===========
//
// File: src/cli/commands/invoke.rs
// Description: `easynet ability invoke <ability-ura> [--args JSON] [--timeout SECS]`.
//
// Routing model after the AXON-RFC-001 P1.5 federation cull:
//
//   easynet ability invoke <ability-ura>        # Local Axon dispatch.
//                                               # Derives the daemon
//                                               # registry key from the
//                                               # canonical Ability URA.
//
//   easynet ability invoke <ability-ura> --node N
//                                               # Remote dispatch. The
//                                               # ability argument is already
//                                               # a canonical Ability URA; the
//                                               # CLI does not infer owner or
//                                               # mint URAs from bare names.
//
// Why this rewrite
// ----------------
// Pre-rewrite this file called
// `bridge.call_mcp_tool_with_timeout(...)`, which AXON-RFC-001 P1.5
// removed in the federation cull. Every `easynet ability invoke <selector>`
// call therefore failed with
//
//     bridge: call_mcp_tool_with_timeout removed by AXON-RFC-001 P1.5;
//     use Invoke against the appropriate Agent ability
//
// regardless of which ability the caller named. The CLI sub-command
// was effectively dead. The fix is to route through the daemon's
// Axon Invocation gRPC surface (`~/.easynet/daemon.sock`), where the
// shared `LocalRuntime` owns admission, dispatch, receipts, and ledger
// persistence. One Axon invoke path, all CLI surfaces; no "federation
// bridge" path that exists in name only.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::{bail, Context};
use clap::Args;
use serde_json::Value;

#[cfg(not(feature = "axon-pb"))]
use crate::cli::commands::invocation_tuple::remote_invocation_transport_unsupported;
use crate::cli::commands::invocation_tuple::{
    required_causal_context, required_nonce_hex, required_subject, AbilityInvocationRef,
};
use crate::support::platform::local_invoke::{
    invoke_local_target_explicit_causal_timeout, LocalAbilityTarget,
};
use crate::support::platform::{output, timeouts};

#[derive(Debug, Args)]
pub struct InvokeArgs {
    /// Canonical Ability URA returned by `easynet ability list`, or an
    /// explicit descriptor ref `<ability-ura>@<version>` when the caller wants
    /// remote origin-caller proof generation to bind a known descriptor
    /// version.
    pub ability_ura: String,
    /// Pin the invocation to a remote node: a canonical Device URA
    /// (`easynet:///r/<realm>/device/<node_id>`) or Authority URA
    /// (`easynet:///r/<realm>/authority`). The call routes through the
    /// local daemon's canonical `Invocation::Invoke` RPC — the
    /// cross-device main channel. Builds without canonical remote invocation
    /// transport reject the flag with a re-build hint. Omit to dispatch locally.
    #[arg(long, short = 'n', value_name = "URA")]
    pub node: Option<String>,
    /// JSON object passed to the ability as its arguments — for
    /// example: --args {"location":"Beijing"}. Defaults to {} when
    /// omitted.
    #[arg(long, value_name = "JSON")]
    pub args: Option<String>,
    /// Per-call transport guard in seconds. '0' uses the configured invocation
    /// guard. Default: 1 hour, governed by `support::timeouts::INVOKE_DEFAULT_SECS`.
    #[arg(long, value_name = "SECS", default_value_t = timeouts::INVOKE_DEFAULT_SECS)]
    pub timeout: u64,
    /// Print the raw ability envelope instead of just the inner
    /// payload. The default (no flag) follows the same pattern as
    /// 'jq -r .result': when the response is the standard
    /// '{result, fulfilled_by, ...}' envelope (chat handler, shell
    /// exec, registry dispatch), unwrap to 'result'; otherwise print
    /// the value as-is. Pass '--raw' when a script needs the full
    /// envelope (timing, exit_code, fulfilled_by) for diagnostics.
    #[arg(long)]
    pub raw: bool,
    /// AXIOM envelope subject — the resource the ability acts on,
    /// expressed as a canonical resource URA
    /// ('easynet:///r/<realm>/resource/<id>'). Required by abilities
    /// whose contract pins behaviour on a specific resource (e.g.
    /// 'camera.snapshot', which uses the subject to look up the
    /// camera's 'hardware_id' and 'resources.json' entry); ignored
    /// by abilities that don't consume 'EnvelopeContext.subject'.
    /// Per INV-SUBJECT-ENVELOPE the subject MUST come from the
    /// envelope, not from --args — passing it via --args
    /// {"subject": "..."} is rejected by the handler.
    #[arg(long, value_name = "URA")]
    pub subject: Option<String>,
    /// AXIOM invocation nonce as exactly 16 bytes of lowercase or uppercase
    /// hex. Required for `--node` remote public invocation so the CLI does not
    /// mint caller freshness behind the operator's back.
    #[arg(long, value_name = "32_HEX")]
    pub nonce_hex: Option<String>,
    /// Declare that this public invocation has no causal parent. Required for
    /// root calls. Mutually exclusive with --causal-context-json.
    #[arg(long)]
    pub causal_root: bool,
    /// Explicit non-root causal context JSON. Accepted forms are:
    /// {"form":"scalar","receipt_hash_hex":"<64_HEX>","receipt_ura":"<URA>"},
    /// {"form":"list","prior":[...]}, or
    /// {"form":"merkle","root_hex":"<64_HEX>","proof_ura":"<URA>"}.
    /// Root calls must use --causal-root so root placement has one encoding.
    #[arg(long, value_name = "JSON")]
    pub causal_context_json: Option<String>,
}

pub fn run(invoke_args: InvokeArgs) -> anyhow::Result<()> {
    let ability_ref = AbilityInvocationRef::parse(&invoke_args.ability_ura)?;
    let ability_selector = ability_ref.selector();

    // `--node` is wired against the local daemon's canonical
    // `Invocation::Invoke` RPC via the descriptor-bound remote invoke helper.
    // Builds without `axon-pb` fail with the same canonical unsupported
    // transport error used by stream and bidi ingress.
    let node_ura: Option<String> = match invoke_args.node.as_deref().map(str::trim) {
        None => None,
        Some("") => bail!(
            "--node was given but empty; omit the flag to dispatch locally, \
             or pass a real `easynet:///r/<tenant>/device/<node>` URA"
        ),
        Some(node) => {
            #[cfg(feature = "axon-pb")]
            {
                Some(crate::daemon::invocation::routing::remote_invoke::parse_node_ura(node)?)
            }
            #[cfg(not(feature = "axon-pb"))]
            {
                let _ = node;
                return Err(remote_invocation_transport_unsupported(
                    "remote ability invoke with --node",
                ));
            }
        }
    };

    let arguments: Value = match invoke_args.args.as_deref() {
        Some(s) => serde_json::from_str(s).context("parse --args JSON")?,
        None => Value::Object(Default::default()),
    };

    let timeout =
        timeouts::invocation_transport_guard(invoke_args.timeout).map_err(anyhow::Error::msg)?;

    // Cross-hub dispatch when `--node` is set; local dispatch
    // otherwise. Both paths surface the same unwrap-or-raw result
    // shape so a script piping to `jq` doesn't have to branch.
    let (result, fulfilled_label) = match node_ura.as_deref() {
        #[cfg(feature = "axon-pb")]
        Some(target) => {
            let credentials = crate::daemon::persistence::config::load_credentials()
                .context("remote ability invoke requires paired device credentials")?;
            let caller_ura =
                crate::support::platform::remote_device::caller_device_ura(&credentials)?;
            let target_call = ability_ref
                .remote_target_for_mode(target, crate::daemon::ability::CallMode::Rpc)?;
            let surface = "remote ability invoke with --node";
            let subject = required_subject(invoke_args.subject.as_deref(), surface)?.to_string();
            let invocation_nonce = required_nonce_hex(invoke_args.nonce_hex.as_deref(), surface)?;
            let causal_context = required_causal_context(
                invoke_args.causal_root,
                invoke_args.causal_context_json.as_deref(),
                surface,
            )?;
            let request = crate::daemon::invocation::routing::remote_invoke::RemoteInvocationTuplePlan::public_explicit(
                &target_call,
                caller_ura,
                subject,
                invocation_nonce,
                causal_context,
                arguments,
                timeout,
            )?
            .into_request()?;
            let value =
                crate::daemon::invocation::routing::remote_invoke::invoke_remote_target(request)?;
            (value, format!("canonical Invoke target={target}"))
        }
        // The `not(axon-pb)` arm of `--node` already returned above;
        // this match is reachable only via `node_ura == None`.
        #[cfg(not(feature = "axon-pb"))]
        Some(_) => unreachable!("--node unsupported return handled before dispatch"),
        None => {
            if ability_ref.is_descriptor_ref() {
                bail!(
                    "local ability invoke does not accept descriptor refs; omit `@version` \
                     for local dispatch or pass `--node` for remote descriptor-bound origin proof"
                );
            }
            // One ability invocation. The shared helper owns the
            // daemon.sock Axon Invoke, subject threading, and
            // daemon-error rendering — every CLI subcommand goes
            // through this same function per the AXON-RFC-001
            // ontology that says "every action is an ability
            // invocation".
            let surface = "local ability invoke";
            let subject = required_subject(invoke_args.subject.as_deref(), surface)?;
            let invocation_nonce = required_nonce_hex(invoke_args.nonce_hex.as_deref(), surface)?;
            let causal_context = required_causal_context(
                invoke_args.causal_root,
                invoke_args.causal_context_json.as_deref(),
                surface,
            )?;
            let dispatch_name = ability_selector.local_registry_ability();
            let target = LocalAbilityTarget::from_selector(ability_selector);
            debug_assert_eq!(target.dispatch_name(), dispatch_name);
            let value = invoke_local_target_explicit_causal_timeout(
                &target,
                arguments,
                subject,
                invocation_nonce,
                causal_context,
                timeout,
            )?;
            (value, "local daemon".to_string())
        }
    };

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
    output::success(&format!(
        "{} → ok ({fulfilled_label})",
        ability_selector.ability_ura()
    ));
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
    use crate::cli::commands::invocation_tuple::parse_invocation_nonce_hex;

    #[test]
    fn invoke_ability_ref_parses_plain_ability_ura() {
        let parsed =
            AbilityInvocationRef::parse("easynet:///r/acme/ability/device.node.observe.health")
                .expect("plain ability URA");

        assert_eq!(
            parsed.selector().ability_ura(),
            "easynet:///r/acme/ability/device.node.observe.health"
        );
        assert!(!parsed.is_descriptor_ref());
    }

    #[test]
    fn invoke_ability_ref_preserves_explicit_descriptor_ref() {
        let descriptor_ref =
            "easynet:///r/acme/ability/device.node.observe.health@2.1.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read";
        let parsed = AbilityInvocationRef::parse(descriptor_ref).expect("descriptor ref");

        assert_eq!(
            parsed.selector().ability_ura(),
            "easynet:///r/acme/ability/device.node.observe.health"
        );
        assert_eq!(parsed.descriptor_ref(), Some(descriptor_ref));
    }

    #[test]
    fn non_canonical_node_ura_returns_actionable_error() {
        // PR-N1 commit 8/N: `--node` now accepts the cross-hub URA
        // shape `easynet:///r/<tenant>/device/<node>`. A non-
        // canonical input (bare hostname, https URL, etc.) is
        // rejected with a typed error before any IPC, so a typo
        // never accidentally hits the wire.
        let res = run(InvokeArgs {
            ability_ura: "easynet:///r/acme/ability/device.local.observe.health".into(),
            node: Some("some-node-id".into()),
            args: None,
            timeout: 60,
            raw: false,
            subject: None,
            nonce_hex: None,
            causal_root: false,
            causal_context_json: None,
        });
        let err = res.expect_err("must reject non-canonical --node");
        let msg = format!("{err}");
        // axon-pb on: parse_node_ura error mentions canonical URA
        // shape. axon-pb off: the canonical unsupported transport error
        // mentions `--node`. Both are operator-actionable and neither
        // preserves the retired not-wired path.
        assert!(
            (msg.contains("--node") && msg.contains("unsupported"))
                || msg.contains("canonical Axon Device or Authority URA"),
            "error must surface a canonical --node error, got: {msg}"
        );
        assert!(
            !msg.contains("not wired") && !msg.contains("legacy"),
            "error must not preserve retired invoke wording: {msg}"
        );
    }

    #[test]
    fn empty_node_string_is_caught_as_shell_expansion_accident() {
        // `--node ""` is almost always an unset shell variable that
        // expanded to empty, not a deliberate intent. Reject loudly.
        let res = run(InvokeArgs {
            ability_ura: "easynet:///r/acme/ability/device.local.observe.health".into(),
            node: Some("   ".into()),
            args: None,
            timeout: 60,
            raw: false,
            subject: None,
            nonce_hex: None,
            causal_root: false,
            causal_context_json: None,
        });
        let err = res.expect_err("must reject empty --node");
        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn malformed_args_json_surfaces_a_parse_error_with_context() {
        // Operator-visible: a typo in --args should say "parse
        // --args JSON", not crash mid-IPC.
        let res = run(InvokeArgs {
            ability_ura: "easynet:///r/acme/ability/device.local.observe.health".into(),
            node: None,
            args: Some("{not valid".into()),
            timeout: 60,
            raw: false,
            subject: None,
            nonce_hex: None,
            causal_root: false,
            causal_context_json: None,
        });
        let err = res.expect_err("must reject malformed JSON");
        assert!(format!("{err:#}").contains("parse --args JSON"));
    }

    #[test]
    #[cfg(feature = "axon-pb")]
    fn parse_invocation_nonce_hex_requires_exact_nonzero_nonce() {
        assert_eq!(
            parse_invocation_nonce_hex("01010101010101010101010101010101").unwrap(),
            [1u8; 16]
        );

        let short = parse_invocation_nonce_hex("0102").expect_err("short nonce must fail");
        assert!(format!("{short}").contains("exactly 16 bytes"));

        let zero = parse_invocation_nonce_hex("00000000000000000000000000000000")
            .expect_err("zero nonce must fail");
        assert!(format!("{zero}").contains("all-zero"));
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
        assert_eq!(
            unwrapped,
            serde_json::Value::String("tokyo: Clear +20C".into())
        );
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

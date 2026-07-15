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

use crate::core::ura::AbilitySelector;
use crate::support::platform::local_invoke::{
    invoke_local_ability_target_with_subject_timeout, LocalAbilityTarget,
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
    /// (`easynet:///r/<realm>/device/<node_id>`) or Hub URA
    /// (`easynet:///r/<realm>/hub`). The call routes through the
    /// local daemon's canonical `Invocation::Invoke` RPC — the
    /// cross-device main channel. Requires the `axon-pb` feature
    /// (production builds always enable it); minimal builds reject
    /// the flag with a re-build hint. Omit to dispatch locally.
    #[arg(long, short = 'n', value_name = "URA")]
    pub node: Option<String>,
    /// JSON object passed to the ability as its arguments — for
    /// example: --args {"location":"Beijing"}. Defaults to {} when
    /// omitted.
    #[arg(long, value_name = "JSON")]
    pub args: Option<String>,
    /// Per-call deadline in seconds. '0' inherits the runtime default.
    /// Default: 60 s, governed by 'support::timeouts::INVOKE_DEFAULT_SECS'.
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
}

pub fn run(invoke_args: InvokeArgs) -> anyhow::Result<()> {
    let ability_ref = InvokeAbilityRef::parse(&invoke_args.ability_ura)?;
    let ability_selector = ability_ref.selector();

    // PR-N1 commit 8/N: `--node` is now wired against the local
    // daemon's canonical `Invocation::Invoke` RPC via the
    // `daemon::invocation::routing::remote_invoke` helper. The path requires the
    // `axon-pb` feature (production builds) — minimal builds still
    // bail with the legacy message.
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
                bail!(
                    "remote pinning via --node requires the `axon-pb` feature, \
                     which is not enabled in this build. Re-build with \
                     `--features axon-pb` (production builds always do) \
                     and try again."
                )
            }
        }
    };

    let arguments: Value = match invoke_args.args.as_deref() {
        Some(s) => serde_json::from_str(s).context("parse --args JSON")?,
        None => Value::Object(Default::default()),
    };

    // Validate-and-clamp the timeout the same way every other CLI
    // invoke surface does, so the operator-visible behaviour around
    // `--timeout 0` (inherit default) and "value too large" matches
    // what `easynet mission run` / `agent send` already enforce.
    let timeout_ms = timeouts::effective_ms(invoke_args.timeout)
        .map_err(anyhow::Error::msg)?
        .unwrap_or(timeouts::INVOKE_DEFAULT_SECS * 1000);
    let timeout = std::time::Duration::from_millis(timeout_ms);

    // Cross-hub dispatch when `--node` is set; local dispatch
    // otherwise. Both paths surface the same unwrap-or-raw result
    // shape so a script piping to `jq` doesn't have to branch.
    let (result, fulfilled_label) = match node_ura.as_deref() {
        #[cfg(feature = "axon-pb")]
        Some(target) => {
            // Resolve a real caller URA from credentials.json when
            // available — the CLI's hardcoded fallback
            // `easynet:///r/cli/device/local` is rejected by the
            // local daemon's admission gate the moment the device
            // it runs against is paired (the daemon's realm-trust
            // anchor knows about its own device URA but not about
            // the generic CLI placeholder). Pass-through to
            // The forward target call signs with the daemon's device URA when
            // credentials are present. None keeps fixture scripts working when
            // they run without credentials.json.
            // URA v4.1.4 Phase 2F: caller URA for an `easynet
            // ability invoke --node ...` originating from a daemon
            // is the daemon's *device* URA, not an agent URA. The
            // legacy `/agent/<node>` shape collapsed devices into
            // the agent namespace; v4.1.4 puts the daemon under the
            // `device` role with the same node-id tail.
            let credentials = crate::daemon::persistence::config::load_credentials().ok();
            let caller_ura = credentials
                .as_ref()
                .filter(|c| !c.realm.trim().is_empty() && !c.node_id.trim().is_empty())
                .map(|c| crate::core::ura::device_ura(c.realm.trim(), c.node_id.trim()));
            let target_call = ability_ref.remote_target(target)?;
            let subject = invoke_args
                .subject
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    credentials
                        .as_ref()
                        .and_then(|creds| default_owner_invoke_subject(creds, ability_selector))
                });
            let value =
                crate::daemon::invocation::routing::remote_invoke::invoke_remote_target_with_timeout(
                    &target_call,
                    subject.as_deref(),
                    arguments,
                    caller_ura.as_deref(),
                    // Originating CLI invoke: no inbound causal parent to chain.
                    &[],
                    timeout,
                )?;
            (value, format!("canonical Invoke target={target}"))
        }
        // The `not(axon-pb)` arm of `--node` already bailed above;
        // this match is reachable only via `node_ura == None`.
        #[cfg(not(feature = "axon-pb"))]
        Some(_) => unreachable!("--node bail handled above when axon-pb is off"),
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
            let subject = invoke_args
                .subject
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let dispatch_name = ability_selector.local_registry_ability();
            let target = LocalAbilityTarget::from_selector(ability_selector);
            debug_assert_eq!(target.dispatch_name(), dispatch_name);
            let value = invoke_local_ability_target_with_subject_timeout(
                &target, arguments, subject, timeout,
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

#[cfg(feature = "axon-pb")]
fn default_owner_invoke_subject(
    credentials: &crate::daemon::persistence::config::Credentials,
    ability_selector: &AbilitySelector,
) -> Option<String> {
    let user_id = credentials.user_id().ok()?;
    Some(crate::core::ura::resource_dot_ura(
        credentials.realm_str().trim(),
        &format!("user.{user_id}"),
        &format!("invoke/{}", ability_selector.public_name()),
    ))
}

#[derive(Debug, Clone)]
struct InvokeAbilityRef {
    selector: AbilitySelector,
    descriptor_ref: Option<String>,
}

impl InvokeAbilityRef {
    fn parse(raw: &str) -> anyhow::Result<Self> {
        let raw = raw.trim();
        if raw.contains('@') {
            let descriptor_ref = easynet_axon::invocation::canonical_ability_descriptor_ref(raw)
                .map_err(|err| anyhow::anyhow!("parse <ability-ura>@<version>: {err}"))?;
            let ability_ura =
                crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
                    &descriptor_ref,
                )
                .map_err(|err| anyhow::anyhow!("parse ability URA inside descriptor ref: {err}"))?;
            let selector = AbilitySelector::parse(&ability_ura)
                .with_context(|| "parse ability URA inside descriptor ref")?;
            return Ok(Self {
                selector,
                descriptor_ref: Some(descriptor_ref),
            });
        }

        Ok(Self {
            selector: AbilitySelector::parse(raw).with_context(|| "parse <ability-ura>")?,
            descriptor_ref: None,
        })
    }

    fn selector(&self) -> &AbilitySelector {
        &self.selector
    }

    fn is_descriptor_ref(&self) -> bool {
        self.descriptor_ref.is_some()
    }

    #[cfg(feature = "axon-pb")]
    fn remote_target(
        &self,
        execution_target_ura: &str,
    ) -> anyhow::Result<
        crate::daemon::invocation::routing::remote_invoke::RemoteAbilityInvocationTarget,
    > {
        match self.descriptor_ref.as_deref() {
            Some(descriptor_ref) => {
                crate::daemon::invocation::routing::remote_invoke::RemoteAbilityInvocationTarget::from_descriptor_ref(
                    execution_target_ura,
                    descriptor_ref,
                )
            }
            None => {
                crate::daemon::invocation::routing::remote_invoke::RemoteAbilityInvocationTarget::from_ability_ura(
                    execution_target_ura,
                    self.selector.ability_ura(),
                )
            }
        }
    }
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
    fn invoke_ability_ref_parses_plain_ability_ura() {
        let parsed =
            InvokeAbilityRef::parse("easynet:///r/acme/ability/device.node.observe.health")
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
        let parsed = InvokeAbilityRef::parse(descriptor_ref).expect("descriptor ref");

        assert_eq!(
            parsed.selector().ability_ura(),
            "easynet:///r/acme/ability/device.node.observe.health"
        );
        assert_eq!(parsed.descriptor_ref.as_deref(), Some(descriptor_ref));
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
        });
        let err = res.expect_err("must reject non-canonical --node");
        let msg = format!("{err}");
        // axon-pb on: parse_node_ura error mentions canonical URA
        // shape. axon-pb off: the legacy "not wired" message still
        // mentions `--node`. Either is acceptable as an operator-
        // actionable error.
        assert!(
            msg.contains("--node") || msg.contains("canonical") || msg.contains("axon-pb"),
            "error must surface a --node-related message, got: {msg}"
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
        });
        let err = res.expect_err("must reject malformed JSON");
        assert!(format!("{err:#}").contains("parse --args JSON"));
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn default_owner_invoke_subject_uses_credentials_user_resource() {
        let credentials = crate::daemon::persistence::config::Credentials {
            node_id: "node-a".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "https://hub.example".to_string(),
            realm: "easynet.run".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("dev".to_string()),
            user_id: Some("user-123".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        let selector =
            AbilitySelector::parse("easynet:///r/easynet.run/ability/device.node-b.shell.run")
                .expect("ability selector");

        assert_eq!(
            default_owner_invoke_subject(&credentials, &selector).as_deref(),
            Some("easynet:///r/easynet.run/resource/user.user-123/invoke/shell.run")
        );
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

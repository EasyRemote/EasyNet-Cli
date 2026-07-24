//! CLI facade for target-owned remote system ability dispatch.
//!
//! Command modules should map argv into ability arguments and call this facade.
//! The lower daemon invocation routing primitive remains owned by
//! `daemon::invocation`; this module is the CLI boundary that knows how simple
//! device/hub system ability sugar projects onto that primitive.

use serde_json::Value;

#[cfg(feature = "axon-pb")]
use anyhow::Context;

#[cfg(feature = "axon-pb")]
use crate::daemon::invocation::routing::remote_invoke::{
    self, RemoteAbilityInvocationTarget, RemoteSystemInvocationIssuer,
};

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_remote_device_system_ability(
    node: &str,
    selector: &str,
    args: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    let _ = action_label;
    let target_ura = crate::support::platform::remote_device::resolve_target_device_ura(node)?;
    let caller_ura =
        crate::support::platform::remote_device::require_caller_device_ura_from_credentials()?;
    invoke_target_owned_system_ability(&target_ura, selector, args, &caller_ura)
        .with_context(|| format!("forward {selector} to remote device target={target_ura}"))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_remote_device_system_ability(
    node: &str,
    _selector: &str,
    _args: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    let label = if action_label.trim().is_empty() {
        format!("invoking a remote system ability on node {node:?}")
    } else {
        action_label.to_string()
    };
    Err(crate::support::platform::local_invoke::federation_capability_unsupported_error(&label))
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_current_realm_hub_system_ability(
    selector: &str,
    args: Value,
) -> anyhow::Result<Option<Value>> {
    let context = match CurrentRealmHubInvocationContext::resolve()? {
        CurrentRealmHubInvocationContext::Ready(context) => context,
        CurrentRealmHubInvocationContext::Unpaired => return Ok(None),
    };
    let value =
        invoke_target_owned_system_ability(&context.hub_ura, selector, args, &context.caller_ura)
            .with_context(|| format!("invoke {selector} against realm hub"))?;
    Ok(Some(value))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_current_realm_hub_system_ability(
    _selector: &str,
    _args: Value,
) -> anyhow::Result<Option<Value>> {
    Ok(None)
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum CurrentRealmHubInvocationContext {
    Ready(ResolvedCurrentRealmHubInvocationContext),
    Unpaired,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedCurrentRealmHubInvocationContext {
    hub_ura: String,
    caller_ura: String,
}

#[cfg(feature = "axon-pb")]
impl CurrentRealmHubInvocationContext {
    fn resolve() -> anyhow::Result<Self> {
        let Some(creds) = crate::daemon::persistence::config::load_credentials_optional()? else {
            return Ok(Self::Unpaired);
        };
        let realm = creds.realm_str().trim();
        let node_id = creds.node_id.trim();
        let context = ResolvedCurrentRealmHubInvocationContext {
            hub_ura: crate::core::ura::hub_ura(realm),
            caller_ura: crate::core::ura::device_ura(realm, node_id),
        };
        Ok(Self::Ready(context))
    }
}

#[cfg(feature = "axon-pb")]
fn invoke_target_owned_system_ability(
    execution_target_ura: &str,
    selector: &str,
    args: Value,
    caller_ura: &str,
) -> anyhow::Result<Value> {
    let target_call =
        RemoteAbilityInvocationTarget::for_target_owned_selector(execution_target_ura, selector)?;
    let request = RemoteSystemInvocationIssuer::target_owned_root_plan(
        &target_call,
        caller_ura,
        args,
        std::time::Duration::from_secs(30),
    )?
    .into_request()?;
    remote_invoke::invoke_remote_target(request)
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;

    #[test]
    fn current_realm_hub_context_is_unpaired_when_credentials_are_absent() {
        let _guard = crate::cli::commands::test_support::HomeGuard::new();

        assert_eq!(
            CurrentRealmHubInvocationContext::resolve().unwrap(),
            CurrentRealmHubInvocationContext::Unpaired
        );
    }

    #[test]
    fn current_realm_hub_context_derives_hub_and_caller_from_valid_credentials() {
        let _guard = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "dev-a".to_string(),
                credential_token: "token".to_string(),
                realm: "acme".to_string(),
                hub_endpoint: "axon://hub.example:50051".to_string(),
                username: Some("alice".to_string()),
                user_id: Some("user-alice".to_string()),
                ..Default::default()
            },
        )
        .expect("write test credentials");

        assert_eq!(
            CurrentRealmHubInvocationContext::resolve().unwrap(),
            CurrentRealmHubInvocationContext::Ready(ResolvedCurrentRealmHubInvocationContext {
                hub_ura: "easynet:///r/acme/authority".to_string(),
                caller_ura: "easynet:///r/acme/device/dev-a".to_string(),
            })
        );
    }

    #[test]
    fn current_realm_hub_context_rejects_malformed_credentials() {
        let _guard = crate::cli::commands::test_support::HomeGuard::new();
        std::fs::create_dir_all(crate::daemon::persistence::config::state_dir())
            .expect("state dir");
        std::fs::write(
            crate::daemon::persistence::config::state_dir().join("credentials.json"),
            b"{",
        )
        .expect("write malformed credentials");

        let error = CurrentRealmHubInvocationContext::resolve()
            .expect_err("malformed credentials must not collapse to unpaired");

        assert!(
            error.to_string().contains("parse credentials"),
            "wrong error: {error}"
        );
    }

    #[test]
    fn current_realm_hub_context_rejects_incomplete_credentials() {
        let _guard = crate::cli::commands::test_support::HomeGuard::new();
        std::fs::create_dir_all(crate::daemon::persistence::config::state_dir())
            .expect("state dir");
        std::fs::write(
            crate::daemon::persistence::config::state_dir().join("credentials.json"),
            r#"{
  "node_id": "",
  "credential_token": "token",
  "hub_endpoint": "axon://hub.example:7700",
  "realm": "acme",
  "username": "alice",
  "user_id": "user-alice"
}
"#,
        )
        .expect("write incomplete credentials");

        let error = CurrentRealmHubInvocationContext::resolve()
            .expect_err("incomplete credentials must not collapse to unpaired");

        assert!(
            error.to_string().contains("validate credentials"),
            "wrong error: {error}"
        );
    }
}

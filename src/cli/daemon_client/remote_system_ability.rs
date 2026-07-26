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
    self, RemoteAbilityInvocationTarget, RemoteCatalogueReadIssuer, RemoteSystemInvocationIssuer,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteDeviceSystemAbility {
    NodeDescribe,
    ProcessExec,
}

impl RemoteDeviceSystemAbility {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NodeDescribe => "node.describe",
            Self::ProcessExec => "process.exec",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RealmHubSystemAbility {
    VoiceCreateCall,
    VoiceShowCall,
    VoiceJoinCall,
    VoiceLeaveCall,
    VoiceEndCall,
    VoiceWatchCall,
    VoiceReportMetrics,
}

impl RealmHubSystemAbility {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::VoiceCreateCall => "voice.create_call",
            Self::VoiceShowCall => "voice.show_call",
            Self::VoiceJoinCall => "voice.join_call",
            Self::VoiceLeaveCall => "voice.leave_call",
            Self::VoiceEndCall => "voice.end_call",
            Self::VoiceWatchCall => "voice.watch_call",
            Self::VoiceReportMetrics => "voice.report_metrics",
        }
    }
}

#[cfg(feature = "axon-pb")]
trait TargetOwnedRemoteSystemAbilityName {
    fn remote_system_ability_name(self) -> &'static str;
}

#[cfg(feature = "axon-pb")]
impl TargetOwnedRemoteSystemAbilityName for RemoteDeviceSystemAbility {
    fn remote_system_ability_name(self) -> &'static str {
        self.as_str()
    }
}

#[cfg(feature = "axon-pb")]
impl TargetOwnedRemoteSystemAbilityName for RealmHubSystemAbility {
    fn remote_system_ability_name(self) -> &'static str {
        self.as_str()
    }
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_remote_device_system_ability(
    node: &str,
    ability: RemoteDeviceSystemAbility,
    args: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    let _ = action_label;
    let selector = ability.as_str();
    let target_ura = crate::support::platform::remote_device::resolve_target_device_ura(node)?;
    let caller_ura =
        crate::support::platform::remote_device::require_caller_device_ura_from_credentials()?;
    invoke_target_owned_system_ability(&target_ura, ability, args, &caller_ura)
        .with_context(|| format!("forward {selector} to remote device target={target_ura}"))
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_remote_device_system_ability_as_caller(
    target_ura: &str,
    ability: RemoteDeviceSystemAbility,
    args: Value,
    caller_ura: &str,
) -> anyhow::Result<Value> {
    let target_ura = remote_invoke::parse_node_ura(target_ura)?;
    let selector = ability.as_str();
    invoke_target_owned_system_ability(&target_ura, ability, args, caller_ura)
        .with_context(|| format!("forward {selector} to remote device target={target_ura}"))
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_remote_device_catalogue_read(
    node: &str,
    args: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    let _ = action_label;
    let target_ura = crate::support::platform::remote_device::resolve_target_device_ura(node)?;
    let caller_ura =
        crate::support::platform::remote_device::require_caller_device_ura_from_credentials()?;
    invoke_remote_catalogue_read_for_target(&target_ura, args, &caller_ura).with_context(|| {
        format!("forward meta.list_abilities to remote device target={target_ura}")
    })
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_remote_device_system_ability(
    node: &str,
    _ability: RemoteDeviceSystemAbility,
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

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_remote_device_system_ability_as_caller(
    target_ura: &str,
    _ability: RemoteDeviceSystemAbility,
    _args: Value,
    _caller_ura: &str,
) -> anyhow::Result<Value> {
    Err(
        crate::support::platform::local_invoke::federation_capability_unsupported_error(&format!(
            "invoking a remote system ability on node {target_ura:?}"
        )),
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_remote_device_catalogue_read(
    node: &str,
    _args: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    let label = if action_label.trim().is_empty() {
        format!("reading a remote ability catalogue on node {node:?}")
    } else {
        action_label.to_string()
    };
    Err(crate::support::platform::local_invoke::federation_capability_unsupported_error(&label))
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_current_realm_hub_system_ability(
    ability: RealmHubSystemAbility,
    args: Value,
) -> anyhow::Result<Option<Value>> {
    let context = match CurrentRealmHubInvocationContext::resolve()? {
        CurrentRealmHubInvocationContext::Ready(context) => context,
        CurrentRealmHubInvocationContext::Unpaired => return Ok(None),
    };
    let selector = ability.as_str();
    let value =
        invoke_target_owned_system_ability(&context.hub_ura, ability, args, &context.caller_ura)
            .with_context(|| format!("invoke {selector} against realm hub"))?;
    Ok(Some(value))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_current_realm_hub_system_ability(
    _ability: RealmHubSystemAbility,
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
fn invoke_target_owned_system_ability<A>(
    execution_target_ura: &str,
    ability: A,
    args: Value,
    caller_ura: &str,
) -> anyhow::Result<Value>
where
    A: TargetOwnedRemoteSystemAbilityName,
{
    let selector = ability.remote_system_ability_name();
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

#[cfg(feature = "axon-pb")]
fn invoke_remote_catalogue_read_for_target(
    execution_target_ura: &str,
    args: Value,
    caller_ura: &str,
) -> anyhow::Result<Value> {
    let target_call = RemoteAbilityInvocationTarget::for_catalogue_read(execution_target_ura)?;
    let request = RemoteCatalogueReadIssuer::catalogue_read_plan(
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
    fn typed_facade_does_not_expose_receipt_history_as_target_owned_system_ability() {
        let remote_device_abilities = [
            RemoteDeviceSystemAbility::NodeDescribe,
            RemoteDeviceSystemAbility::ProcessExec,
        ];
        let realm_hub_abilities = [
            RealmHubSystemAbility::VoiceCreateCall,
            RealmHubSystemAbility::VoiceShowCall,
            RealmHubSystemAbility::VoiceJoinCall,
            RealmHubSystemAbility::VoiceLeaveCall,
            RealmHubSystemAbility::VoiceEndCall,
            RealmHubSystemAbility::VoiceWatchCall,
            RealmHubSystemAbility::VoiceReportMetrics,
        ];

        assert!(remote_device_abilities
            .iter()
            .all(|ability| !ability.as_str().starts_with("invocation.history.")));
        assert!(realm_hub_abilities
            .iter()
            .all(|ability| !ability.as_str().starts_with("invocation.history.")));
    }

    #[test]
    fn typed_facade_does_not_expose_catalogue_read_as_target_owned_device_action() {
        let remote_device_abilities = [
            RemoteDeviceSystemAbility::NodeDescribe,
            RemoteDeviceSystemAbility::ProcessExec,
        ];

        assert!(remote_device_abilities
            .iter()
            .all(|ability| ability.as_str() != "meta.list_abilities"));
    }

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

//! CLI facade for target-owned remote system ability dispatch.
//!
//! Command modules should map argv into ability arguments and call this facade.
//! The lower daemon invocation routing primitive remains owned by
//! `daemon::invocation`; this module is the CLI boundary that knows how simple
//! device/hub target sugar projects onto that primitive.
//!
//! A Device target is an execution host/custody selector. The routed callee and
//! descriptor owner remain the target's SystemAgent or Authority selected by the
//! remote invocation target.

use serde_json::Value;

#[cfg(feature = "axon-pb")]
use anyhow::Context;

#[cfg(feature = "axon-pb")]
use crate::daemon::invocation::routing::remote_invoke::{
    self, RemoteAbilityInvocationTarget, RemoteCatalogueReadIssuer, RemoteSessionInvocationIssuer,
    RemoteSystemInvocationIssuer, RemoteUserActionInvocationIssuer,
};
#[cfg(feature = "axon-pb")]
use crate::daemon::invocation::routing::route_target;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteTargetSystemAbility {
    NodeDescribe,
    ProcessExec,
    TerminalCreate,
    TerminalList,
}

impl RemoteTargetSystemAbility {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NodeDescribe => "node.describe",
            Self::ProcessExec => "process.exec",
            Self::TerminalCreate => "terminal.create",
            Self::TerminalList => "terminal.list",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteDeviceSessionAbility {
    Close,
}

impl RemoteDeviceSessionAbility {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Close => "terminal.close",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RealmHubSystemAbility {
    CreateCall,
    ShowCall,
    JoinCall,
    LeaveCall,
    EndCall,
    WatchCall,
    ReportMetrics,
}

impl RealmHubSystemAbility {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::CreateCall => "voice.create_call",
            Self::ShowCall => "voice.show_call",
            Self::JoinCall => "voice.join_call",
            Self::LeaveCall => "voice.leave_call",
            Self::EndCall => "voice.end_call",
            Self::WatchCall => "voice.watch_call",
            Self::ReportMetrics => "voice.report_metrics",
        }
    }
}

#[cfg(feature = "axon-pb")]
trait TargetOwnedRemoteSystemAbilityName {
    fn remote_system_ability_name(self) -> &'static str;
}

#[cfg(feature = "axon-pb")]
impl TargetOwnedRemoteSystemAbilityName for RemoteTargetSystemAbility {
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
    ability: RemoteTargetSystemAbility,
    args: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    let selector = ability.as_str();
    let target_ura = crate::support::platform::remote_device::resolve_target_device_ura(node)?;
    let identity =
        crate::support::platform::remote_device::PairedInvocationIdentity::load(action_label)?;
    invoke_target_owned_system_ability(&target_ura, ability, args, identity.caller_user_ura())
        .with_context(|| format!("forward {selector} to remote device target={target_ura}"))
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_remote_device_system_ability_as_caller(
    target_ura: &str,
    ability: RemoteTargetSystemAbility,
    args: Value,
    caller_ura: &str,
) -> anyhow::Result<Value> {
    let target_ura = route_target::parse_device_placement_ura(target_ura)?;
    let selector = ability.as_str();
    invoke_target_owned_system_ability(&target_ura, ability, args, caller_ura)
        .with_context(|| format!("forward {selector} to remote device target={target_ura}"))
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_remote_device_session_ability(
    target_ura: &str,
    caller_ura: &str,
    subject_ura: &str,
    ability: RemoteDeviceSessionAbility,
    args: Value,
    authority_metadata: crate::daemon::invocation::admission::authority_metadata::IssuedAuthorityMetadata,
    signer: crate::daemon::invocation::routing::remote_invoke::RemoteInvocationCallerSigner,
) -> anyhow::Result<Value> {
    let target_ura = route_target::parse_device_placement_ura(target_ura)?;
    let timeout = crate::support::platform::timeouts::remote_system_transport_guard(0)
        .map_err(anyhow::Error::msg)?;
    let target_call =
        RemoteAbilityInvocationTarget::for_target_owned_selector(&target_ura, ability.as_str())?;
    let request = RemoteSessionInvocationIssuer::followup_root_plan(
        &target_call,
        caller_ura,
        subject_ura,
        args,
        timeout,
    )?
    .into_request()?
    .with_authority_metadata(authority_metadata);
    remote_invoke::invoke_remote_target_with_signer(request, signer).with_context(|| {
        format!(
            "forward {} to remote device target={target_ura}",
            ability.as_str()
        )
    })
}

#[cfg(feature = "axon-pb")]
pub(crate) async fn open_remote_terminal_attach(
    target_ura: &str,
    caller_ura: &str,
    subject_ura: &str,
    session_id: &str,
    attachment_id: &str,
    expected_epoch: u64,
    authority_metadata: crate::daemon::invocation::admission::authority_metadata::IssuedAuthorityMetadata,
    signer: crate::daemon::invocation::routing::remote_invoke::RemoteInvocationCallerSigner,
    timeout: std::time::Duration,
) -> anyhow::Result<crate::support::platform::bidi_session::DaemonBidiSession> {
    let target_ura = route_target::parse_device_placement_ura(target_ura)?;
    let target_call = RemoteAbilityInvocationTarget::for_target_owned_selector_for_mode(
        &target_ura,
        crate::daemon::ability::names::device_control::TERMINAL_ATTACH,
        crate::daemon::ability::CallMode::Bidi,
    )?;
    let request = RemoteSessionInvocationIssuer::followup_root_plan(
        &target_call,
        caller_ura,
        subject_ura,
        serde_json::json!({
            "session_id": session_id,
            "attachment_id": attachment_id,
            "expected_epoch": expected_epoch,
        }),
        timeout,
    )?
    .into_request()?
    .with_authority_metadata(authority_metadata);
    remote_invoke::open_remote_target_bidi_session_with_signer(request, signer)
        .await
        .with_context(|| format!("attach terminal session on remote device target={target_ura}"))
}

#[cfg(feature = "axon-pb")]
pub(crate) async fn open_remote_file_transfer(
    target_ura: &str,
    caller_ura: &str,
    subject_ura: &str,
    args: Value,
    signer: crate::daemon::invocation::routing::remote_invoke::RemoteInvocationCallerSigner,
    timeout: std::time::Duration,
) -> anyhow::Result<crate::support::platform::bidi_session::DaemonBidiSession> {
    let target_ura = route_target::parse_device_placement_ura(target_ura)?;
    let target_call = RemoteAbilityInvocationTarget::for_target_owned_selector_for_mode(
        &target_ura,
        crate::daemon::ability::names::device_control::FS_TRANSFER,
        crate::daemon::ability::CallMode::Bidi,
    )?;
    let request = RemoteUserActionInvocationIssuer::caller_declared_root_plan(
        &target_call,
        caller_ura,
        subject_ura,
        args,
        timeout,
    )?
    .into_request()?;
    remote_invoke::open_remote_target_bidi_session_with_signer(request, signer)
        .await
        .with_context(|| format!("open fs.transfer on remote device target={target_ura}"))
}

#[cfg(feature = "axon-pb")]
pub(crate) async fn open_remote_net_tunnel(
    target_ura: &str,
    caller_ura: &str,
    subject_ura: &str,
    args: Value,
    signer: crate::daemon::invocation::routing::remote_invoke::RemoteInvocationCallerSigner,
    timeout: std::time::Duration,
) -> anyhow::Result<crate::support::platform::bidi_session::DaemonBidiSession> {
    let target_ura = route_target::parse_device_placement_ura(target_ura)?;
    let target_call = RemoteAbilityInvocationTarget::for_target_owned_selector_for_mode(
        &target_ura,
        crate::daemon::ability::names::device_control::NET_TUNNEL,
        crate::daemon::ability::CallMode::Bidi,
    )?;
    let request = RemoteUserActionInvocationIssuer::caller_declared_root_plan(
        &target_call,
        caller_ura,
        subject_ura,
        args,
        timeout,
    )?
    .into_request()?;
    remote_invoke::open_remote_target_bidi_session_with_signer(request, signer)
        .await
        .with_context(|| format!("open net.tunnel on remote device target={target_ura}"))
}

#[cfg(feature = "axon-pb")]
pub(crate) fn upload_target_resource_via_file_transfer(
    target_ura: &str,
    caller_ura: &str,
    subject_ura: &str,
    resource_ref: Value,
    input_chunks: Vec<Vec<u8>>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalBidiFrame>> {
    let target_ura = route_target::parse_device_placement_ura(target_ura)?;
    let timeout = crate::support::platform::timeouts::remote_system_transport_guard(0)
        .map_err(anyhow::Error::msg)?;
    let target_call = RemoteAbilityInvocationTarget::for_target_owned_selector_for_mode(
        &target_ura,
        crate::daemon::ability::names::device_control::FS_TRANSFER,
        crate::daemon::ability::CallMode::Bidi,
    )?;
    let request = RemoteUserActionInvocationIssuer::caller_declared_root_plan(
        &target_call,
        caller_ura,
        subject_ura,
        serde_json::json!({
            "mode": "upload",
            "resource_ref": resource_ref,
        }),
        timeout,
    )?
    .into_request()?;
    let mut frames: Vec<_> = input_chunks
        .into_iter()
        .map(remote_invoke::RemoteBidiInputFrame::Binary)
        .collect();
    frames.push(remote_invoke::RemoteBidiInputFrame::Eof);
    remote_invoke::invoke_remote_target_bidi_frames(request, frames, None)
        .with_context(|| format!("upload ResourceRef to remote device target={target_ura}"))
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_target_ability_deploy_from_resource(
    target_ura: &str,
    caller_ura: &str,
    subject_ura: &str,
    resource_ref: Value,
) -> anyhow::Result<Value> {
    let target_ura = route_target::parse_device_placement_ura(target_ura)?;
    let timeout = crate::support::platform::timeouts::remote_system_transport_guard(0)
        .map_err(anyhow::Error::msg)?;
    let target_call = RemoteAbilityInvocationTarget::for_target_owned_selector(
        &target_ura,
        crate::daemon::ability::names::federation::ABILITY_DEPLOY,
    )?;
    let request = RemoteUserActionInvocationIssuer::caller_declared_root_plan(
        &target_call,
        caller_ura,
        subject_ura,
        serde_json::json!({
            "resource_ref": resource_ref,
            "target_ura": target_ura,
        }),
        timeout,
    )?
    .into_request()?;
    remote_invoke::invoke_remote_target(request)
        .with_context(|| format!("invoke ability.deploy on remote device target={target_ura}"))
}

/// Invoke the inverse deployment transition with the same ontology as deploy:
/// an accountable User caller addresses the target's ability-management
/// SystemAgent, the removed Ability URA is the subject, and the Device remains
/// only the execution host selected by `target_ura`.
#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_target_ability_uninstall(
    target_ura: &str,
    caller_ura: &str,
    ability_ura: &str,
    install_id: Option<&str>,
) -> anyhow::Result<Value> {
    let target_ura = route_target::parse_device_placement_ura(target_ura)?;
    let timeout = crate::support::platform::timeouts::remote_system_transport_guard(0)
        .map_err(anyhow::Error::msg)?;
    let target_call = RemoteAbilityInvocationTarget::for_target_owned_selector(
        &target_ura,
        crate::daemon::ability::names::federation::ABILITY_UNINSTALL,
    )?;
    let mut args = serde_json::json!({
        "ability_ura": ability_ura,
        "target_ura": target_ura,
    });
    if let Some(install_id) = install_id.filter(|value| !value.trim().is_empty()) {
        args["install_id"] = serde_json::json!(install_id);
    }
    let request = RemoteUserActionInvocationIssuer::caller_declared_root_plan(
        &target_call,
        caller_ura,
        ability_ura,
        args,
        timeout,
    )?
    .into_request()?;
    remote_invoke::invoke_remote_target(request)
        .with_context(|| format!("invoke ability.uninstall on device host target={target_ura}"))
}

/// Run one user-requested Mission on a Device-hosted automation SystemAgent.
///
/// The target Device selects the execution host, the registry projects the
/// automation SystemAgent callee, and `subject_ura` names the entity the
/// Mission acts on (for `agent send`, the canonical hosted Agent).
#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_agent_subject_mission_run(
    target_device_ura: &str,
    caller_user_ura: &str,
    subject_ura: &str,
    args: Value,
    timeout: std::time::Duration,
) -> anyhow::Result<Value> {
    let target_device_ura = route_target::parse_device_placement_ura(target_device_ura)?;
    let subject = crate::core::ura::parse_ura(subject_ura)
        .map_err(|error| anyhow::anyhow!("mission.run subject URA is invalid: {error}"))?;
    if subject.kind != crate::core::ura::URAKind::Agent {
        anyhow::bail!(
            "mission.run user action subject must be an Agent URA, got {}",
            subject.kind
        );
    }
    let target_call = RemoteAbilityInvocationTarget::for_target_owned_selector(
        &target_device_ura,
        crate::daemon::ability::names::automation::MISSION_RUN,
    )?;
    let request = RemoteUserActionInvocationIssuer::caller_declared_root_plan(
        &target_call,
        caller_user_ura,
        subject_ura,
        args,
        timeout,
    )?
    .into_request()?;
    remote_invoke::invoke_remote_target(request)
        .with_context(|| format!("invoke mission.run on device host target={target_device_ura}"))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn upload_target_resource_via_file_transfer(
    target_ura: &str,
    _caller_ura: &str,
    _subject_ura: &str,
    _resource_ref: Value,
    _input_chunks: Vec<Vec<u8>>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalBidiFrame>> {
    Err(
        crate::support::platform::local_invoke::federation_capability_unsupported_error(&format!(
            "uploading a ResourceRef to remote device target {target_ura:?}"
        )),
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_target_ability_deploy_from_resource(
    target_ura: &str,
    _caller_ura: &str,
    _subject_ura: &str,
    _resource_ref: Value,
) -> anyhow::Result<Value> {
    Err(
        crate::support::platform::local_invoke::federation_capability_unsupported_error(&format!(
            "deploying ability bundle to remote device target {target_ura:?}"
        )),
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_target_ability_uninstall(
    target_ura: &str,
    _caller_ura: &str,
    _ability_ura: &str,
    _install_id: Option<&str>,
) -> anyhow::Result<Value> {
    Err(
        crate::support::platform::local_invoke::federation_capability_unsupported_error(&format!(
            "uninstalling an ability from device host target {target_ura:?}"
        )),
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_agent_subject_mission_run(
    target_device_ura: &str,
    _caller_user_ura: &str,
    _subject_ura: &str,
    _args: Value,
    _timeout: std::time::Duration,
) -> anyhow::Result<Value> {
    Err(
        crate::support::platform::local_invoke::federation_capability_unsupported_error(&format!(
            "invoking mission.run on device host {target_device_ura:?}"
        )),
    )
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_remote_device_catalogue_read(
    node: &str,
    args: Value,
    action_label: &str,
) -> anyhow::Result<Value> {
    let target_ura = crate::support::platform::remote_device::resolve_target_device_ura(node)?;
    let identity =
        crate::support::platform::remote_device::PairedInvocationIdentity::load(action_label)?;
    match CatalogueReadRoute::resolve(
        target_ura,
        identity.local_device_ura().to_string(),
        identity.caller_user_ura().to_string(),
    ) {
        CatalogueReadRoute::LocalRuntime { target_ura } => {
            crate::support::platform::local_invoke::LocalRuntimeCatalogueReadIssuer::list_abilities(
                args,
            )
            .with_context(|| format!("read local meta.list_abilities for target={target_ura}"))
        }
        CatalogueReadRoute::RemoteTarget {
            target_ura,
            caller_ura,
        } => invoke_remote_catalogue_read_for_target(&target_ura, args, &caller_ura).with_context(
            || format!("forward meta.list_abilities to remote device target={target_ura}"),
        ),
    }
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_remote_device_system_ability(
    node: &str,
    _ability: RemoteTargetSystemAbility,
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
    _ability: RemoteTargetSystemAbility,
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
#[derive(Debug, Clone, PartialEq, Eq)]
enum CatalogueReadRoute {
    LocalRuntime {
        target_ura: String,
    },
    RemoteTarget {
        target_ura: String,
        caller_ura: String,
    },
}

#[cfg(feature = "axon-pb")]
impl CatalogueReadRoute {
    fn resolve(target_ura: String, local_device_ura: String, caller_user_ura: String) -> Self {
        if target_ura == local_device_ura {
            Self::LocalRuntime { target_ura }
        } else {
            Self::RemoteTarget {
                target_ura,
                caller_ura: caller_user_ura,
            }
        }
    }
}

#[cfg(feature = "axon-pb")]
impl CurrentRealmHubInvocationContext {
    fn resolve() -> anyhow::Result<Self> {
        let Some(creds) = crate::daemon::persistence::config::load_credentials_optional()? else {
            return Ok(Self::Unpaired);
        };
        let realm = creds.realm_str().trim();
        let caller_ura = match creds.runtime_user_binding()? {
            crate::daemon::persistence::config::RuntimeUserBinding::Bound { user_ura } => user_ura,
            crate::daemon::persistence::config::RuntimeUserBinding::Unbound { reason } => {
                anyhow::bail!(
                    "realm Hub invocation requires an accountable User Principal caller; runtime user binding is {reason}"
                )
            }
        };
        let context = ResolvedCurrentRealmHubInvocationContext {
            hub_ura: crate::core::ura::hub_ura(realm),
            caller_ura,
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
    let timeout = crate::support::platform::timeouts::remote_system_transport_guard(0)
        .map_err(anyhow::Error::msg)?;
    let target_call =
        RemoteAbilityInvocationTarget::for_target_owned_selector(execution_target_ura, selector)?;
    let request = RemoteSystemInvocationIssuer::target_owned_root_plan(
        &target_call,
        caller_ura,
        args,
        timeout,
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
    let timeout = crate::support::platform::timeouts::catalogue_read_transport_guard(0)
        .map_err(anyhow::Error::msg)?;
    let target_call = RemoteAbilityInvocationTarget::for_catalogue_read(execution_target_ura)?;
    let request =
        RemoteCatalogueReadIssuer::catalogue_read_plan(&target_call, caller_ura, args, timeout)?
            .into_request()?;
    remote_invoke::invoke_remote_target(request)
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;

    #[test]
    fn typed_facade_does_not_expose_receipt_history_as_target_owned_system_ability() {
        let remote_device_abilities = [
            RemoteTargetSystemAbility::NodeDescribe,
            RemoteTargetSystemAbility::ProcessExec,
            RemoteTargetSystemAbility::TerminalCreate,
        ];
        let realm_hub_abilities = [
            RealmHubSystemAbility::CreateCall,
            RealmHubSystemAbility::ShowCall,
            RealmHubSystemAbility::JoinCall,
            RealmHubSystemAbility::LeaveCall,
            RealmHubSystemAbility::EndCall,
            RealmHubSystemAbility::WatchCall,
            RealmHubSystemAbility::ReportMetrics,
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
            RemoteTargetSystemAbility::NodeDescribe,
            RemoteTargetSystemAbility::ProcessExec,
            RemoteTargetSystemAbility::TerminalCreate,
        ];

        assert!(remote_device_abilities
            .iter()
            .all(|ability| ability.as_str() != "meta.list_abilities"));
    }

    #[test]
    fn catalogue_read_route_selects_local_runtime_for_self_device() {
        let local = "easynet:///r/acme/device/dev-a".to_string();

        assert_eq!(
            CatalogueReadRoute::resolve(
                local.clone(),
                local,
                "easynet:///r/acme/user/user-alice".to_string(),
            ),
            CatalogueReadRoute::LocalRuntime {
                target_ura: "easynet:///r/acme/device/dev-a".to_string(),
            }
        );
    }

    #[test]
    fn catalogue_read_route_keeps_peer_device_remote() {
        assert_eq!(
            CatalogueReadRoute::resolve(
                "easynet:///r/acme/device/dev-b".to_string(),
                "easynet:///r/acme/device/dev-a".to_string(),
                "easynet:///r/acme/user/user-alice".to_string(),
            ),
            CatalogueReadRoute::RemoteTarget {
                target_ura: "easynet:///r/acme/device/dev-b".to_string(),
                caller_ura: "easynet:///r/acme/user/user-alice".to_string(),
            }
        );
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
                caller_ura: "easynet:///r/acme/user/user-alice".to_string(),
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

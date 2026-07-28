#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-runtime-state-read-subject-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p \
  "$SB/tools/scripts" \
  "$SB/src/support/platform" \
  "$SB/src/cli/commands/groups" \
  "$SB/src/cli/commands" \
  "$SB/src/cli/daemon_client"
cp "$SCRIPT" "$SB/tools/scripts/check-runtime-state-read-subject-boundary.sh"

cat >"$SB/src/support/platform/local_invoke.rs" <<'RS'
pub struct LocalRuntimeStateReadIssuer;

impl LocalRuntimeStateReadIssuer {
    pub fn agent_list(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::agent_list_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn agent_list_timeout(
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        Self::read_state_timeout("agent.list", args, timeout)
    }

    fn read_state_timeout(
        _ability: &str,
        _args: serde_json::Value,
        _timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        let _ = Self::subject_ura()?;
        Ok(serde_json::json!({}))
    }

    fn subject_ura() -> anyhow::Result<String> {
        LocalRuntimeStateReadAttachment::from_runtime_attachment_file(&KeyServiceRuntimeStateReadSignerCustody)
            .and_then(|attachment| attachment.into_subject_ura())
    }
}

pub struct LocalRuntimeApiKeyInventoryReadIssuer;

impl LocalRuntimeApiKeyInventoryReadIssuer {
    pub fn list_api_keys(ability: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::list_api_keys_timeout(ability, args, std::time::Duration::from_secs(30))
    }

    pub fn list_api_keys_timeout(
        ability: &str,
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        LocalRuntimeStateReadIssuer::read_state_timeout(ability, args, timeout)
    }
}

pub struct LocalRuntimeDeviceDirectoryReadIssuer;

impl LocalRuntimeDeviceDirectoryReadIssuer {
    pub fn describe_node(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::describe_node_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn describe_node_timeout(
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        LocalRuntimeStateReadIssuer::read_state_timeout("node.describe", args, timeout)
    }
}

pub struct LocalRuntimeModelCatalogueReadIssuer;

impl LocalRuntimeModelCatalogueReadIssuer {
    pub fn list_openai_models(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::list_openai_models_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn list_openai_models_timeout(
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        LocalRuntimeStateReadIssuer::read_state_timeout("openai.list_models", args, timeout)
    }
}

pub struct LocalRuntimeSkillCatalogueReadIssuer;

impl LocalRuntimeSkillCatalogueReadIssuer {
    pub fn list_installed_skills(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::list_installed_skills_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn list_installed_skills_timeout(
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        LocalRuntimeStateReadIssuer::read_state_timeout("skill.list", args, timeout)
    }
}

pub struct LocalRuntimeCatalogueReadIssuer;

impl LocalRuntimeCatalogueReadIssuer {
    pub fn list_abilities(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::list_abilities_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn list_abilities_timeout(
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        Self::invoke_catalogue_read_timeout("meta.list_abilities", args, timeout)
    }

    pub fn list_resources(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::list_resources_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn list_resources_timeout(
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        Self::invoke_catalogue_read_timeout("meta.list_resources", args, timeout)
    }

    fn invoke_catalogue_read_timeout(
        _ability: &str,
        _args: serde_json::Value,
        _timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }
}

pub struct LocalRuntimeGovernanceReadIssuer;

impl LocalRuntimeGovernanceReadIssuer {
    pub fn invocation_history_path(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::invocation_history_path_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn invocation_history_path_timeout(
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        Self::invoke_governance_read_timeout("invocation.history.path", args, timeout)
    }

    pub fn invocation_history_list(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::invocation_history_list_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn invocation_history_list_timeout(
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        Self::invoke_governance_read_timeout("invocation.history.list", args, timeout)
    }

    pub fn invocation_history_get(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::invocation_history_get_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn invocation_history_get_timeout(
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        Self::invoke_governance_read_timeout("invocation.history.get", args, timeout)
    }

    pub fn invocation_record_get(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::invocation_record_get_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn invocation_record_get_timeout(
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        Self::invoke_governance_read_timeout("invocation.record.get", args, timeout)
    }

    pub fn invocation_trace_get(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::invocation_trace_get_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn invocation_trace_get_timeout(
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        Self::invoke_governance_read_timeout("invocation.trace.get", args, timeout)
    }

    fn invoke_governance_read_timeout(
        _ability: &str,
        _args: serde_json::Value,
        _timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        let _ = LocalRuntimeOwnerReadAttachment::from_discovery_file(
            &KeyServiceRuntimeStateReadSignerCustody,
            "runtime governance read subject unavailable",
        )
        .and_then(|attachment| attachment.into_subject_ura())?;
        Ok(serde_json::json!({}))
    }
}

pub struct LocalRuntimeOperationalReadIssuer;

impl LocalRuntimeOperationalReadIssuer {
    pub fn observe_health(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::observe_health_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn observe_health_timeout(
        _args: serde_json::Value,
        _timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        let _ = LocalRuntimeOwnerReadAttachment::from_discovery_file(
            &KeyServiceRuntimeStateReadSignerCustody,
            "runtime operational read subject unavailable",
        )
        .and_then(|attachment| attachment.into_subject_ura())?;
        Ok(serde_json::json!({}))
    }
}

pub struct LocalRuntimeIdentityReadIssuer;

impl LocalRuntimeIdentityReadIssuer {
    pub fn list_user_pubkeys(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        Self::list_user_pubkeys_timeout(args, std::time::Duration::from_secs(30))
    }

    pub fn list_user_pubkeys_timeout(
        _args: serde_json::Value,
        _timeout: std::time::Duration,
    ) -> anyhow::Result<serde_json::Value> {
        let _ = LocalRuntimeOwnerReadAttachment::from_discovery_file(
            &KeyServiceRuntimeStateReadSignerCustody,
            "runtime identity read subject unavailable",
        )
        .and_then(|attachment| attachment.into_subject_ura())?;
        Ok(serde_json::json!({}))
    }
}

trait RuntimeStateReadSignerCustody {
    fn prove(&self, user_ura: &str) -> anyhow::Result<()>;
}

struct KeyServiceRuntimeStateReadSignerCustody;

impl RuntimeStateReadSignerCustody for KeyServiceRuntimeStateReadSignerCustody {
    fn prove(&self, user_ura: &str) -> anyhow::Result<()> {
        crate::daemon::identity::self_identity::prove_runtime_caller_signer_custody(user_ura)
    }
}

/// Runtime-state reads bind to a user-owned Resource URA.
struct LocalRuntimeStateReadAttachment {
    realm: String,
    user_id: String,
}

enum LocalRuntimeOwnerReadAttachment {
    PairedUser(LocalRuntimeStateReadAttachment),
    RuntimeOwner { subject_ura: String },
}

impl LocalRuntimeOwnerReadAttachment {
    fn from_discovery_file(
        signer_custody: &dyn RuntimeStateReadSignerCustody,
        error_prefix: &'static str,
    ) -> anyhow::Result<Self> {
        let credentials = crate::daemon::persistence::config::load_credentials()?;
        let discovery = crate::daemon::control::discovery::read(
            &crate::daemon::control::discovery::default_path(),
        )?
        .ok_or_else(|| anyhow::anyhow!("{error_prefix}: daemon Ready discovery is missing"))?;
        let identity = discovery.daemon_identity.as_ref().ok_or_else(|| {
            anyhow::anyhow!("{error_prefix}: daemon Ready discovery has no runtime identity")
        })?;
        if identity.mode == "hub" {
            let authority = crate::core::ura::hub_ura(identity.realm.trim());
            crate::core::identity::RuntimeGovernanceReadSubject::parse_for_callee(
                &authority,
                &authority,
            )?;
            return Ok(Self::RuntimeOwner {
                subject_ura: authority,
            });
        }
        let attachment = LocalRuntimeStateReadAttachment::from_runtime_attachment(
            &credentials,
            &discovery,
            signer_custody,
        )?;
        Ok(Self::PairedUser(attachment))
    }

    fn into_subject_ura(self) -> anyhow::Result<String> {
        match self {
            Self::PairedUser(attachment) => attachment.into_subject_ura(),
            Self::RuntimeOwner { subject_ura } => Ok(subject_ura),
        }
    }
}

impl LocalRuntimeStateReadAttachment {
    fn from_runtime_attachment_file(
        signer_custody: &dyn RuntimeStateReadSignerCustody,
    ) -> anyhow::Result<Self> {
        let credentials = crate::daemon::persistence::config::load_credentials()?;
        let discovery = crate::daemon::control::discovery::read(
            &crate::daemon::control::discovery::default_path(),
        )?
        .ok_or_else(|| anyhow::anyhow!("runtime-state read subject unavailable: daemon Ready discovery is missing"))?;
        Self::from_runtime_attachment(&credentials, &discovery, signer_custody)
    }

    fn from_runtime_attachment(
        credentials: &crate::daemon::persistence::config::Credentials,
        discovery: &crate::daemon::control::discovery::ControlDiscovery,
        signer_custody: &dyn RuntimeStateReadSignerCustody,
    ) -> anyhow::Result<Self> {
        let identity = discovery.daemon_identity.as_ref().ok_or_else(|| {
            anyhow::anyhow!("runtime-state read subject unavailable: daemon Ready discovery has no runtime identity")
        })?;
        if !discovery.capability_flags.iter().any(|flag| {
            flag == crate::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER
        }) {
            anyhow::bail!("runtime-state read subject unavailable: daemon Ready did not prove paired User caller signer custody");
        }
        if identity.realm.trim() != credentials.realm_str().trim() {
            anyhow::bail!("runtime-state read subject unavailable: daemon realm mismatch");
        }
        if let Some(node_id) = identity.node_id.as_deref() {
            if node_id.trim() != credentials.node_id.trim() {
                anyhow::bail!("runtime-state read subject unavailable: daemon node mismatch");
            }
        }
        let user_ura = credentials.user_ura()?;
        signer_custody.prove(&user_ura)?;
        let user_id = credentials.user_id()?;
        Ok(Self {
            realm: credentials.realm_str().to_string(),
            user_id,
        })
    }

    fn subject(&self) -> anyhow::Result<crate::core::identity::RuntimeStateReadSubject> {
        crate::core::identity::RuntimeStateReadSubject::new(&self.realm, &self.user_id)
    }

    fn into_subject_ura(self) -> anyhow::Result<String> {
        self.subject().map(|subject| subject.into_string())
    }
}

#[test]
fn runtime_state_read_subject_uses_user_owned_resource_not_daemon_identity() {}

#[test]
fn runtime_state_read_subject_rejects_missing_user_id_before_device_fallback() {
    let _ = LocalRuntimeStateReadAttachment::from_runtime_attachment(
        &credentials,
        &discovery,
        &ReadyRuntimeStateReadSignerCustody,
    );
}

#[test]
fn runtime_state_read_subject_requires_ready_signer_capability() {}

#[test]
fn runtime_state_read_subject_rejects_stale_runtime_attachment() {}

#[test]
fn runtime_state_read_subject_rejects_missing_live_signer_custody() {}

/// Invoke a canonical local target with public-ingress tuple facts.
pub struct LocalDaemonSystemAbilityIssuer;
RS

for target in \
  "$SB/src/cli/commands/ability_record.rs" \
  "$SB/src/cli/daemon_client/ability_catalog.rs" \
  "$SB/src/cli/daemon_client/remote_system_ability.rs" \
  "$SB/src/daemon/ability/catalog/profiles/mcp.rs"
do
  mkdir -p "$(dirname "$target")"
  cat >"$target" <<'RS'
use crate::support::platform::local_invoke::LocalRuntimeCatalogueReadIssuer;

fn read_runtime_catalogue() {
    let _ = LocalRuntimeCatalogueReadIssuer::list_abilities(serde_json::json!({}));
}
RS
done

for target in \
  "$SB/src/cli/commands/discover.rs"
do
  mkdir -p "$(dirname "$target")"
  cat >"$target" <<'RS'
use crate::support::platform::local_invoke::LocalRuntimeStateReadIssuer;

fn read_runtime_state() {
    let _ = LocalRuntimeStateReadIssuer::agent_list(serde_json::json!({}));
}
RS
done

cat >"$SB/src/cli/commands/status.rs" <<'RS'
use crate::support::platform::local_invoke::{
    LocalRuntimeCatalogueReadIssuer,
    LocalRuntimeOperationalReadIssuer,
};

fn read_status() {
    let _ = LocalRuntimeOperationalReadIssuer::observe_health(serde_json::json!({}));
    let _ = LocalRuntimeCatalogueReadIssuer::list_abilities(serde_json::json!({}));
}
RS

cat >"$SB/src/cli/commands/groups/device.rs" <<'RS'
use crate::support::platform::local_invoke::LocalRuntimeDeviceDirectoryReadIssuer;

fn read_device_directory() {
    let _ = LocalRuntimeDeviceDirectoryReadIssuer::describe_node(serde_json::json!({}));
}
RS

for target in \
  "$SB/src/cli/commands/doctor.rs" \
  "$SB/src/cli/commands/user_signing_identity.rs"
do
  mkdir -p "$(dirname "$target")"
  cat >"$target" <<'RS'
use crate::support::platform::local_invoke::LocalRuntimeIdentityReadIssuer;

fn read_identity() {
    let _ = LocalRuntimeIdentityReadIssuer::list_user_pubkeys(serde_json::json!({}));
}
RS
done

cat >"$SB/src/cli/commands/invocation_watch.rs" <<'RS'
use crate::support::platform::local_invoke::LocalRuntimeGovernanceReadIssuer;

fn watch_governance() {
    let _ = LocalRuntimeGovernanceReadIssuer::invocation_record_get(serde_json::json!({}));
}
RS

mkdir -p "$SB/src/cli/commands/groups"
cat >"$SB/src/cli/commands/groups/mcp.rs" <<'RS'
use crate::support::platform::local_invoke::LocalRuntimeOperationalReadIssuer;

fn read_runtime_health() {
    let _ = LocalRuntimeOperationalReadIssuer::observe_health(serde_json::json!({}));
}
RS

cat >"$SB/src/cli/commands/groups/invocation.rs" <<'RS'
use crate::support::platform::local_invoke::LocalRuntimeGovernanceReadIssuer;

fn read_runtime_governance() {
    let _ = LocalRuntimeGovernanceReadIssuer::invocation_history_list(serde_json::json!({}));
}
RS

mkdir -p "$SB/src/cli/commands/agent" "$SB/src/cli/daemon_client"
cat >"$SB/src/cli/daemon_client/agent_gateway.rs" <<'RS'
pub trait AgentCommandGateway {
    fn invoke(&self, ability: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value>;
}

pub trait AgentReadGateway {
    fn list_agents(&self) -> anyhow::Result<serde_json::Value>;

    fn list_agent_abilities(&self, agent_ura: &str) -> anyhow::Result<serde_json::Value>;
}

struct DaemonAgentCommandGateway;
struct DaemonAgentReadGateway;

impl AgentCommandGateway for DaemonAgentCommandGateway {
    fn invoke(&self, ability: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        crate::support::platform::local_invoke::LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity(
            ability,
            args,
        )
    }
}

impl AgentReadGateway for DaemonAgentReadGateway {
    fn list_agents(&self) -> anyhow::Result<serde_json::Value> {
        crate::support::platform::local_invoke::LocalRuntimeStateReadIssuer::agent_list(
            serde_json::json!({}),
        )
    }

    fn list_agent_abilities(&self, agent_ura: &str) -> anyhow::Result<serde_json::Value> {
        crate::support::platform::local_invoke::LocalRuntimeCatalogueReadIssuer::list_abilities(
            serde_json::json!({
                "scope": "local",
                "agent_ura": agent_ura,
            }),
        )
    }
}
RS

cat >"$SB/src/cli/daemon_client/agent_view.rs" <<'RS'
use crate::cli::daemon_client::agent_gateway::AgentReadGateway;

fn read_agents(gateway: &dyn AgentReadGateway) -> anyhow::Result<serde_json::Value> {
    gateway.list_agents()
}
RS

cat >"$SB/src/cli/commands/agent/publish.rs" <<'RS'
use crate::cli::daemon_client::agent_gateway::AgentReadGateway;

fn publish_view(gateway: &dyn AgentReadGateway) -> anyhow::Result<serde_json::Value> {
    gateway.list_agent_abilities("easynet:///r/acme/agent/alice.helper")
}
RS

cat >"$SB/src/cli/commands/llm_api.rs" <<'RS'
use crate::support::platform::local_invoke::{LocalDaemonSystemAbilityIssuer, LocalRuntimeStateReadIssuer};

fn pick_model() -> anyhow::Result<serde_json::Value> {
    crate::support::platform::local_invoke::LocalRuntimeModelCatalogueReadIssuer::list_openai_models(serde_json::json!({}))
}

fn run(adapter_args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    invoke_openai_chat_completions(adapter_args)
}

fn invoke_openai_chat_completions(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity("openai.chat_completions", args)
}
RS

cat >"$SB/src/cli/commands/skill.rs" <<'RS'
use crate::support::platform::local_invoke::{LocalDaemonSystemAbilityIssuer, LocalRuntimeSkillCatalogueReadIssuer};

fn install() -> anyhow::Result<serde_json::Value> {
    invoke_daemon_skill_mutation("skill.install", serde_json::json!({}))
}

fn list() -> anyhow::Result<serde_json::Value> {
    LocalRuntimeSkillCatalogueReadIssuer::list_installed_skills(serde_json::json!({}))
}

fn upgrade() -> anyhow::Result<serde_json::Value> {
    invoke_daemon_skill_mutation("skill.upgrade", serde_json::json!({}))
}

fn remove() -> anyhow::Result<serde_json::Value> {
    invoke_daemon_skill_mutation("skill.remove", serde_json::json!({}))
}

fn invoke_daemon_skill_mutation(ability: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity(ability, args)
}
RS

cat >"$SB/src/cli/commands/api_key_cli.rs" <<'RS'
use crate::support::platform::local_invoke::{LocalDaemonSystemAbilityIssuer, LocalRuntimeApiKeyInventoryReadIssuer};

struct Principal {
    subject_ura: String,
}

fn create(principal: Principal, ability: String, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    invoke_api_key_manage(&principal, &ability, args)
}

fn list(ability: String) -> anyhow::Result<serde_json::Value> {
    LocalRuntimeApiKeyInventoryReadIssuer::list_api_keys(&ability, serde_json::json!({}))
}

fn revoke(principal: Principal, ability: String) -> anyhow::Result<serde_json::Value> {
    invoke_api_key_manage(&principal, &ability, serde_json::json!({ "id_prefix": "key" }))
}

fn invoke_api_key_manage(
    principal: &Principal,
    ability: &str,
    args: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    LocalDaemonSystemAbilityIssuer::invoke_root_for_subject(ability, args, &principal.subject_ura)
}
RS

cat >"$SB/src/cli/commands/groups/ability.rs" <<'RS'
use crate::support::platform::local_invoke::LocalDaemonSystemAbilityIssuer;

struct UninstallArgs;

fn run_uninstall(args: UninstallArgs) -> anyhow::Result<serde_json::Value> {
    let payload = ability_uninstall_payload(&args)?;
    let result = invoke_ability_uninstall(payload)?;
    Ok(result)
}

fn invoke_ability_uninstall(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity("ability.uninstall", args)
}

fn ability_uninstall_payload(_args: &UninstallArgs) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::json!({}))
}
RS

(
  cd "$SB"
  bash tools/scripts/check-runtime-state-read-subject-boundary.sh
) >/dev/null || fail "happy path should pass"

perl -0pi -e 's/\n    fn into_subject_ura\(self\)/\n    fn from_credentials(\n        _credentials: &crate::daemon::persistence::config::Credentials,\n    ) -> anyhow::Result<Self> {\n        anyhow::bail!("credentials-only constructor")\n    }\n\n    fn into_subject_ura(self)/' \
  "$SB/src/support/platform/local_invoke.rs"

set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-state-read-subject-boundary.sh
) >/tmp/check-runtime-state-read-subject-boundary-from-credentials.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "credentials-only constructor should exit 1 (got $rc)"

perl -0pi -e 's/\n    fn from_credentials\(\n        _credentials: &crate::daemon::persistence::config::Credentials,\n    \) -> anyhow::Result<Self> \{\n        anyhow::bail!\("credentials-only constructor"\)\n    \}\n//' \
  "$SB/src/support/platform/local_invoke.rs"

perl -0pi -e 's/from_runtime_attachment_file\(&KeyServiceRuntimeStateReadSignerCustody\)/from_credentials_file()/g; s/fn from_runtime_attachment_file/fn from_credentials_file/g' \
  "$SB/src/support/platform/local_invoke.rs"

set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-state-read-subject-boundary.sh
) >/tmp/check-runtime-state-read-subject-boundary-credentials-only.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "credentials-only runtime-state subject issuer should exit 1 (got $rc)"

perl -0pi -e 's/from_credentials_file\(\)/from_runtime_attachment_file(&KeyServiceRuntimeStateReadSignerCustody)/g; s/fn from_credentials_file/fn from_runtime_attachment_file/g' \
  "$SB/src/support/platform/local_invoke.rs"

cat >>"$SB/src/cli/commands/groups/invocation.rs" <<'RS'
fn legacy_read() {
    let _ = invoke_local_ability("invocation.history.list", serde_json::json!({}));
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-state-read-subject-boundary.sh
) >/tmp/check-runtime-state-read-subject-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "generic runtime-state read should exit 1 (got $rc)"

perl -0pi -e 's/\nfn legacy_read\(\) \{\n    let _ = invoke_local_ability\("invocation\.history\.list", serde_json::json!\(\{\}\)\);\n\}\n//' \
  "$SB/src/cli/commands/groups/invocation.rs"

perl -0pi -e 's/AgentReadGateway/AgentCommandGateway/g; s/gateway\.list_agents\(\)/gateway.invoke("agent.list", serde_json::json!({}))/g' \
  "$SB/src/cli/daemon_client/agent_view.rs"

set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-state-read-subject-boundary.sh
) >/tmp/check-runtime-state-read-subject-boundary-agent.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "agent.list command-gateway regression should exit 1 (got $rc)"

perl -0pi -e 's/AgentCommandGateway/AgentReadGateway/g; s/gateway\.invoke\("agent\.list", serde_json::json!\(\{\}\)\)/gateway.list_agents()/g' \
  "$SB/src/cli/daemon_client/agent_view.rs"

cat >>"$SB/src/cli/daemon_client/agent_gateway.rs" <<'RS'
fn legacy_agent_command(ability: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    crate::support::platform::local_invoke::invoke_local_ability(ability, args)
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-state-read-subject-boundary.sh
) >/tmp/check-runtime-state-read-subject-boundary-agent-command.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "agent command gateway generic invoke regression should exit 1 (got $rc)"

perl -0pi -e 's/\nfn legacy_agent_command\(ability: &str, args: serde_json::Value\) -> anyhow::Result<serde_json::Value> \{\n    crate::support::platform::local_invoke::invoke_local_ability\(ability, args\)\n\}\n//' \
  "$SB/src/cli/daemon_client/agent_gateway.rs"

perl -0pi -e 's/\Qcrate::support::platform::local_invoke::LocalRuntimeModelCatalogueReadIssuer::list_openai_models(\E/invoke_local_ability("openai.list_models", /' \
  "$SB/src/cli/commands/llm_api.rs"

set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-state-read-subject-boundary.sh
) >/tmp/check-runtime-state-read-subject-boundary-llm.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "llm-api model read regression should exit 1 (got $rc)"

perl -0pi -e 's/\Qinvoke_local_ability("openai.list_models", \E/crate::support::platform::local_invoke::LocalRuntimeModelCatalogueReadIssuer::list_openai_models(/' \
  "$SB/src/cli/commands/llm_api.rs"

perl -0pi -e 's/\QLocalRuntimeSkillCatalogueReadIssuer::list_installed_skills(\E/invoke_local_ability("skill.list", /' \
  "$SB/src/cli/commands/skill.rs"

set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-state-read-subject-boundary.sh
) >/tmp/check-runtime-state-read-subject-boundary-skill.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "skill.list read regression should exit 1 (got $rc)"

perl -0pi -e 's/\Qinvoke_local_ability("skill.list", \E/LocalRuntimeSkillCatalogueReadIssuer::list_installed_skills(/' \
  "$SB/src/cli/commands/skill.rs"

perl -0pi -e 's/\QLocalRuntimeApiKeyInventoryReadIssuer::list_api_keys(&ability, serde_json::json!({})\E/invoke_local_ability(&ability, serde_json::json!({})/' \
  "$SB/src/cli/commands/api_key_cli.rs"

set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-state-read-subject-boundary.sh
) >/tmp/check-runtime-state-read-subject-boundary-api-key.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "api-key list read regression should exit 1 (got $rc)"

echo "test_check_runtime_state_read_subject_boundary.sh: all cases passed"

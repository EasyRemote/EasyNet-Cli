use std::path::{Component, Path, PathBuf};

use crate::daemon::ability::names::{
    agents, automation, device_control, federation, governance, integrations, resources,
};

/// Canonical root for daemon-owned AbilityDescriptor TOMLs.
///
/// The helper owns the descriptor root so production code does not concatenate
/// the path directly. Future grouping under this root remains transparent to
/// callers that use this module.
pub const SYSTEM_ABILITY_DESCRIPTOR_ROOT: &str = "ability-descriptors/system";

/// Clean-final product group for a daemon-owned system AbilityDescriptor.
///
/// What this is: the source-of-truth mapping from a stable public ability name
/// to its contract directory under `ability-descriptors/system`.
/// What this is NOT: runtime ontology, dispatch ownership, or an Ability owner
/// classifier. The enum exists only to place descriptor contract files without
/// leaking a flat filesystem assumption into callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemAbilityDescriptorGroup {
    /// Agent interaction and hosted-agent lifecycle surfaces.
    Agents,
    /// Host/device control and device ability management surfaces.
    DeviceControl,
    /// EasyNet resource surfaces: skills, context, pages, media, and voice.
    Resources,
    /// Mission, schedule, loop, discussion, and orchestration surfaces.
    Automation,
    /// External protocol and plugin integration surfaces.
    Integrations,
    /// Safety, audit, observation, and meta-governance surfaces.
    Governance,
}

impl SystemAbilityDescriptorGroup {
    /// Return the directory name used below `SYSTEM_ABILITY_DESCRIPTOR_ROOT`.
    ///
    /// The names mirror `docs/spec/project-structure-v1.md` and are stable
    /// contract paths, not Rust module names.
    #[must_use]
    pub const fn directory_name(self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::DeviceControl => "device_control",
            Self::Resources => "resources",
            Self::Automation => "automation",
            Self::Integrations => "integrations",
            Self::Governance => "governance",
        }
    }

    /// Classify one stable public ability name into its descriptor group.
    ///
    /// The mapping is deliberately explicit instead of prefix-driven. Several
    /// ability families cross lexical prefixes (`meta.*`, `ability.*`,
    /// `node.*`), and descriptor placement must follow product ownership from
    /// the project-structure spec rather than string shape.
    pub fn for_ability_name(ability_name: &str) -> Result<Self, DescriptorPathError> {
        validate_ability_descriptor_name(ability_name)?;
        match ability_name {
            agents::CHAT
            | agents::DISCOVER
            | agents::INVOKE
            | agents::AGENT_LIST
            | agents::AGENT_START
            | agents::AGENT_STOP
            | agents::AGENT_REFRESH
            | agents::CHAT_HISTORY_LIST
            | agents::CHAT_HISTORY_GET => Ok(Self::Agents),

            device_control::BROWSER_OPEN_SESSION
            | device_control::BROWSER_SEND_INPUT
            | device_control::BROWSER_CAPTURE_VIEWPORT
            | device_control::BROWSER_CLOSE_SESSION
            | device_control::BROWSER_ATTACH_SESSION
            | device_control::FS_READ
            | device_control::FS_WRITE
            | device_control::FS_STAT
            | device_control::FS_LIST
            | device_control::FS_EDIT
            | device_control::FS_TRANSFER
            | device_control::HTTP_REQUEST
            | device_control::PROCESS_EXEC
            | device_control::SHELL_RUN
            | device_control::SESSION_LIST
            | device_control::SESSION_ATTACH
            | device_control::SESSION_OPEN
            | device_control::TERMINAL_ATTACH
            | device_control::TERMINAL_CREATE
            | device_control::TERMINAL_LIST
            | device_control::TERMINAL_CLOSE
            | device_control::TERMINAL_INPUT
            | device_control::TERMINAL_READ
            | device_control::TERMINAL_RESIZE
            | federation::NODE_LIST
            | federation::NODE_DESCRIBE
            | federation::NODE_REMOVE
            | federation::ABILITY_DEPLOY
            | federation::ABILITY_UNINSTALL
            | federation::ABILITY_PUBLISH
            | federation::ABILITY_UNPUBLISH => Ok(Self::DeviceControl),

            resources::CONTEXT_CLIPBOARD_LIST
            | resources::CONTEXT_CLIPBOARD_GET
            | resources::CONTEXT_CLIPBOARD_TRACK
            | resources::CONTEXT_CLIPBOARD_REMOVE
            | resources::CONTEXT_FOLDERS_LIST
            | resources::CONTEXT_FS_LIST
            | resources::CONTEXT_FAVORITES_LIST
            | resources::CONTEXT_FAVORITES_ADD
            | resources::CONTEXT_FAVORITES_REMOVE
            | resources::CONTEXT_CAPTURES_LIST
            | resources::CONTEXT_CAPTURES_GET
            | resources::MEDIA_MIC_SUBSCRIBE
            | resources::MEDIA_CAMERA_SUBSCRIBE
            | resources::MEDIA_CAMERA_SNAPSHOT
            | resources::MEDIA_CAMERA_RECORD_START
            | resources::MEDIA_CAMERA_RECORD_STOP
            | resources::MEDIA_SCREEN_SUBSCRIBE
            | resources::MEDIA_SCREEN_SNAPSHOT
            | resources::MEDIA_SPEAKER_PUBLISH
            | resources::META_LIST_RESOURCES
            | resources::SKILL_INSTALL
            | resources::SKILL_REMOVE
            | resources::SKILL_UPGRADE
            | resources::SKILL_PUBLISH
            | resources::SKILL_UNPUBLISH
            | resources::SKILL_LIST
            | resources::SKILL_TREE
            | resources::SKILL_READ_FILE
            | resources::SKILL_WRITE_FILE
            | resources::VOICE_CREATE_CALL
            | resources::VOICE_SHOW_CALL
            | resources::VOICE_JOIN_CALL
            | resources::VOICE_LEAVE_CALL
            | resources::VOICE_END_CALL
            | resources::VOICE_WATCH_CALL
            | resources::VOICE_REPORT_METRICS
            | resources::VOICE_LIST_CALLS
            | resources::VOICE_SUBSCRIBE
            | resources::VOICE_TRANSCRIBE => Ok(Self::Resources),

            automation::DISCUSS_CREATE
            | automation::DISCUSS_POST
            | automation::DISCUSS_SUBSCRIBE
            | automation::DISCUSS_LIST_TURNS
            | automation::LOOP_CREATE
            | automation::LOOP_STATUS
            | automation::LOOP_SUBSCRIBE
            | automation::LOOP_CANCEL
            | automation::MISSION_RUN
            | automation::MISSION_TRACK
            | automation::MISSION_CANCEL
            | automation::MISSION_DISCUSS_ROUND
            | automation::MISSION_THINK
            | automation::SCHEDULE_ADD
            | automation::SCHEDULE_LIST
            | automation::SCHEDULE_REMOVE
            | automation::SCHEDULE_ENABLE => Ok(Self::Automation),

            integrations::A2A_BRIDGE_LIST_SKILLS
            | integrations::A2A_BRIDGE_SEND_TASK
            | integrations::A2A_CLIENT_SEND_TASK
            | integrations::MCP_BRIDGE_LIST_TOOLS
            | integrations::MCP_BRIDGE_CALL_TOOL
            | integrations::MCP_CLIENT_LIST
            | integrations::MCP_CLIENT_CALL
            | integrations::OPENAI_CHAT_COMPLETIONS
            | integrations::OPENAI_LIST_MODELS
            | integrations::PLUGIN_RELOAD
            | integrations::PLUGIN_STATUS
            | integrations::PLUGIN_ACTIVATE_REALTIME
            | federation::RUNTIME_INVOKE_REMOTE => Ok(Self::Integrations),

            governance::ADMIN_STATUS
            | governance::OBSERVE_HEALTH
            | governance::OBSERVE_NETWORK_HEALTH
            | governance::SYSTEM_WATCH_BOOT
            | governance::CONSENT_SUBSCRIBE
            | governance::CONSENT_DECIDE
            | governance::CONSENT_LIST_PENDING
            | governance::INVOCATION_HISTORY_LIST
            | governance::INVOCATION_HISTORY_GET
            | governance::INVOCATION_HISTORY_PATH
            | governance::INVOCATION_RECORD_GET
            | governance::INVOCATION_TRACE_GET
            | governance::META_ACQUIRE
            | governance::META_DESCRIBE
            | governance::META_FORGET
            | governance::META_LIST_ABILITIES
            | governance::META_TEACH
            | federation::IDENTITY_LIST_USER_PUBKEYS
            | federation::IDENTITY_REGISTER_PUBKEY
            | federation::IDENTITY_REVOKE_USER_PUBKEY => Ok(Self::Governance),

            other => Err(DescriptorPathError::UnknownSystemAbility(other.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DescriptorPathError {
    #[error("system ability descriptor name must not be empty")]
    EmptyName,
    #[error("system ability descriptor name must be relative and contain no path separators: {0}")]
    UnsafeName(String),
    #[error("unknown daemon system ability descriptor name: {0}")]
    UnknownSystemAbility(String),
}

/// Return the canonical root directory for daemon-owned system descriptors.
///
/// The returned path is relative to the repository or installed contract root.
#[must_use]
pub fn system_ability_descriptor_root() -> PathBuf {
    PathBuf::from(SYSTEM_ABILITY_DESCRIPTOR_ROOT)
}

/// Resolve a stable public ability name to its grouped descriptor path.
///
/// The helper owns product-module grouping. Callers must not join
/// `SYSTEM_ABILITY_DESCRIPTOR_ROOT` with a flat file name themselves.
pub fn try_system_ability_descriptor_path(
    ability_name: &str,
) -> Result<PathBuf, DescriptorPathError> {
    let group = SystemAbilityDescriptorGroup::for_ability_name(ability_name)?;
    Ok(system_ability_descriptor_root()
        .join(group.directory_name())
        .join(format!("{ability_name}.ability.toml")))
}

/// Resolve a stable public ability name to its grouped descriptor path.
///
/// Panics when called with an unsafe or unknown ability name. The infallible
/// wrapper is for generator and test code that iterates `published_system_abilities`;
/// user input should call `try_system_ability_descriptor_path`.
#[must_use]
pub fn system_ability_descriptor_path(ability_name: &str) -> PathBuf {
    try_system_ability_descriptor_path(ability_name)
        .unwrap_or_else(|error| panic!("invalid system ability descriptor path: {error}"))
}

/// Iterate current system descriptor files without assuming future grouping.
///
/// Missing roots produce an empty iterator; callers that require the directory
/// to exist should validate `system_ability_descriptor_root()` first.
pub fn iter_system_ability_descriptor_paths() -> impl Iterator<Item = PathBuf> {
    let mut paths = Vec::new();
    collect_descriptor_paths(&system_ability_descriptor_root(), &mut paths);
    paths.sort();
    paths.into_iter()
}

fn collect_descriptor_paths(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_descriptor_paths(&path, out);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".ability.toml"))
        {
            out.push(path);
        }
    }
}

fn validate_ability_descriptor_name(name: &str) -> Result<(), DescriptorPathError> {
    if name.trim().is_empty() {
        return Err(DescriptorPathError::EmptyName);
    }
    if name != name.trim() || name.contains('/') || name.contains('\\') {
        return Err(DescriptorPathError::UnsafeName(name.to_string()));
    }
    let path = Path::new(name);
    let mut components = path.components();
    if matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() {
        Ok(())
    } else {
        Err(DescriptorPathError::UnsafeName(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_path_uses_clean_final_group() {
        assert_eq!(
            system_ability_descriptor_path("observe.health"),
            PathBuf::from("ability-descriptors/system/governance/observe.health.ability.toml")
        );
    }

    #[test]
    fn descriptor_group_classifies_cross_prefix_abilities_by_owner() {
        assert_eq!(
            SystemAbilityDescriptorGroup::for_ability_name("meta.list_resources").unwrap(),
            SystemAbilityDescriptorGroup::Resources
        );
        assert_eq!(
            SystemAbilityDescriptorGroup::for_ability_name("meta.list_abilities").unwrap(),
            SystemAbilityDescriptorGroup::Governance
        );
        assert_eq!(
            SystemAbilityDescriptorGroup::for_ability_name("ability.deploy").unwrap(),
            SystemAbilityDescriptorGroup::DeviceControl
        );
        assert_eq!(
            SystemAbilityDescriptorGroup::for_ability_name("openai.chat_completions").unwrap(),
            SystemAbilityDescriptorGroup::Integrations
        );
    }

    #[test]
    fn descriptor_path_rejects_path_escape() {
        assert!(try_system_ability_descriptor_path("../x").is_err());
        assert!(try_system_ability_descriptor_path("group/x").is_err());
        assert!(try_system_ability_descriptor_path(" observe.health").is_err());
    }

    #[test]
    fn descriptor_path_rejects_unknown_system_name() {
        assert!(matches!(
            try_system_ability_descriptor_path("unknown.ability"),
            Err(DescriptorPathError::UnknownSystemAbility(_))
        ));
    }
}

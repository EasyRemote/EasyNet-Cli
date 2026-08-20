//! Device profile — RFC-001 §1.
//!
//! Empty migration cursor for retired direct Device-owned daemon-local rows.
//! This is not an actor identity and cannot emit `AbilityDescriptor` values.
//! The physical Device hosts resources and the daemon; device-native callable
//! surfaces are emitted by named device-sponsored SystemAgent profiles.
//!
//! The empty cursor may remain for migration/high-water bookkeeping. Its live
//! descriptor inventory is unconditionally empty.
//!
//! Per RFC §A4: "device" is an implementation profile, NOT a protocol type.
//! The projection has no `kind` field on the wire.
//!
/// Return the intentionally empty live inventory for the retired Device
/// projection. Device-sponsored callable behavior belongs to SystemAgents.
pub fn descriptors_for(
    _owner_ura: &str,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_profile_excludes_migrated_system_agent_families() {
        // Device is the execution substrate, not the public owner for
        // device-native agent, terminal, session, node-management, locomotion,
        // skill-management, context, media, plugin lifecycle,
        // automation/orchestration, runtime-governance, runtime-health,
        // runtime-introspection, descriptor-transfer, ability-management,
        // openai-compat, or A2A integration descriptors.
        let descriptors = descriptors_for("easynet:///r/acme/device/01DEV");
        let names = descriptors
            .iter()
            .map(|d| d.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for migrated in [
            crate::daemon::ability::names::agents::AGENT_LIST,
            crate::daemon::ability::names::agents::AGENT_START,
            crate::daemon::ability::names::agents::AGENT_STOP,
            crate::daemon::ability::names::agents::AGENT_PURGE,
            crate::daemon::ability::names::agents::AGENT_REFRESH,
            crate::daemon::ability::names::agents::AGENT_ABILITY_PUT,
            crate::daemon::ability::names::agents::CHAT_HISTORY_LIST,
            crate::daemon::ability::names::agents::CHAT_HISTORY_GET,
            crate::daemon::ability::names::device_control::TERMINAL_CREATE,
            crate::daemon::ability::names::device_control::TERMINAL_LIST,
            crate::daemon::ability::names::device_control::TERMINAL_CLOSE,
            crate::daemon::ability::names::device_control::TERMINAL_ATTACH,
            crate::daemon::ability::names::device_control::TERMINAL_INPUT,
            crate::daemon::ability::names::device_control::TERMINAL_READ,
            crate::daemon::ability::names::device_control::TERMINAL_RESIZE,
            crate::daemon::ability::names::device_control::SESSION_LIST,
            crate::daemon::ability::names::device_control::SESSION_ATTACH,
            crate::daemon::ability::names::device_control::NODE_DESCRIBE,
            crate::daemon::ability::names::device_control::NODE_REMOVE,
            crate::daemon::ability::names::device_control::FS_READ,
            crate::daemon::ability::names::device_control::FS_WRITE,
            crate::daemon::ability::names::device_control::FS_STAT,
            crate::daemon::ability::names::device_control::FS_LIST,
            crate::daemon::ability::names::device_control::FS_EDIT,
            crate::daemon::ability::names::device_control::FS_TRANSFER,
            crate::daemon::ability::names::device_control::PROCESS_EXEC,
            crate::daemon::ability::names::device_control::SHELL_RUN,
            crate::daemon::ability::names::device_control::HTTP_REQUEST,
            crate::daemon::ability::names::resources::SKILL_INSTALL,
            crate::daemon::ability::names::resources::SKILL_REMOVE,
            crate::daemon::ability::names::resources::SKILL_UPGRADE,
            crate::daemon::ability::names::resources::SKILL_PUBLISH,
            crate::daemon::ability::names::resources::SKILL_UNPUBLISH,
            crate::daemon::ability::names::resources::SKILL_LIST,
            crate::daemon::ability::names::resources::SKILL_TREE,
            crate::daemon::ability::names::resources::SKILL_READ_FILE,
            crate::daemon::ability::names::resources::SKILL_WRITE_FILE,
            crate::daemon::ability::names::resources::CONTEXT_CLIPBOARD_LIST,
            crate::daemon::ability::names::resources::CONTEXT_CLIPBOARD_GET,
            crate::daemon::ability::names::resources::CONTEXT_CLIPBOARD_TRACK,
            crate::daemon::ability::names::resources::CONTEXT_CLIPBOARD_REMOVE,
            crate::daemon::ability::names::resources::CONTEXT_CATALOG,
            crate::daemon::ability::names::resources::CONTEXT_FOLDERS_LIST,
            crate::daemon::ability::names::resources::CONTEXT_FS_LIST,
            crate::daemon::ability::names::resources::CONTEXT_FAVORITES_LIST,
            crate::daemon::ability::names::resources::CONTEXT_FAVORITES_ADD,
            crate::daemon::ability::names::resources::CONTEXT_FAVORITES_REMOVE,
            crate::daemon::ability::names::resources::CONTEXT_CAPTURES_LIST,
            crate::daemon::ability::names::resources::CONTEXT_CAPTURES_GET,
            crate::daemon::ability::names::resources::RESOURCE_REFRESH_REMOTE_TARGETS,
            crate::daemon::ability::names::resources::RESOURCE_WATCH_REMOTE_TARGETS,
            crate::daemon::ability::names::integrations::PLUGIN_RELOAD,
            crate::daemon::ability::names::integrations::PLUGIN_STATUS,
            crate::daemon::ability::names::integrations::PLUGIN_ACTIVATE_REALTIME,
            crate::daemon::ability::names::integrations::PLUGIN_COMPANION_STATUS,
            crate::daemon::ability::names::integrations::PLUGIN_COMPANION_RECONCILE,
            crate::daemon::ability::names::automation::DISCUSS_CREATE,
            crate::daemon::ability::names::automation::DISCUSS_POST,
            crate::daemon::ability::names::automation::DISCUSS_LIST_TURNS,
            crate::daemon::ability::names::automation::DISCUSS_SUBSCRIBE,
            crate::daemon::ability::names::automation::LOOP_CREATE,
            crate::daemon::ability::names::automation::LOOP_STATUS,
            crate::daemon::ability::names::automation::LOOP_SUBSCRIBE,
            crate::daemon::ability::names::automation::LOOP_CANCEL,
            crate::daemon::ability::names::automation::MISSION_RUN,
            crate::daemon::ability::names::automation::MISSION_TRACK,
            crate::daemon::ability::names::automation::MISSION_CANCEL,
            crate::daemon::ability::names::automation::MISSION_THINK,
            crate::daemon::ability::names::automation::MISSION_DISCUSS_ROUND,
            crate::daemon::ability::names::automation::SCHEDULE_ADD,
            crate::daemon::ability::names::automation::SCHEDULE_LIST,
            crate::daemon::ability::names::automation::SCHEDULE_REMOVE,
            crate::daemon::ability::names::automation::SCHEDULE_ENABLE,
            crate::daemon::ability::names::governance::AUTHORITY_BINDING_GRANT,
            crate::daemon::ability::names::governance::AUTHORITY_BINDING_REVOKE,
            crate::daemon::ability::names::governance::AUTHORITY_BINDING_LIST,
            crate::daemon::ability::names::governance::AUTHORITY_BINDING_CHECK,
            crate::daemon::ability::names::governance::POLICY_REQUEST_CREATE,
            crate::daemon::ability::names::governance::POLICY_REQUEST_RESOLVE,
            crate::daemon::ability::names::governance::POLICY_REQUEST_LIST,
            crate::daemon::ability::names::governance::ADMISSION_EXPLAIN,
            crate::daemon::ability::names::governance::OBSERVE_HEALTH,
            crate::daemon::ability::names::governance::OBSERVE_NETWORK_HEALTH,
            crate::daemon::ability::names::governance::ADMIN_STATUS,
            crate::daemon::ability::names::governance::META_DESCRIBE,
            crate::daemon::ability::names::governance::META_LIST_ABILITIES,
            crate::daemon::ability::names::governance::META_TEACH,
            crate::daemon::ability::names::governance::META_ACQUIRE,
            crate::daemon::ability::names::governance::META_FORGET,
            crate::daemon::ability::names::resources::META_LIST_RESOURCES,
            crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST,
            crate::daemon::ability::names::governance::INVOCATION_HISTORY_GET,
            crate::daemon::ability::names::governance::INVOCATION_HISTORY_PATH,
            crate::daemon::ability::names::governance::INVOCATION_RECORD_GET,
            crate::daemon::ability::names::governance::INVOCATION_TRACE_GET,
            crate::daemon::ability::names::governance::INVOCATION_CANCEL,
            crate::daemon::ability::names::federation::ABILITY_DEPLOY,
            crate::daemon::ability::names::federation::ABILITY_UNINSTALL,
            crate::daemon::ability::names::federation::ABILITY_PUBLISH,
            crate::daemon::ability::names::federation::ABILITY_UNPUBLISH,
            crate::daemon::ability::names::integrations::OPENAI_CHAT_COMPLETIONS,
            crate::daemon::ability::names::integrations::OPENAI_LIST_MODELS,
            crate::daemon::ability::names::integrations::OPENAI_FILES_UPLOAD,
            crate::daemon::ability::names::integrations::OPENAI_FILES_RETRIEVE,
            crate::daemon::ability::names::integrations::OPENAI_FILES_DELETE,
            crate::daemon::ability::names::integrations::A2A_BRIDGE_LIST_SKILLS,
            crate::daemon::ability::names::integrations::A2A_BRIDGE_SEND_TASK,
            crate::daemon::ability::names::integrations::A2A_CLIENT_SEND_TASK,
            crate::daemon::ability::names::resources::CONTEXT_CAPTURES_READ,
            crate::daemon::ability::names::resources::MEDIA_MIC_SUBSCRIBE,
            crate::daemon::ability::names::resources::MEDIA_CAMERA_SUBSCRIBE,
            crate::daemon::ability::names::resources::MEDIA_CAMERA_SNAPSHOT,
            crate::daemon::ability::names::resources::MEDIA_CAMERA_RECORD_START,
            crate::daemon::ability::names::resources::MEDIA_CAMERA_RECORD_STOP,
            crate::daemon::ability::names::resources::MEDIA_SCREEN_SUBSCRIBE,
            crate::daemon::ability::names::resources::MEDIA_SCREEN_SNAPSHOT,
            crate::daemon::ability::names::resources::MEDIA_SPEAKER_PUBLISH,
        ] {
            assert!(
                !names.contains(migrated),
                "{migrated} is owned by a device-sponsored SystemAgent, not direct Device"
            );
        }
    }

    #[test]
    fn direct_device_owner_inventory_is_explicit() {
        let actual = descriptors_for("easynet:///r/acme/device/01DEV")
            .into_iter()
            .map(|descriptor| descriptor.name)
            .collect::<Vec<_>>();
        let expected: Vec<String> = Vec::new();
        assert_eq!(
            actual, expected,
            "direct Device-owner ability inventory changed; migrate the family to a SystemAgent or document a bootstrap/self-maintenance exception"
        );
    }

    #[test]
    fn descriptors_for_does_not_steal_sub_profile_abilities() {
        let descriptors = descriptors_for("easynet:///r/acme/device/01DEV");
        let names = descriptors
            .iter()
            .map(|d| d.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        for name in [
            "consent.subscribe",
            "mcp.bridge.call_tool",
            "conversation.send",
            "federation.join",
            "voice.create_call",
            "voice.subscribe",
        ] {
            assert!(
                !names.contains(name),
                "{name} is not OwnerKind::DeviceProfileProjection and must not be described by the device profile"
            );
        }
    }

    #[test]
    fn descriptors_for_excludes_local_companion_control() {
        let descriptors = descriptors_for("easynet:///r/acme/device/01DEV");
        let names = descriptors
            .iter()
            .map(|d| d.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        for name in ["plugin.companion_status", "plugin.companion_reconcile"] {
            assert!(
                !names.contains(name),
                "{name} is daemon-local companion control and must not be remotely advertised"
            );
        }
    }

    #[test]
    fn descriptors_for_emit_only_owned_names() {
        let owner = "easynet:///r/acme/device/01DEV";
        let descriptors = descriptors_for(owner);
        assert!(
            descriptors.is_empty(),
            "Device profile is a migration projection; no public direct Device descriptors should remain"
        );
        for d in &descriptors {
            assert_eq!(d.owner_ura, owner);
            assert_eq!(d.source, "kernel:built-in");
        }
        assert!(
            descriptors
                .iter()
                .all(|d| d.name != crate::daemon::ability::names::resources::META_LIST_RESOURCES),
            "meta.list_resources is owned by runtime-introspection, not direct Device"
        );
        assert!(
            descriptors.iter().all(|d| d.name
                != crate::daemon::ability::names::resources::RESOURCE_REFRESH_REMOTE_TARGETS),
            "resource.refresh_remote_targets is owned by media SystemAgent, not direct Device"
        );
        assert!(
            descriptors.iter().all(|d| d.name
                != crate::daemon::ability::names::resources::RESOURCE_WATCH_REMOTE_TARGETS),
            "resource.watch_remote_targets is owned by media SystemAgent, not direct Device"
        );
    }

    #[test]
    fn descriptors_for_marks_remaining_direct_device_rows_scoped() {
        use crate::daemon::ability::descriptors::Visibility;
        let descriptors = descriptors_for("easynet:///r/acme/device/01DEV");
        for d in descriptors {
            assert_eq!(
                d.visibility,
                Visibility::Scoped,
                "{} must be SCOPED",
                d.name
            );
        }
    }
}

// EasyNet CLI — remote desktop resource subject boundary
// ======================================================
//
// File: plugins/remote-desktop/src/resource.rs
// Description: Resource-subject resolution for remote desktop abilities.

use serde_json::Value;

use crate::daemon::ability::builtins::resources::media::resource_subject::{
    resolve_required_resource_subject, ResourceSubjectSpec,
};
use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::persistence::resources::{ResourceEntry, ResourceType};

pub(in crate::daemon::plugins::remote_desktop) fn resolve_screen_resource_from_envelope(
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
) -> anyhow::Result<ResourceEntry> {
    resolve_required_resource_subject(env, args, screen_resource_subject_spec(ability))
}

fn screen_resource_subject_spec(ability: &str) -> ResourceSubjectSpec<'_> {
    ResourceSubjectSpec {
        ability,
        required_subject: "a display/window/application",
        allowed_kinds: &[
            ResourceType::Display,
            ResourceType::Application,
            ResourceType::Window,
        ],
        allowed_label: "display/application/window",
    }
}

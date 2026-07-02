// EasyNet CLI — media resource subject resolver
// =============================================
//
// Shared envelope-subject validation for media and media-adjacent
// device plugins. The AXIOM rule is one rule, so handlers should not
// each hand-roll a slightly different `subject` parser.

use serde_json::Value;

use crate::persistence::resources::{self, lookup_by_ura, ResourceEntry, ResourceType};
use crate::runtime::ability_dispatch::EnvelopeContext;
use crate::ura::URAKind;

pub const REASON_SUBJECT_REQUIRED: &str = "subject_required";
pub const REASON_SUBJECT_IN_ARGS: &str = "subject_in_args";
pub const REASON_RESOURCE_NOT_FOUND: &str = "resource_not_found";
pub const REASON_RESOURCE_TABLE_UNAVAILABLE: &str = "resource_table_unavailable";
pub const REASON_RESOURCE_TYPE_MISMATCH: &str = "resource_type_mismatch";

/// Contract for resolving one envelope subject into a resource row.
pub struct ResourceSubjectSpec<'a> {
    pub ability: &'a str,
    pub required_subject: &'a str,
    pub allowed_kinds: &'a [ResourceType],
    pub allowed_label: &'a str,
}

/// Reject callers that try to pass AXIOM `subject` through JSON args.
pub fn reject_subject_in_args(ability: &str, args: &Value) -> anyhow::Result<()> {
    if let Value::Object(map) = args {
        if map.contains_key("subject") {
            anyhow::bail!(
                "{ability}: `subject` MUST come from the invocation envelope, not args; reason={REASON_SUBJECT_IN_ARGS}"
            );
        }
    }
    Ok(())
}

/// Resolve the envelope `subject` to a typed local resource entry.
pub fn resolve_required_resource_subject(
    env: &EnvelopeContext,
    args: &Value,
    spec: ResourceSubjectSpec<'_>,
) -> anyhow::Result<ResourceEntry> {
    reject_subject_in_args(spec.ability, args)?;
    let subject =
        require_resource_ura_subject(spec.ability, Some(env.subject()), spec.required_subject)?;
    resolve_resource_ura_subject(subject, spec)
}

/// Resolve an already-bound resource URA through the same type gate.
pub fn resolve_resource_ura_subject(
    subject: &str,
    spec: ResourceSubjectSpec<'_>,
) -> anyhow::Result<ResourceEntry> {
    let subject = require_resource_ura_subject(spec.ability, Some(subject), spec.required_subject)?;
    let file = resources::load().map_err(|err| {
        anyhow::anyhow!(
            "{}: local resources table could not be loaded; \
             reason={REASON_RESOURCE_TABLE_UNAVAILABLE}; source={err}",
            spec.ability
        )
    })?;
    let entry = lookup_by_ura(&file, subject).ok_or_else(|| {
        anyhow::anyhow!(
            "{}: subject {subject} not found in local resources table; reason={REASON_RESOURCE_NOT_FOUND}",
            spec.ability
        )
    })?;
    if !spec.allowed_kinds.contains(&entry.kind) {
        anyhow::bail!(
            "{}: subject {subject} resolves to {}, not {}; reason={REASON_RESOURCE_TYPE_MISMATCH}",
            spec.ability,
            entry.kind.as_str(),
            spec.allowed_label
        );
    }
    Ok(entry.clone())
}

/// Validate and return the envelope subject as a resource URA.
///
/// Resource-scoped abilities are not allowed to fall back to the
/// caller/callee/device subject that local dispatch may provide. Only a
/// canonical `URAKind::Resource` subject means the caller selected a concrete
/// media resource.
pub fn require_resource_ura_subject<'a>(
    ability: &str,
    subject: Option<&'a str>,
    required_subject: &str,
) -> anyhow::Result<&'a str> {
    let subject = subject
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .ok_or_else(|| subject_required_error(ability, required_subject))?;
    if !is_resource_ura_subject(subject) {
        return Err(subject_required_error(ability, required_subject));
    }
    Ok(subject)
}

/// True only for canonical resource URAs. Invalid URAs and non-resource URAs
/// are both false because neither can identify a media resource.
pub fn is_resource_ura_subject(subject: &str) -> bool {
    crate::ura::parse_ura(subject.trim())
        .map(|parsed| parsed.kind == URAKind::Resource)
        .unwrap_or(false)
}

fn subject_required_error(ability: &str, required_subject: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{ability}: subject required (resource_ura of {required_subject}); reason={REASON_SUBJECT_REQUIRED}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::test_support::HomeGuard;

    #[test]
    fn corrupt_resources_table_is_not_reported_as_resource_not_found() {
        let _home = HomeGuard::new();
        let path = resources::path();
        std::fs::create_dir_all(path.parent().expect("resources path has parent"))
            .expect("create state dir");
        std::fs::write(&path, b"{not-json").expect("write corrupt resources table");

        let err = resolve_resource_ura_subject(
            "easynet:///r/acme/resource/device.01DEV/streams/display.01",
            ResourceSubjectSpec {
                ability: "device.test.media",
                required_subject: "display",
                allowed_kinds: &[ResourceType::Display],
                allowed_label: "display",
            },
        )
        .expect_err("corrupt resources table must surface as table load failure");

        let message = err.to_string();
        assert!(
            message.contains(REASON_RESOURCE_TABLE_UNAVAILABLE),
            "expected reason={REASON_RESOURCE_TABLE_UNAVAILABLE}; got: {message}"
        );
        assert!(
            !message.contains(REASON_RESOURCE_NOT_FOUND),
            "corrupt table must not be misreported as a missing resource: {message}"
        );
    }

    #[test]
    fn non_resource_subject_is_subject_required_not_resource_not_found() {
        let _home = HomeGuard::new();
        resources::save(&resources::ResourcesFile::default()).expect("save empty resources table");

        let err = resolve_resource_ura_subject(
            "easynet:///r/default/device/local",
            ResourceSubjectSpec {
                ability: "device.test.media",
                required_subject: "display",
                allowed_kinds: &[ResourceType::Display],
                allowed_label: "display",
            },
        )
        .expect_err("device subject must not be treated as a resource lookup");

        let message = err.to_string();
        assert!(
            message.contains(REASON_SUBJECT_REQUIRED),
            "expected reason={REASON_SUBJECT_REQUIRED}; got: {message}"
        );
        assert!(
            !message.contains(REASON_RESOURCE_NOT_FOUND),
            "non-resource subjects must not hit the resources table: {message}"
        );
    }
}

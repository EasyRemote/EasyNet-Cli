// EasyNet CLI - Ability owner projection grammar
// =================================================
//
// File: src/daemon/ability/owner_projection.rs
// Description: Single runtime-plane marker grammar for ability owner
//              projections stored in AuthorityScope/control-plane records.
//
// Protocol Responsibility
// -----------------------
// `OwnerProjection` is not a protocol principal and not an execution host. It is
// the daemon-local string grammar that labels which owner plane a governed
// AbilityDescriptor belongs to: DeviceProfileProjection migration facts,
// realm Authority, device-sponsored SystemAgent, hosted Agent, or plugin
// implementation plane.
//
// Implementation Approach
// -----------------------
// Keep parsing, validation, and canonical rendering in one value object so
// AuthorityScope, dispatch registration, and tests cannot maintain independent
// `device` / `authority` / `<plane>:<id>` tables.
//
// Usage Contract
// --------------
// Callers that need runtime ownership convert between `OwnerProjection` and
// `OwnerKind` at the dispatch boundary. This module deliberately does not decide
// authority support or host custody.

use crate::daemon::ability::AbilityControlPlaneError;

/// Canonical owner-plane marker grammar for an `AuthorityScope`.
///
/// The owner projection is a runtime-plane label, never a product deployment
/// mode. It has exactly five shapes: two bare planes (`device`, `authority`)
/// and three `<plane>:<id>` planes (`system-agent`, `agent`, `plugin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OwnerProjection {
    Device,
    RealmAuthority,
    SystemAgent(String),
    Agent(String),
    Plugin(String),
}

impl OwnerProjection {
    /// Parse a trimmed owner-plane marker. The caller is responsible for
    /// trimming; this method rejects anything that is not one of the canonical
    /// shapes, including a present-but-empty `<plane>:` id.
    pub(crate) fn parse(marker: &str) -> Result<Self, AbilityControlPlaneError> {
        let invalid = || AbilityControlPlaneError::InvalidAuthorityOwnerProjection {
            projection: marker.to_string(),
        };
        match marker {
            "device" => Ok(Self::Device),
            "authority" => Ok(Self::RealmAuthority),
            _ => {
                let (plane, id) = marker.split_once(':').ok_or_else(invalid)?;
                if !is_valid_owner_projection_id(id) {
                    return Err(invalid());
                }
                let id = id.to_string();
                match plane {
                    "system-agent" => Ok(Self::SystemAgent(id)),
                    "agent" => Ok(Self::Agent(id)),
                    "plugin" => Ok(Self::Plugin(id)),
                    _ => Err(invalid()),
                }
            }
        }
    }

    /// Re-render the canonical marker. This always round-trips `parse`.
    pub(crate) fn canonical(&self) -> String {
        match self {
            Self::Device => "device".to_string(),
            Self::RealmAuthority => "authority".to_string(),
            Self::SystemAgent(id) => format!("system-agent:{id}"),
            Self::Agent(id) => format!("agent:{id}"),
            Self::Plugin(id) => format!("plugin:{id}"),
        }
    }
}

/// A `<plane>:<id>` identifier segment must be present and must stay a
/// stable, unambiguous map key. The id charset stays deliberately permissive —
/// agent ids and plugin slugs flow through here — so only shapes that break the
/// marker as a key are rejected: an empty id, surrounding/interior whitespace,
/// control characters, or a further `:` that would make the plane ambiguous.
fn is_valid_owner_projection_id(id: &str) -> bool {
    !id.is_empty()
        && id == id.trim()
        && !id.contains(':')
        && !id.chars().any(char::is_whitespace)
        && !id.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_projection_round_trips_all_canonical_shapes() {
        for marker in [
            "device",
            "authority",
            "system-agent:agent-management",
            "agent:codex",
            "plugin:remote-desktop",
        ] {
            let projection = OwnerProjection::parse(marker)
                .unwrap_or_else(|err| panic!("{marker} must parse: {err}"));
            assert_eq!(projection.canonical(), marker);
        }
    }

    #[test]
    fn owner_projection_rejects_ambiguous_or_retired_markers() {
        for marker in [
            "",
            "hub",
            "system-agent:",
            "system-agent:agent management",
            "agent:codex:extra",
            "plugin:\nremote",
            "user:alice",
        ] {
            assert_eq!(
                OwnerProjection::parse(marker).unwrap_err(),
                AbilityControlPlaneError::InvalidAuthorityOwnerProjection {
                    projection: marker.to_string()
                }
            );
        }
    }
}

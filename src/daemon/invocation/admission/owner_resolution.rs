// EasyNet CLI — RFC-014 owner resolution
// =======================================
//
// Resolves the single accountable user owner before policy evaluation.

use super::decision::{OwnerResolution, OwnerSource};
use crate::core::ura::{parse_ura, user_ura, URAKind};
use crate::daemon::persistence::config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerFact {
    pub owner_user_id: Option<String>,
    pub owner_ura: Option<String>,
    pub authoritative: bool,
}

impl OwnerFact {
    #[must_use]
    pub fn user(owner_user_id: impl Into<String>, owner_ura: impl Into<String>) -> Self {
        Self {
            owner_user_id: Some(owner_user_id.into()),
            owner_ura: Some(owner_ura.into()),
            authoritative: true,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OwnerResolutionInput {
    pub subject: Option<OwnerFact>,
    pub callee: Option<OwnerFact>,
    pub device: Option<OwnerFact>,
    pub session: Option<OwnerFact>,
}

pub struct OwnerResolver;

/// Projects the locally paired device into the canonical accountable user.
///
/// This is the device's local identity source during the bootstrap window,
/// before the trust anchor has materialized the same owner fact.  Keeping the
/// projection here makes bootstrap authority and ordinary policy admission use
/// the same owner-resolution model.
#[must_use]
pub(crate) fn local_device_owner_fact(ura: &str) -> Option<OwnerFact> {
    let parsed = parse_ura(ura).ok()?;
    if parsed.kind != URAKind::Device {
        return None;
    }
    let credentials = config::load_credentials().ok()?;
    if parsed.realm != credentials.realm || parsed.device_id()? != credentials.node_id.as_str() {
        return None;
    }
    let owner_user_id = credentials.user_id().ok()?.to_string();
    Some(OwnerFact::user(
        owner_user_id.clone(),
        user_ura(&credentials.realm, &owner_user_id),
    ))
}

impl OwnerResolver {
    #[must_use]
    pub fn resolve(input: &OwnerResolutionInput) -> OwnerResolution {
        let ordered = [
            (OwnerSource::Subject, input.subject.as_ref()),
            (OwnerSource::Callee, input.callee.as_ref()),
            (OwnerSource::Device, input.device.as_ref()),
            (OwnerSource::Session, input.session.as_ref()),
        ];

        let authoritative: Vec<_> = ordered
            .iter()
            .filter_map(|(source, fact)| fact.filter(|f| f.authoritative).map(|f| (*source, f)))
            .collect();
        let mut audit_warnings = conflict_warnings(&authoritative);

        for (source, fact) in ordered {
            let Some(fact) = fact.filter(|f| f.authoritative) else {
                continue;
            };
            let Some(owner_user_id) = fact.owner_user_id.clone().filter(|v| !v.trim().is_empty())
            else {
                audit_warnings.push(format!(
                    "{} owner fact did not project to an accountable user principal",
                    source.as_str()
                ));
                return OwnerResolution {
                    owner_user_id: None,
                    owner_ura: fact.owner_ura.clone(),
                    owner_source: OwnerSource::Unresolved,
                    audit_warnings,
                };
            };
            return OwnerResolution {
                owner_user_id: Some(owner_user_id),
                owner_ura: fact.owner_ura.clone(),
                owner_source: source,
                audit_warnings,
            };
        }

        OwnerResolution::unresolved("no authoritative owner source was present")
    }
}

fn conflict_warnings(facts: &[(OwnerSource, &OwnerFact)]) -> Vec<String> {
    let mut warnings = Vec::new();
    for i in 0..facts.len() {
        for j in (i + 1)..facts.len() {
            let (left_source, left) = facts[i];
            let (right_source, right) = facts[j];
            if left.owner_user_id.is_some()
                && right.owner_user_id.is_some()
                && left.owner_user_id != right.owner_user_id
            {
                warnings.push(format!(
                    "owner conflict before precedence: {}={:?} {}={:?}",
                    left_source.as_str(),
                    left.owner_user_id,
                    right_source.as_str(),
                    right.owner_user_id
                ));
            }
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_owner_wins_over_lower_precedence_sources() {
        let input = OwnerResolutionInput {
            subject: Some(OwnerFact::user("alice", "easynet:///r/test/user/alice")),
            callee: Some(OwnerFact::user("bob", "easynet:///r/test/user/bob")),
            ..OwnerResolutionInput::default()
        };
        let got = OwnerResolver::resolve(&input);
        assert_eq!(got.owner_user_id.as_deref(), Some("alice"));
        assert_eq!(got.owner_source, OwnerSource::Subject);
        assert!(
            got.audit_warnings
                .iter()
                .any(|warning| warning.contains("owner conflict")),
            "conflicting lower-precedence owner must be auditable"
        );
    }

    #[test]
    fn device_owner_without_user_projection_is_unresolved() {
        let input = OwnerResolutionInput {
            device: Some(OwnerFact {
                owner_user_id: None,
                owner_ura: Some("easynet:///r/test/device/dev-a".to_string()),
                authoritative: true,
            }),
            ..OwnerResolutionInput::default()
        };
        let got = OwnerResolver::resolve(&input);
        assert_eq!(got.owner_source, OwnerSource::Unresolved);
        assert!(got.owner_user_id.is_none());
    }
}

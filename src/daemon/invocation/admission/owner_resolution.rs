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
pub(crate) fn local_device_owner_fact(ura: &str) -> anyhow::Result<Option<OwnerFact>> {
    let parsed = parse_ura(ura)
        .map_err(|error| anyhow::anyhow!("local device owner URA invalid: {ura}: {error}"))?;
    if parsed.kind != URAKind::Device {
        return Ok(None);
    }
    let Some(credentials) = config::load_credentials_optional()? else {
        return Ok(None);
    };
    let Some(device_id) = parsed.device_id() else {
        return Ok(None);
    };
    if parsed.realm != credentials.realm || device_id != credentials.node_id.as_str() {
        return Ok(None);
    }
    let owner_user_id = credentials.user_id()?.to_string();
    Ok(Some(OwnerFact::user(
        owner_user_id.clone(),
        user_ura(&credentials.realm, &owner_user_id),
    )))
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
    use crate::cli::commands::test_support::HomeGuard;
    use crate::daemon::persistence::config::{save_credentials, state_dir, Credentials};

    fn credentials() -> Credentials {
        Credentials {
            node_id: "dev-1".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "https://127.0.0.1:50443".to_string(),
            realm: "test".to_string(),
            deploy_signature: String::new(),
            hub_api_base: Some("http://127.0.0.1:8080".to_string()),
            username: Some("alice".to_string()),
            user_id: Some("alice".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: Some("join-hash".to_string()),
        }
    }

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

    #[test]
    fn local_device_owner_fact_returns_none_when_credentials_missing() {
        let _home = HomeGuard::new();

        let owner = local_device_owner_fact("easynet:///r/test/device/dev-1")
            .expect("missing credentials should be classified");

        assert_eq!(owner, None);
    }

    #[test]
    fn local_device_owner_fact_projects_saved_credentials() {
        let _home = HomeGuard::new();
        save_credentials(&credentials()).expect("save credentials");

        let owner = local_device_owner_fact("easynet:///r/test/device/dev-1")
            .expect("valid credentials should project")
            .expect("owner fact");

        assert_eq!(owner.owner_user_id.as_deref(), Some("alice"));
        assert_eq!(
            owner.owner_ura.as_deref(),
            Some("easynet:///r/test/user/alice")
        );
    }

    #[test]
    fn local_device_owner_fact_rejects_malformed_credentials() {
        let _home = HomeGuard::new();
        std::fs::create_dir_all(state_dir()).expect("create isolated state dir");
        std::fs::write(state_dir().join("credentials.json"), b"{")
            .expect("write malformed credentials");

        let error = local_device_owner_fact("easynet:///r/test/device/dev-1")
            .expect_err("malformed credentials must fail");

        let message = format!("{error:#}");
        assert!(
            message.contains("parse credentials"),
            "unexpected error: {message}"
        );
    }
}

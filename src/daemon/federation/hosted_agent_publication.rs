//! Hosted-Agent publication lifecycle domain.
//!
//! A Device does not choose a realm generation. It persists one opaque
//! incarnation id before network I/O, asks the Hub to bind that incarnation,
//! and may publish abilities only after the exact Hub assignment is durably
//! recorded. The id is an idempotency/lifecycle key, never authority.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize};

const INCARNATION_ID_HEX_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct HostedAgentIncarnationId(String);

impl<'de> Deserialize<'de> for HostedAgentIncarnationId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl HostedAgentIncarnationId {
    #[must_use]
    pub(crate) fn fresh() -> Self {
        Self(uuid::Uuid::new_v4().simple().to_string())
    }

    pub(crate) fn parse(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.len() != INCARNATION_ID_HEX_LEN
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(
                "hosted Agent incarnation_id must be exactly 32 lowercase hexadecimal characters"
                    .to_string(),
            );
        }
        Ok(Self(value))
    }

    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HostedAgentIncarnationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostedAgentGenerationAssignment {
    pub(crate) agent_ura: String,
    pub(crate) host_device_ura: String,
    pub(crate) incarnation_id: HostedAgentIncarnationId,
    pub(crate) generation: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostedAgentGenerationAssignmentWire {
    agent_ura: String,
    host_device_ura: String,
    incarnation_id: HostedAgentIncarnationId,
    generation: u64,
}

impl<'de> Deserialize<'de> for HostedAgentGenerationAssignment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = HostedAgentGenerationAssignmentWire::deserialize(deserializer)?;
        let assignment = Self {
            agent_ura: wire.agent_ura,
            host_device_ura: wire.host_device_ura,
            incarnation_id: wire.incarnation_id,
            generation: wire.generation,
        };
        assignment.validate().map_err(serde::de::Error::custom)?;
        Ok(assignment)
    }
}

impl HostedAgentGenerationAssignment {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let agent = crate::core::ura::parse_ura(self.agent_ura.trim())
            .map_err(|error| format!("assignment agent_ura is invalid: {error}"))?;
        let host = crate::core::ura::parse_ura(self.host_device_ura.trim())
            .map_err(|error| format!("assignment host_device_ura is invalid: {error}"))?;
        if agent.kind != crate::core::ura::URAKind::Agent
            || agent.agent_ids().is_none()
            || host.kind != crate::core::ura::URAKind::Device
            || agent.realm != host.realm
            || self.generation == 0
        {
            return Err("hosted Agent generation assignment has invalid geometry".to_string());
        }
        Ok(())
    }
}

/// One cohesive Device-side publication workflow shared by session prelude,
/// reconnect reconciliation, and hot Agent start. Construction crosses the
/// durable `RegistrationPending` boundary. Activation accepts only the exact
/// Hub assignment and persists it before creating an ability projection.
#[derive(Debug, Clone)]
pub(crate) struct HostedAgentPublicationPlan {
    assignment: HostedAgentAssignmentPlan,
    catalog_epoch: u64,
    descriptors: Vec<crate::daemon::ability::descriptors::AbilityDescriptor>,
}

/// Exact, durable Device-side input to the Hub generation allocator.
///
/// This smaller plan is intentionally independent of ability descriptors. A
/// committed stop/purge may need to replay only `federation.advertise_agent`
/// after a crash so it can learn the generation that must be tombstoned and
/// revoked. That recovery path must never regain an ability-publication
/// capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedAgentAssignmentPlan {
    agent_ura: String,
    host_device_ura: String,
    incarnation_id: HostedAgentIncarnationId,
}

impl HostedAgentAssignmentPlan {
    fn begin(agent_ura: &str, host_device_ura: &str) -> Result<Self, String> {
        let pending = crate::daemon::persistence::hosted_agent_publications::begin_registration(
            agent_ura,
            host_device_ura,
            now_unix_ms(),
        )
        .map_err(|error| format!("persist hosted Agent registration intent: {error}"))?;
        let incarnation_id = pending.incarnation_id().clone();
        Ok(Self {
            agent_ura: pending.agent_ura,
            host_device_ura: pending.host_device_ura,
            incarnation_id,
        })
    }

    /// Reconstruct the identity-only command for a committed retraction.
    /// Every fact must match the durable RegistrationPending record; callers
    /// cannot mint a replacement incarnation during recovery.
    pub(crate) fn resume_pending_retraction(
        agent_ura: &str,
        host_device_ura: &str,
        incarnation_id: &HostedAgentIncarnationId,
    ) -> Result<Self, String> {
        let record = crate::daemon::persistence::hosted_agent_publications::record_for(agent_ura)
            .map_err(|error| format!("load hosted Agent registration intent: {error:#}"))?
            .ok_or_else(|| {
                format!("hosted Agent `{agent_ura}` has no durable registration intent")
            })?;
        let exact_pending = record.host_device_ura == host_device_ura
            && matches!(
                &record.lifecycle,
                crate::daemon::persistence::hosted_agent_publications::HostedAgentDevicePublicationState::RegistrationPending {
                    incarnation_id: persisted,
                } if persisted == incarnation_id
            );
        if !exact_pending {
            return Err(format!(
                "hosted Agent `{agent_ura}` registration intent changed during retraction recovery"
            ));
        }
        Ok(Self {
            agent_ura: agent_ura.to_string(),
            host_device_ura: host_device_ura.to_string(),
            incarnation_id: incarnation_id.clone(),
        })
    }

    pub(crate) fn identity_payload_bytes(&self) -> Result<Vec<u8>, String> {
        crate::daemon::federation::advertise::advertise_agent_payload_bytes(
            &self.agent_ura,
            &self.incarnation_id,
        )
    }

    pub(crate) fn bind(
        self,
        assignment: HostedAgentGenerationAssignment,
    ) -> Result<HostedAgentGenerationAssignment, String> {
        if assignment.agent_ura != self.agent_ura
            || assignment.host_device_ura != self.host_device_ura
            || assignment.incarnation_id != self.incarnation_id
        {
            return Err("Hub assignment does not match the hosted Agent assignment plan".into());
        }
        crate::daemon::persistence::hosted_agent_publications::bind_assignment(
            &assignment,
            now_unix_ms(),
        )
        .map_err(|error| format!("persist hosted Agent generation assignment: {error}"))?;
        Ok(assignment)
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn agent_ura(&self) -> &str {
        &self.agent_ura
    }

    #[must_use]
    pub(crate) fn host_device_ura(&self) -> &str {
        &self.host_device_ura
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn incarnation_id(&self) -> &HostedAgentIncarnationId {
        &self.incarnation_id
    }
}

impl HostedAgentPublicationPlan {
    pub(crate) fn begin(
        agent_ura: &str,
        host_device_ura: &str,
        available_descriptors: &[crate::daemon::ability::descriptors::AbilityDescriptor],
    ) -> Result<Self, String> {
        if !available_descriptors
            .iter()
            .any(|descriptor| descriptor.owner_ura == agent_ura)
        {
            return Err(format!(
                "hosted Agent `{agent_ura}` has no committed LocalRuntime descriptors; refusing to publish an empty owner projection"
            ));
        }
        Self::begin_at_catalog_epoch(agent_ura, host_device_ura, available_descriptors, None)
    }

    /// Begin from an already captured durable catalog epoch. Dynamic
    /// publication uses this form so a later commit cannot relabel an older
    /// descriptor snapshot as current merely because network work was delayed.
    pub(crate) fn begin_at_catalog_epoch(
        agent_ura: &str,
        host_device_ura: &str,
        available_descriptors: &[crate::daemon::ability::descriptors::AbilityDescriptor],
        captured_catalog_epoch: Option<u64>,
    ) -> Result<Self, String> {
        let descriptors = available_descriptors
            .iter()
            .filter(|descriptor| descriptor.owner_ura == agent_ura)
            .cloned()
            .collect::<Vec<_>>();
        let _lifecycle_guard =
            crate::daemon::persistence::agent_lifecycle::AgentLifecycleMutationGuard::acquire()
                .map_err(|error| format!("fence hosted Agent registration plan: {error:#}"))?;
        require_local_llm_publication_owner(agent_ura, host_device_ura)?;
        let retraction_pending =
            crate::daemon::persistence::agent_lifecycle::load_retraction_journal()
                .map_err(|error| format!("load hosted Agent retraction fence: {error:#}"))?
                .is_some_and(|journal| journal.agent_ura == agent_ura);
        let outbox_pending = crate::daemon::persistence::agent_lifecycle::load_publication_outbox()
            .map_err(|error| format!("load hosted Agent revocation fence: {error:#}"))?
            .entries
            .iter()
            .any(|entry| entry.agent_ura == agent_ura);
        if retraction_pending || outbox_pending {
            return Err(format!(
                "hosted Agent `{agent_ura}` registration is fenced by local retraction"
            ));
        }
        let assignment = HostedAgentAssignmentPlan::begin(agent_ura, host_device_ura)?;
        let desired_catalog_epoch =
            crate::daemon::persistence::hosted_agent_publications::catalog_epoch_for_plan(
                agent_ura,
                host_device_ura,
                now_unix_ms(),
            )
            .map_err(|error| format!("load hosted Agent catalog epoch: {error:#}"))?;
        let catalog_epoch = captured_catalog_epoch.unwrap_or(desired_catalog_epoch);
        if catalog_epoch == 0 {
            return Err("hosted Agent publication catalog epoch must be nonzero".to_string());
        }
        Ok(Self {
            assignment,
            catalog_epoch,
            descriptors,
        })
    }

    pub(crate) fn identity_payload_bytes(&self) -> Result<Vec<u8>, String> {
        self.assignment.identity_payload_bytes()
    }

    #[must_use]
    pub(crate) fn assignment_plan(&self) -> &HostedAgentAssignmentPlan {
        &self.assignment
    }

    pub(crate) fn activate(
        self,
        assignment: HostedAgentGenerationAssignment,
    ) -> Result<AssignedHostedAgentPublication, String> {
        // Serialize assignment activation with local start/stop/purge. This
        // closes the late-response race where stop commits while an earlier
        // advertise_agent call is still in flight.
        let lifecycle_guard =
            crate::daemon::persistence::agent_lifecycle::AgentLifecycleMutationGuard::acquire()
                .map_err(|error| format!("fence hosted Agent assignment activation: {error:#}"))?;
        let assignment = self.assignment.bind(assignment)?;
        let retraction_pending =
            crate::daemon::persistence::agent_lifecycle::load_retraction_journal()
                .map_err(|error| format!("load hosted Agent retraction fence: {error:#}"))?
                .is_some_and(|journal| journal.agent_ura == assignment.agent_ura);
        let outbox_pending = crate::daemon::persistence::agent_lifecycle::load_publication_outbox()
            .map_err(|error| format!("load hosted Agent revocation fence: {error:#}"))?
            .entries
            .iter()
            .any(|entry| entry.agent_ura == assignment.agent_ura);
        if retraction_pending || outbox_pending {
            drop(lifecycle_guard);
            if retraction_pending {
                crate::daemon::ability::builtins::agents::lifecycle::recover_committed_retraction_after_assignment(
                    &assignment.agent_ura,
                )
                .map_err(|error| {
                    format!("handoff late Hub assignment to durable retraction: {error:#}")
                })?;
            }
            return Err(
                "Hub assignment arrived after local Agent retraction; ability publication is fenced"
                    .to_string(),
            );
        }
        require_local_llm_publication_owner(&assignment.agent_ura, &assignment.host_device_ura)?;
        let publication =
            crate::daemon::federation::read_model::owner_projection::prepare_and_persist_assigned(
                &assignment,
                &self.descriptors,
            )?;
        crate::daemon::persistence::hosted_agent_publications::stage_projection(
            &assignment,
            self.catalog_epoch,
            publication.projection_revision,
            &publication.projection_digest,
            now_unix_ms(),
        )
        .map_err(|error| format!("stage hosted Agent ability projection: {error:#}"))?;
        let abilities_payload = crate::daemon::federation::advertise::advertise_abilities_payload(
            &assignment.agent_ura,
            &publication,
        )
        .and_then(|value| {
            serde_json::to_vec(&value)
                .map_err(|error| format!("serialize advertise_abilities args: {error}"))
        })?;
        let ability_count = publication.ability_count();
        Ok(AssignedHostedAgentPublication {
            assignment,
            catalog_epoch: self.catalog_epoch,
            publication,
            abilities_payload,
            ability_count,
        })
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn incarnation_id(&self) -> &HostedAgentIncarnationId {
        self.assignment.incarnation_id()
    }
}

fn require_local_llm_publication_owner(
    agent_ura: &str,
    host_device_ura: &str,
) -> Result<(), String> {
    let file = crate::daemon::persistence::local_agents::load_for_fresh_host_projection()
        .map_err(|error| format!("load hosted Agent identity aggregate: {error:#}"))?;
    let aggregate =
        crate::daemon::persistence::local_agents::LocalHostedAgentIdentityAggregate::validate(
            &file,
        )
        .map_err(|error| format!("validate hosted Agent identity aggregate: {error:#}"))?;
    aggregate
        .require_llm_publication_owner(agent_ura, host_device_ura)
        .map_err(|error| format!("hosted Agent publication ownership denied: {error:#}"))
}

#[derive(Debug, Clone)]
pub(crate) struct AssignedHostedAgentPublication {
    pub(crate) assignment: HostedAgentGenerationAssignment,
    catalog_epoch: u64,
    pub(crate) publication:
        crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication,
    pub(crate) abilities_payload: Vec<u8>,
    pub(crate) ability_count: usize,
}

impl AssignedHostedAgentPublication {
    /// Cross the final Device-side lifecycle boundary after the Hub has
    /// acknowledged this exact complete-set projection.
    pub(crate) fn mark_published(&self) -> Result<(), String> {
        crate::daemon::persistence::hosted_agent_publications::mark_published(
            &self.assignment,
            self.catalog_epoch,
            self.publication.projection_revision,
            &self.publication.projection_digest,
            now_unix_ms(),
        )
        .map(|_| ())
        .map_err(|error| format!("commit hosted Agent published state: {error:#}"))
    }
}

fn now_unix_ms() -> u64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incarnation_id_is_strict_lower_hex_and_fresh() {
        let first = HostedAgentIncarnationId::fresh();
        let second = HostedAgentIncarnationId::fresh();
        assert_ne!(first, second);
        HostedAgentIncarnationId::parse(first.to_string()).unwrap();
        assert!(HostedAgentIncarnationId::parse("A".repeat(32)).is_err());
        assert!(HostedAgentIncarnationId::parse("a".repeat(31)).is_err());
        assert!(HostedAgentIncarnationId::parse("g".repeat(32)).is_err());
    }

    #[test]
    fn incarnation_id_deserialization_enforces_the_value_object_invariant() {
        let valid = format!("\"{}\"", "a".repeat(32));
        let parsed: HostedAgentIncarnationId =
            serde_json::from_str(&valid).expect("lowercase hex incarnation id");
        assert_eq!(parsed.as_str(), "a".repeat(32));

        for invalid in ["A".repeat(32), "a".repeat(31), "g".repeat(32)] {
            let encoded = serde_json::to_string(&invalid).unwrap();
            let error = serde_json::from_str::<HostedAgentIncarnationId>(&encoded)
                .expect_err("deserialization must not bypass incarnation validation");
            assert!(error.to_string().contains("lowercase hexadecimal"));
        }
    }

    #[test]
    fn durable_agent_intent_can_publish_an_empty_complete_set_after_last_ability_removal() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let host = "easynet:///r/test/device/dev-1";
        let agent = "easynet:///r/test/agent/alice.worker";
        crate::daemon::persistence::local_agents::save_test_llm_publication_owner(host, agent)
            .expect("persist test publication owner");
        let descriptor = crate::daemon::ability::descriptors::AbilityDescriptor::new(
            "chat",
            agent,
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("test descriptor");
        let first = HostedAgentPublicationPlan::begin(agent, host, &[descriptor])
            .expect("initial non-empty plan");
        let assignment = HostedAgentGenerationAssignment {
            agent_ura: agent.to_string(),
            host_device_ura: host.to_string(),
            incarnation_id: first.incarnation_id().clone(),
            generation: 1,
        };
        let published = first
            .activate(assignment.clone())
            .expect("initial projection");
        published.mark_published().expect("initial acknowledgement");

        let removal_epoch =
            crate::daemon::persistence::hosted_agent_publications::fence_catalog_commit(
                host,
                [agent],
                10,
            )
            .expect("fence final ability removal");
        let tombstone = HostedAgentPublicationPlan::begin_at_catalog_epoch(
            agent,
            host,
            &[],
            Some(removal_epoch),
        )
        .expect("durable intent may publish empty complete set")
        .activate(assignment)
        .expect("stage empty complete set");
        assert_eq!(tombstone.ability_count, 0);
        assert!(tombstone.publication.ability_summaries.is_empty());
        tombstone
            .mark_published()
            .expect("acknowledge empty complete set");
        assert_eq!(
            crate::daemon::persistence::hosted_agent_publications::record_for(agent)
                .unwrap()
                .unwrap()
                .publication_state(),
            "published"
        );
    }

    #[test]
    fn hosted_agent_projection_preserves_executable_descriptor_refs_for_every_mode() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let host = "easynet:///r/test/device/dev-1";
        let agent = "easynet:///r/test/agent/alice.worker";
        crate::daemon::persistence::local_agents::save_test_llm_publication_owner(host, agent)
            .expect("persist test publication owner");

        let rpc = crate::daemon::ability::descriptors::AbilityDescriptor::new(
            "chat",
            agent,
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("RPC descriptor")
        .with_source("daemon:control-plane")
        .with_metadata_entry("subject_contract_kind", "authenticated-user");
        let stream = rpc
            .clone()
            .with_call_mode(crate::daemon::ability::CallMode::Stream);
        let descriptors = vec![rpc, stream];
        let expected_refs = descriptors
            .iter()
            .map(|descriptor| {
                (
                    descriptor.call_mode().as_str().to_string(),
                    descriptor
                        .descriptor_ref()
                        .expect("executable descriptor ref"),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        let plan = HostedAgentPublicationPlan::begin(agent, host, &descriptors)
            .expect("hosted Agent publication plan");
        let assignment = HostedAgentGenerationAssignment {
            agent_ura: agent.to_string(),
            host_device_ura: host.to_string(),
            incarnation_id: plan.incarnation_id().clone(),
            generation: 1,
        };
        let active = plan.activate(assignment).expect("active publication");
        let published_refs = active
            .publication
            .ability_summaries
            .iter()
            .flat_map(|summary| summary.callable_summary.mode_geometry.iter())
            .map(|geometry| {
                (
                    geometry.call_mode.as_str().to_string(),
                    geometry.descriptor_ref.clone(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(published_refs, expected_refs);
    }

    #[test]
    fn generation_assignment_deserialization_enforces_exact_valid_geometry() {
        let canonical = serde_json::json!({
            "agent_ura": "easynet:///r/realm/agent/user.worker",
            "host_device_ura": "easynet:///r/realm/device/dev-1",
            "incarnation_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "generation": 1
        });
        serde_json::from_value::<HostedAgentGenerationAssignment>(canonical.clone())
            .expect("canonical assignment");

        let mut zero_generation = canonical.clone();
        zero_generation["generation"] = serde_json::json!(0);
        assert!(
            serde_json::from_value::<HostedAgentGenerationAssignment>(zero_generation).is_err()
        );

        let mut unknown = canonical;
        unknown["extra"] = serde_json::json!(true);
        assert!(serde_json::from_value::<HostedAgentGenerationAssignment>(unknown).is_err());
    }
}

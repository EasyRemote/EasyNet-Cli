//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/resources/context/device_scope.rs
//! Description: Canonical Device storage scope for daemon-local Context data.
//!
//! Protocol Responsibility:
//! - Keep Context persistence keyed by the sponsoring Device, never by the
//!   particular SystemAgent that produced or read an item.
//! - Reject execution actors that are not a Device or a declared
//!   device-sponsored SystemAgent.
//!
//! Implementation Approach:
//! - Parse canonical URAs and use the ability catalogue's inverse placement
//!   projection for SystemAgents.
//! - Carry the resulting Device URA as a validated value object.
//!
//! Usage Contract:
//! - Media and Context handlers must resolve this scope before touching the
//!   Context repository.
//! - Caller or subject URAs are not storage authority and must not be passed.
//!
//! Architectural Position:
//! - Context application boundary, above persistence and below ability handlers.

use crate::core::ura::URAKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextDeviceScope {
    device_ura: String,
}

impl ContextDeviceScope {
    pub(crate) fn from_execution_actor(actor_ura: &str) -> anyhow::Result<Self> {
        let actor = crate::core::ura::parse_ura(actor_ura)
            .map_err(|error| anyhow::anyhow!("Context execution actor is invalid: {error}"))?;
        let device_ura = match actor.kind {
            URAKind::Device => actor_ura.to_string(),
            URAKind::Agent => {
                crate::daemon::ability::catalog::ownership::execution_host_ura_for_device_sponsored_system_agent(
                    actor_ura,
                )?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Context execution actor is not a device-sponsored SystemAgent: {actor_ura}"
                    )
                })?
            }
            kind => anyhow::bail!(
                "Context execution actor must be a Device or device-sponsored SystemAgent, got {kind}"
            ),
        };
        Ok(Self { device_ura })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.device_ura
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_and_declared_system_agents_converge_on_one_storage_scope() {
        let device = "easynet:///r/localhost/device/dev-1";
        let media = crate::core::ura::device_agent_ura(
            "localhost",
            "dev-1",
            crate::daemon::ability::names::resources::MEDIA_SYSTEM_AGENT_ID,
        );
        let context = crate::core::ura::device_agent_ura(
            "localhost",
            "dev-1",
            crate::daemon::ability::names::resources::CONTEXT_SYSTEM_AGENT_ID,
        );

        for actor in [device, media.as_str(), context.as_str()] {
            assert_eq!(
                ContextDeviceScope::from_execution_actor(actor)
                    .expect("declared Device execution actor")
                    .as_str(),
                device
            );
        }
    }

    #[test]
    fn user_owned_agent_is_not_a_device_storage_scope() {
        let error = ContextDeviceScope::from_execution_actor(
            "easynet:///r/localhost/agent/user-1.assistant",
        )
        .expect_err("user Agent must not select local Device storage");
        assert!(error
            .to_string()
            .contains("not a device-sponsored SystemAgent"));
    }
}

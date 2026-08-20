//! Agent command boundary between CLI presentation and daemon-owned reads.

use std::sync::Arc;

use serde_json::Value;

/// The only dependency Agent CLI commands have on the daemon invocation plane.
pub(crate) trait AgentCommandGateway: Send + Sync {
    fn invoke(&self, ability: &str, args: Value) -> anyhow::Result<Value>;
}

/// Read-only dependency for daemon-owned Agent projections.
///
/// `agent.list` is a runtime-state read, not a command mutation. Keeping it
/// outside [`AgentCommandGateway`] prevents read projections from inheriting
/// the daemon-system command issuer used by mutating command abilities.
///
/// Agent ability publication also needs the runtime catalogue projection for a
/// concrete Agent owner. Keeping both operations semantic prevents CLI command
/// code from carrying arbitrary ability-string read dispatch.
pub(crate) trait AgentReadGateway: Send + Sync {
    fn list_agents(&self) -> anyhow::Result<Value>;

    fn list_agent_abilities(&self, agent_ura: &str) -> anyhow::Result<Value>;
}

#[derive(Debug, Default)]
struct DaemonAgentCommandGateway;

#[derive(Debug, Default)]
struct DaemonAgentReadGateway;

impl AgentCommandGateway for DaemonAgentCommandGateway {
    fn invoke(&self, ability: &str, args: Value) -> anyhow::Result<Value> {
        crate::support::platform::local_invoke::LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity(
            ability,
            args,
        )
        .map_err(|error| anyhow::anyhow!("{ability} failed: {error}"))
    }
}

impl AgentReadGateway for DaemonAgentReadGateway {
    fn list_agents(&self) -> anyhow::Result<Value> {
        crate::support::platform::local_invoke::LocalRuntimeStateReadIssuer::agent_list(
            serde_json::json!({}),
        )
        .map_err(|error| anyhow::anyhow!("agent list read failed: {error}"))
    }

    fn list_agent_abilities(&self, agent_ura: &str) -> anyhow::Result<Value> {
        crate::support::platform::local_invoke::LocalRuntimeCatalogueReadIssuer::list_abilities(
            serde_json::json!({
                "scope": "local",
                "agent_ura": agent_ura,
            }),
        )
        .map_err(|error| anyhow::anyhow!("agent ability catalogue read failed: {error}"))
    }
}

pub(crate) fn agent_command_gateway() -> Arc<dyn AgentCommandGateway> {
    #[cfg(test)]
    if let Some(gateway) = TEST_GATEWAY.with(|slot| slot.borrow().clone()) {
        return gateway;
    }

    Arc::new(DaemonAgentCommandGateway)
}

pub(crate) fn agent_read_gateway() -> Arc<dyn AgentReadGateway> {
    #[cfg(test)]
    if let Some(gateway) = TEST_READ_GATEWAY.with(|slot| slot.borrow().clone()) {
        return gateway;
    }

    Arc::new(DaemonAgentReadGateway)
}

#[cfg(test)]
thread_local! {
    static TEST_GATEWAY: std::cell::RefCell<Option<Arc<dyn AgentCommandGateway>>> =
        std::cell::RefCell::new(None);
    static TEST_READ_GATEWAY: std::cell::RefCell<Option<Arc<dyn AgentReadGateway>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn install_test_agent_command_gateway(
    gateway: Arc<dyn AgentCommandGateway>,
) -> TestAgentCommandGatewayGuard {
    let previous = TEST_GATEWAY.with(|slot| slot.replace(Some(gateway)));
    TestAgentCommandGatewayGuard { previous }
}

#[cfg(test)]
pub(crate) fn install_test_agent_read_gateway(
    gateway: Arc<dyn AgentReadGateway>,
) -> TestAgentReadGatewayGuard {
    let previous = TEST_READ_GATEWAY.with(|slot| slot.replace(Some(gateway)));
    TestAgentReadGatewayGuard { previous }
}

#[cfg(test)]
pub(crate) struct TestAgentCommandGatewayGuard {
    previous: Option<Arc<dyn AgentCommandGateway>>,
}

#[cfg(test)]
pub(crate) struct TestAgentReadGatewayGuard {
    previous: Option<Arc<dyn AgentReadGateway>>,
}

#[cfg(test)]
impl Drop for TestAgentCommandGatewayGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        TEST_GATEWAY.with(|slot| {
            slot.replace(previous);
        });
    }
}

#[cfg(test)]
impl Drop for TestAgentReadGatewayGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        TEST_READ_GATEWAY.with(|slot| {
            slot.replace(previous);
        });
    }
}

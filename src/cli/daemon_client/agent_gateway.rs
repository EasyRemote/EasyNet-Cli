//! Agent command boundary between CLI presentation and daemon-owned state.

use std::sync::Arc;

use serde_json::Value;

/// The only dependency Agent CLI commands have on the daemon invocation plane.
pub(crate) trait AgentCommandGateway: Send + Sync {
    fn invoke(&self, ability: &str, args: Value) -> anyhow::Result<Value>;
}

/// Read-only dependency for daemon-owned Agent state projections.
///
/// `agent.list` is a runtime-state read, not a command mutation. Keeping it
/// outside [`AgentCommandGateway`] prevents read projections from inheriting
/// the daemon-system command issuer used by mutating command abilities.
pub(crate) trait AgentStateReadGateway: Send + Sync {
    fn invoke_read(&self, ability: &str, args: Value) -> anyhow::Result<Value>;

    fn list_agents(&self) -> anyhow::Result<Value> {
        self.invoke_read("agent.list", serde_json::json!({}))
    }
}

#[derive(Debug, Default)]
struct DaemonAgentCommandGateway;

#[derive(Debug, Default)]
struct DaemonAgentStateReadGateway;

impl AgentCommandGateway for DaemonAgentCommandGateway {
    fn invoke(&self, ability: &str, args: Value) -> anyhow::Result<Value> {
        crate::support::platform::local_invoke::LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity(
            ability,
            args,
        )
        .map_err(|error| anyhow::anyhow!("{ability} failed: {error}"))
    }
}

impl AgentStateReadGateway for DaemonAgentStateReadGateway {
    fn invoke_read(&self, ability: &str, args: Value) -> anyhow::Result<Value> {
        crate::support::platform::local_invoke::LocalRuntimeStateReadIssuer::invoke(ability, args)
            .map_err(|error| anyhow::anyhow!("{ability} failed: {error}"))
    }
}

pub(crate) fn agent_command_gateway() -> Arc<dyn AgentCommandGateway> {
    #[cfg(test)]
    if let Some(gateway) = TEST_GATEWAY.with(|slot| slot.borrow().clone()) {
        return gateway;
    }

    Arc::new(DaemonAgentCommandGateway)
}

pub(crate) fn agent_state_read_gateway() -> Arc<dyn AgentStateReadGateway> {
    #[cfg(test)]
    if let Some(gateway) = TEST_STATE_READ_GATEWAY.with(|slot| slot.borrow().clone()) {
        return gateway;
    }

    Arc::new(DaemonAgentStateReadGateway)
}

#[cfg(test)]
thread_local! {
    static TEST_GATEWAY: std::cell::RefCell<Option<Arc<dyn AgentCommandGateway>>> =
        std::cell::RefCell::new(None);
    static TEST_STATE_READ_GATEWAY: std::cell::RefCell<Option<Arc<dyn AgentStateReadGateway>>> =
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
pub(crate) fn install_test_agent_state_read_gateway(
    gateway: Arc<dyn AgentStateReadGateway>,
) -> TestAgentStateReadGatewayGuard {
    let previous = TEST_STATE_READ_GATEWAY.with(|slot| slot.replace(Some(gateway)));
    TestAgentStateReadGatewayGuard { previous }
}

#[cfg(test)]
pub(crate) struct TestAgentCommandGatewayGuard {
    previous: Option<Arc<dyn AgentCommandGateway>>,
}

#[cfg(test)]
pub(crate) struct TestAgentStateReadGatewayGuard {
    previous: Option<Arc<dyn AgentStateReadGateway>>,
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
impl Drop for TestAgentStateReadGatewayGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        TEST_STATE_READ_GATEWAY.with(|slot| {
            slot.replace(previous);
        });
    }
}

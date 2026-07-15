//! Agent command boundary between CLI presentation and daemon-owned state.

use std::sync::Arc;

use serde_json::Value;

/// The only dependency Agent CLI commands have on the daemon invocation plane.
pub(crate) trait AgentCommandGateway: Send + Sync {
    fn invoke(&self, ability: &str, args: Value) -> anyhow::Result<Value>;
}

#[derive(Debug, Default)]
struct DaemonAgentCommandGateway;

impl AgentCommandGateway for DaemonAgentCommandGateway {
    fn invoke(&self, ability: &str, args: Value) -> anyhow::Result<Value> {
        crate::support::platform::local_invoke::invoke_local_ability(ability, args)
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

#[cfg(test)]
thread_local! {
    static TEST_GATEWAY: std::cell::RefCell<Option<Arc<dyn AgentCommandGateway>>> =
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
pub(crate) struct TestAgentCommandGatewayGuard {
    previous: Option<Arc<dyn AgentCommandGateway>>,
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

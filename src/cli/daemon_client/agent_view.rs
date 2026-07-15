use serde::Deserialize;
use std::str::FromStr;

/// Read-only view of daemon-owned agent state.
///
/// CLI modules use this for prompts, validation, and diagnostics. The
/// registry itself remains daemon-owned and is exposed through
/// `agent.list`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DaemonAgentRow {
    pub(crate) name: String,
    pub(crate) runtime: String,
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) root_path: Option<std::path::PathBuf>,
    #[serde(default)]
    pub(crate) timeout_secs: Option<u64>,
    #[serde(default)]
    pub(crate) root_exists: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentRuntimeKind {
    ClaudeCode,
    Codex,
    CodexAppServer,
    External,
}

impl AgentRuntimeKind {
    pub(crate) fn is_claude_code(self) -> bool {
        matches!(self, Self::ClaudeCode)
    }
}

impl std::fmt::Display for AgentRuntimeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClaudeCode => write!(f, "claude-code"),
            Self::Codex => write!(f, "codex"),
            Self::CodexAppServer => write!(f, "codex-app-server"),
            Self::External => write!(f, "external"),
        }
    }
}

impl FromStr for AgentRuntimeKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "claude-code" | "claude" => Ok(Self::ClaudeCode),
            "codex" => Ok(Self::Codex),
            "codex-app-server" | "codex-appserver" => Ok(Self::CodexAppServer),
            "external" | "custom" => Ok(Self::External),
            _ => anyhow::bail!(
                "unknown agent runtime: {value} (expected: claude-code, codex, codex-app-server, external)"
            ),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DaemonAgentListResponse {
    agents: Vec<DaemonAgentRow>,
}

pub(crate) fn list_agents() -> anyhow::Result<Vec<DaemonAgentRow>> {
    let gateway = crate::cli::daemon_client::agent_gateway::agent_command_gateway();
    list_agents_with_gateway(gateway.as_ref())
}

pub(crate) fn list_agents_with_gateway(
    gateway: &dyn crate::cli::daemon_client::agent_gateway::AgentCommandGateway,
) -> anyhow::Result<Vec<DaemonAgentRow>> {
    let response = gateway.invoke("agent.list", serde_json::json!({}))?;
    decode_agent_list_response(response)
}

fn decode_agent_list_response(response: serde_json::Value) -> anyhow::Result<Vec<DaemonAgentRow>> {
    let decoded: DaemonAgentListResponse = serde_json::from_value(response)
        .map_err(|err| anyhow::anyhow!("agent.list returned invalid payload: {err}"))?;
    Ok(decoded.agents)
}

pub(crate) fn agent_kind(row: &DaemonAgentRow) -> anyhow::Result<AgentRuntimeKind> {
    row.runtime
        .parse()
        .map_err(|err| anyhow::anyhow!("daemon returned invalid runtime {:?}: {err}", row.runtime))
}

pub(crate) fn agent_root(row: &DaemonAgentRow) -> anyhow::Result<std::path::PathBuf> {
    row.root_path.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "agent.list omitted root_path for agent {:?}; daemon agent projection is incomplete",
            row.name
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{agent_root, DaemonAgentRow};

    #[test]
    fn missing_root_path_is_a_projection_error_not_a_persistence_fallback() {
        let row = DaemonAgentRow {
            name: "alice".to_string(),
            runtime: "codex".to_string(),
            model: None,
            root_path: None,
            timeout_secs: None,
            root_exists: None,
        };

        let error = agent_root(&row).expect_err("missing daemon-owned root_path must fail");
        assert!(error.to_string().contains("agent.list omitted root_path"));
    }
}

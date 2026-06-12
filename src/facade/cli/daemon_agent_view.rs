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
            _ => anyhow::bail!(
                "unknown agent runtime: {value} (expected: claude-code, codex, codex-app-server)"
            ),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DaemonAgentListResponse {
    agents: Vec<DaemonAgentRow>,
}

pub(crate) fn list_agents() -> anyhow::Result<Vec<DaemonAgentRow>> {
    let response = match crate::support::local_invoke::invoke_local_ability(
        "agent.list",
        serde_json::json!({}),
    ) {
        Ok(response) => response,
        Err(err) => {
            #[cfg(test)]
            {
                if matches!(
                    crate::support::local_invoke::classify_invoke_error(&err),
                    crate::support::local_invoke::LocalInvokeErrorKind::DaemonOffline
                ) {
                    return list_agents_from_disk_for_tests();
                }
            }
            return Err(err);
        }
    };
    decode_agent_list_response(response)
}

pub(crate) fn list_agents_with_client(
    client: &crate::support::local_daemon_grpc::LocalDaemonAbilityClient,
) -> anyhow::Result<Vec<DaemonAgentRow>> {
    let response = client
        .invoke("agent.list", serde_json::json!({}))
        .map_err(|err| anyhow::anyhow!("agent.list failed: {err}"))?;
    decode_agent_list_response(response)
}

fn decode_agent_list_response(response: serde_json::Value) -> anyhow::Result<Vec<DaemonAgentRow>> {
    let decoded: DaemonAgentListResponse = serde_json::from_value(response)
        .map_err(|err| anyhow::anyhow!("agent.list returned invalid payload: {err}"))?;
    Ok(decoded.agents)
}

#[cfg(test)]
fn list_agents_from_disk_for_tests() -> anyhow::Result<Vec<DaemonAgentRow>> {
    let registry = crate::registry::agents::load_agents()?;
    Ok(registry
        .agents
        .into_iter()
        .map(|(name, entry)| {
            let root_exists = entry.root_path.as_ref().map(|path| path.exists());
            DaemonAgentRow {
                name,
                runtime: entry.agent_type.to_string(),
                model: entry.model,
                root_path: entry.root_path,
                timeout_secs: Some(entry.timeout_secs),
                root_exists,
            }
        })
        .collect())
}

pub(crate) fn agent_kind(row: &DaemonAgentRow) -> anyhow::Result<AgentRuntimeKind> {
    row.runtime
        .parse()
        .map_err(|err| anyhow::anyhow!("daemon returned invalid runtime {:?}: {err}", row.runtime))
}

pub(crate) fn agent_root(row: &DaemonAgentRow) -> std::path::PathBuf {
    row.root_path
        .clone()
        .unwrap_or_else(|| crate::persistence::config::agents_root().join(&row.name))
}

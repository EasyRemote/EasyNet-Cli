use crate::cli::daemon_client::agent_view::AgentRuntimeKind;
use crate::daemon::execution::mission::drivers::{claude_code, codex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalAgentCliProbe {
    ClaudeCode,
    Codex,
}

impl LocalAgentCliProbe {
    pub(crate) fn for_runtime(runtime: AgentRuntimeKind) -> Option<Self> {
        match runtime {
            AgentRuntimeKind::ClaudeCode => Some(Self::ClaudeCode),
            AgentRuntimeKind::Codex | AgentRuntimeKind::CodexAppServer => Some(Self::Codex),
            AgentRuntimeKind::External => None,
        }
    }

    pub(crate) fn run(self) -> anyhow::Result<String> {
        match self {
            Self::ClaudeCode => claude_code::doctor(),
            Self::Codex => codex::doctor(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentRuntimeKind, LocalAgentCliProbe};

    #[test]
    fn runtime_kind_selects_local_cli_probe_explicitly() {
        assert_eq!(
            LocalAgentCliProbe::for_runtime(AgentRuntimeKind::ClaudeCode),
            Some(LocalAgentCliProbe::ClaudeCode)
        );
        assert_eq!(
            LocalAgentCliProbe::for_runtime(AgentRuntimeKind::Codex),
            Some(LocalAgentCliProbe::Codex)
        );
        assert_eq!(
            LocalAgentCliProbe::for_runtime(AgentRuntimeKind::CodexAppServer),
            Some(LocalAgentCliProbe::Codex)
        );
        assert_eq!(
            LocalAgentCliProbe::for_runtime(AgentRuntimeKind::External),
            None
        );
    }
}

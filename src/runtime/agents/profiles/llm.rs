//! llm profile — RFC-001 §1.
//!
//! An Agent advertising conversation.* + session.* + meta.* +
//! per-skill abilities. One per registered AI sub-agent (claude /
//! codex / future). Per RFC §1.1 + §A15: each LLM sub-agent's
//! installed skills are PRIVATE abilities owned by that sub-agent.
//!
//! Owned ability namespaces
//! ------------------------
//!   conversation.send / conversation.stream  (default visibility SCOPED [P8])
//!   session.create / session.list / session.resume / session.close
//!   meta.describe / meta.list_abilities / meta.acquire / meta.forget
//!     / meta.publish / meta.compose / meta.cancel
//!   skill.<name>  (per-skill PRIVATE abilities; one per directory in
//!                  ~/.claude/skills/, ~/.agents/skills/, etc.)
//!
//! Currently wired in agents/chat_ability.rs (which renames to
//! conversation_ability.rs in a follow-up cleanup). The chat handler
//! is the conversation.send implementation.

pub const LLM_PROFILE_ABILITY_PREFIXES: &[&str] = &[
    "conversation.",
    "session.",
    "meta.",
    "skill.",
];

pub fn owns(ability_name: &str) -> bool {
    LLM_PROFILE_ABILITY_PREFIXES
        .iter()
        .any(|p| ability_name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owns_recognizes_llm_namespaces() {
        assert!(owns("conversation.send"));
        assert!(owns("conversation.stream"));
        assert!(owns("session.create"));
        assert!(owns("session.resume"));
        assert!(owns("meta.describe"));
        assert!(owns("meta.list_abilities"));
        assert!(owns("skill.alive-video"));
        assert!(owns("skill.design"));
    }

    #[test]
    fn owns_rejects_other_profiles() {
        assert!(!owns("fleet.list_abilities"));
        assert!(!owns("consent.subscribe"));
        assert!(!owns("policy.evaluate"));
    }
}

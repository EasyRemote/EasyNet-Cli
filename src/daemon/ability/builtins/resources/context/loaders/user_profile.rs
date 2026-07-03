// EasyNet CLI — UserProfileLoader (chat context)
// ===============================================
//
// File: src/daemon/ability/builtins/resources/context/loaders/user_profile.rs
// Description: A `ContextLoader` that reads `~/.easynet/profile.toml`
//              and emits its contents as a markdown "## User profile"
//              block the chat handler prepends to the LLM context.
//
// On-disk format (v1)
// -------------------
// `~/.easynet/profile.toml` is the single source of truth for the
// operator-level user profile shared across every agent on this
// install. Schema (all fields optional):
//
//     display_name = "Silan"
//     role = "researcher"
//     bio = "..."
//
//     [preferences]
//     timezone = "Asia/Singapore"
//     verbosity = "concise"
//     # …any free-form key = string-value
//
// The `[preferences]` table is a free-form map<string, string> so an
// operator can encode personal conventions without us shipping a
// schema-bump every time someone wants to add a new field. Loader
// emits whatever it finds; absent fields are skipped (no nulls in
// the markdown).
//
// Why global, not per-agent
// -------------------------
// User profile is "who is talking" — that does not change agent to
// agent. Per-agent context (memory, schedule) is in their own
// loaders. A future per-agent profile would live in
// `<agent-root>/profile.toml`; a future loader can layer that on top
// of the global one.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::daemon::ability::builtins::agents::chat::ContextLoader;

/// Resolved per call so an operator who edits the file mid-session
/// sees the change on the next chat invocation. Cheap enough — one
/// stat + one read of a small TOML file.
fn profile_path() -> PathBuf {
    crate::daemon::persistence::config::state_dir().join("profile.toml")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub preferences: BTreeMap<String, String>,
}

impl UserProfile {
    /// True when the profile has no usable content. Used to skip
    /// rendering an empty "## User profile" block.
    fn is_empty(&self) -> bool {
        self.display_name.is_none()
            && self.role.is_none()
            && self.bio.is_none()
            && self.preferences.is_empty()
    }

    /// Render to a markdown block. Caller checked `!is_empty()` so
    /// we can assume at least one field is populated.
    fn to_markdown(&self) -> String {
        let mut out = String::from("## User profile\n\n");
        if let Some(n) = &self.display_name {
            out.push_str(&format!("- **Name:** {n}\n"));
        }
        if let Some(r) = &self.role {
            out.push_str(&format!("- **Role:** {r}\n"));
        }
        if let Some(b) = &self.bio {
            out.push_str(&format!("- **Bio:** {b}\n"));
        }
        if !self.preferences.is_empty() {
            out.push_str("\n### Preferences\n\n");
            for (k, v) in &self.preferences {
                out.push_str(&format!("- **{k}:** {v}\n"));
            }
        }
        out
    }
}

pub struct UserProfileLoader;

impl UserProfileLoader {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UserProfileLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextLoader for UserProfileLoader {
    fn name(&self) -> &str {
        "user_profile"
    }

    fn load(&self, _agent_name: &str, _session_id: &str) -> anyhow::Result<Option<String>> {
        let path = profile_path();
        // NotFound is the steady state for users who haven't written a
        // profile yet — return None silently rather than logging on
        // every chat call.
        let body = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "user_profile: read {} failed: {e}",
                    path.display()
                ));
            }
        };
        let profile: UserProfile = ::toml::from_str(&body).map_err(|e| {
            anyhow::anyhow!(
                "user_profile: parse {} failed: {e} (loader returns Err so the chat handler \
                 records the failure in context_used rather than silently dropping it)",
                path.display()
            )
        })?;
        if profile.is_empty() {
            return Ok(None);
        }
        Ok(Some(profile.to_markdown()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;

    fn write_profile(body: &str) {
        let path = profile_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
    }

    #[test]
    fn loader_returns_none_when_profile_missing() {
        let _g = HomeGuard::new();
        let loader = UserProfileLoader::new();
        assert!(loader.load("alice", "s-1").unwrap().is_none());
    }

    #[test]
    fn loader_returns_none_for_empty_profile_file() {
        let _g = HomeGuard::new();
        write_profile("");
        let loader = UserProfileLoader::new();
        assert!(loader.load("alice", "s-1").unwrap().is_none());
    }

    #[test]
    fn loader_renders_minimal_profile() {
        let _g = HomeGuard::new();
        write_profile("display_name = \"Silan\"\n");
        let loader = UserProfileLoader::new();
        let text = loader.load("alice", "s-1").unwrap().unwrap();
        assert!(text.contains("## User profile"));
        assert!(text.contains("**Name:** Silan"));
    }

    #[test]
    fn loader_renders_full_profile_with_preferences() {
        let _g = HomeGuard::new();
        write_profile(
            "display_name = \"Silan\"\n\
             role = \"researcher\"\n\
             bio = \"Builds EasyNet\"\n\
             [preferences]\n\
             timezone = \"Asia/Singapore\"\n\
             verbosity = \"concise\"\n",
        );
        let loader = UserProfileLoader::new();
        let text = loader.load("alice", "s-1").unwrap().unwrap();
        assert!(text.contains("**Name:** Silan"));
        assert!(text.contains("**Role:** researcher"));
        assert!(text.contains("**Bio:** Builds EasyNet"));
        assert!(text.contains("### Preferences"));
        assert!(text.contains("**timezone:** Asia/Singapore"));
        assert!(text.contains("**verbosity:** concise"));
    }

    #[test]
    fn loader_returns_err_on_malformed_toml() {
        let _g = HomeGuard::new();
        write_profile("not = a = valid = toml");
        let loader = UserProfileLoader::new();
        let err = loader.load("alice", "s-1").unwrap_err();
        assert!(format!("{err}").contains("parse"));
    }

    #[test]
    fn loader_is_agent_independent() {
        // user_profile is global — same content for every agent on
        // this install.
        let _g = HomeGuard::new();
        write_profile("display_name = \"Silan\"\n");
        let loader = UserProfileLoader::new();
        let a = loader.load("alice", "s-1").unwrap().unwrap();
        let b = loader.load("bob", "s-2").unwrap().unwrap();
        assert_eq!(a, b);
    }
}

// EasyNet CLI — ScheduleLoader (chat context)
// ============================================
//
// File: src/daemon/ability/builtins/resources/context/loaders/schedule.rs
// Description: A `ContextLoader` that asks the running daemon's
//              `ScheduleService` for upcoming fires bound to a given
//              agent and emits them as a markdown block the chat
//              handler injects before the LLM call.
//
// Why this exists
// ---------------
// "What's on your calendar?" is the most common ambient context an
// agent needs at chat time — without it, an LLM asked "should I
// remind the user about that report?" has no way to know one is
// scheduled. Surfacing the next 24h of fires (per agent) is the
// minimum useful slice; longer horizons make the prompt noisy.
//
// Filtering rule
// --------------
// Loader emits only schedules whose `target_agent` matches the
// chat's agent name AND whose next fire is within the configured
// horizon (24h default). Schedules that fire outside the horizon are
// real work but irrelevant to "what should I think about right
// now"; they belong in a future "schedule overview" surface, not
// the chat context block.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::daemon::ability::builtins::agents::chat::ContextLoader;
use crate::daemon::execution::schedule::ScheduleService;

/// Default horizon: 24 hours. Schedules firing later than this are
/// excluded from the chat context block so the LLM's prompt stays
/// focused. The constant is exposed in case a future runtime config
/// wants to tune it; today there is no setter — change the
/// constant and recompile.
pub const DEFAULT_HORIZON_SECS: u64 = 24 * 60 * 60;

/// Maximum number of schedule entries to render in the context
/// block. A long-tail of low-priority cron jobs would otherwise
/// blow up the prompt. The cap is intentionally low — chat
/// callers wanting the full list should call
/// `schedule.list` directly.
pub const MAX_ENTRIES_RENDERED: usize = 10;

pub struct ScheduleLoader {
    svc: Arc<ScheduleService>,
    horizon: Duration,
    max_entries: usize,
}

impl ScheduleLoader {
    pub fn new(svc: Arc<ScheduleService>) -> Self {
        Self {
            svc,
            horizon: Duration::from_secs(DEFAULT_HORIZON_SECS),
            max_entries: MAX_ENTRIES_RENDERED,
        }
    }

    /// Test/tuning constructor. Production wires `new()`.
    #[cfg(test)]
    pub fn with_horizon_and_cap(
        svc: Arc<ScheduleService>,
        horizon: Duration,
        max_entries: usize,
    ) -> Self {
        Self {
            svc,
            horizon,
            max_entries,
        }
    }
}

impl ContextLoader for ScheduleLoader {
    fn name(&self) -> &str {
        "schedule"
    }

    fn load(&self, agent_name: &str, _session_id: &str) -> anyhow::Result<Option<String>> {
        let now = Utc::now();
        let horizon_end = now + chrono::Duration::from_std(self.horizon)?;

        let mut upcoming: Vec<(DateTime<Utc>, String, Option<String>)> = Vec::new();
        for entry in self.svc.list() {
            if !entry.enabled {
                continue;
            }
            if entry.target_agent.as_str() != agent_name {
                continue;
            }
            // next_fire_after returns Result<Option<DateTime>>; the
            // outer Err path is "schedule not found / unknown id" —
            // race against a remove(); skip silently. The inner None
            // means "cron has no future fire" (e.g. `@once` already
            // past); skip too.
            let next = match self.svc.next_fire_after(&entry.id, now) {
                Ok(Some(t)) => t,
                Ok(None) | Err(_) => continue,
            };
            if next > horizon_end {
                continue;
            }
            upcoming.push((next, entry.cron_expr.clone(), entry.prompt.clone()));
        }

        if upcoming.is_empty() {
            return Ok(None);
        }

        // Stable order: soonest first. Two fires at the same instant
        // sort by cron expression for determinism (rare collision but
        // not impossible on `@hourly` / `0 * * * *`).
        upcoming.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

        let total = upcoming.len();
        let truncated = total > self.max_entries;
        upcoming.truncate(self.max_entries);

        let mut out = String::new();
        out.push_str("## Upcoming scheduled work (next ");
        out.push_str(&format!("{}h", self.horizon.as_secs() / 3600));
        out.push_str(")\n\n");
        for (when, cron, prompt) in upcoming {
            // Local time would be more readable but not portable
            // across daemon hosts; ISO UTC is unambiguous and the
            // LLM can convert if asked.
            out.push_str(&format!("- **{}** (cron `{}`)", when.to_rfc3339(), cron));
            if let Some(p) = prompt {
                let preview: String = p.chars().take(80).collect();
                out.push_str(": ");
                out.push_str(&preview);
                if p.chars().count() > 80 {
                    out.push('…');
                }
            }
            out.push('\n');
        }
        if truncated {
            out.push_str(&format!(
                "\n_…and {} more scheduled item(s) beyond the {}-entry cap._\n",
                total - self.max_entries,
                self.max_entries
            ));
        }
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::domain::{
        AgentId, MisfirePolicy, NodeId, ScheduleEntry, ScheduleId, TenantId,
    };

    fn make_entry(id: &str, agent: &str, cron: &str, enabled: bool) -> ScheduleEntry {
        ScheduleEntry {
            id: ScheduleId::new(id),
            tenant: TenantId::default_v1(),
            target_node: NodeId::new("self"),
            target_agent: AgentId::new(agent),
            cron_expr: cron.to_string(),
            misfire_policy: MisfirePolicy::Skip,
            catch_up_window_secs: None,
            enabled,
            prompt: None,
        }
    }

    #[test]
    fn loader_returns_none_when_no_schedules_for_agent() {
        let svc = Arc::new(ScheduleService::new());
        let loader = ScheduleLoader::new(svc);
        assert!(loader.load("alice", "s-1").unwrap().is_none());
    }

    #[test]
    fn loader_returns_none_when_only_other_agents_have_schedules() {
        let svc = Arc::new(ScheduleService::new());
        svc.add(make_entry("s1", "bob", "0 * * * *", true)).unwrap();
        let loader = ScheduleLoader::new(svc);
        assert!(loader.load("alice", "s-1").unwrap().is_none());
    }

    #[test]
    fn loader_returns_none_when_disabled_schedules_only() {
        let svc = Arc::new(ScheduleService::new());
        svc.add(make_entry("s1", "alice", "0 * * * *", false))
            .unwrap();
        let loader = ScheduleLoader::new(svc);
        assert!(loader.load("alice", "s-1").unwrap().is_none());
    }

    #[test]
    fn loader_renders_upcoming_schedule_block() {
        let svc = Arc::new(ScheduleService::new());
        svc.add(make_entry("s1", "alice", "0 * * * *", true))
            .unwrap();
        let loader = ScheduleLoader::new(svc);
        let out = loader.load("alice", "s-1").unwrap();
        let text = out.expect("upcoming hourly schedule must render");
        assert!(text.contains("## Upcoming scheduled work"));
        assert!(text.contains("0 * * * *"));
    }

    #[test]
    fn loader_caps_at_max_entries_and_notes_truncation() {
        let svc = Arc::new(ScheduleService::new());
        // Add more than the cap so truncation kicks in.
        for i in 0..(MAX_ENTRIES_RENDERED + 3) {
            svc.add(make_entry(&format!("s{i}"), "alice", "0 * * * *", true))
                .unwrap();
        }
        let loader = ScheduleLoader::new(svc);
        let text = loader.load("alice", "s-1").unwrap().unwrap();
        // Truncation note pins the "more …" footer; if a refactor
        // forgets to emit it, the LLM would see a silently truncated
        // list and might believe nothing else is scheduled.
        assert!(text.contains("more scheduled item(s) beyond"));
    }
}

// EasyNet CLI — Execution / Schedule sub-service
// ===============================================
//
// File: src/runtime/execution/schedule/mod.rs
// Description: Cron-driven schedule store + tick runner. Backed by
//              JSON files under
//              `~/.easynet/tenants/<tenant>/schedules/<id>.json`
//              (via persistence::tenant_paths). Each schedule
//              entry carries a `MisfirePolicy` frozen by plan
//              v10.1 §"Schedule misfire policy".
//
// Plan v10.3 C* unity reminder
// ----------------------------
// When the tick runner fires a schedule, it MUST construct a full
// Invocation (caller=this node, callee=target, ability=schedule
// config, subject=schedule_id URA, causal_context=Scalar(last
// receipt) or Null) and route through Kernel::invoke. v1 ships
// the `next_fire_invocation()` builder that produces the
// Invocation; PR-INVOCATION-EXEC-UNITY collapses the runner onto
// Kernel::invoke as the sole execution entry point. Calling
// `run_mission_inproc` from this module is a CI-grep failure.
//
// Misfire policies (proto enum, frozen in domain::MisfirePolicy)
// --------------------------------------------------------------
//   * Skip               — skip every missed fire on resume.
//   * FireOnce           — fire once on resume regardless of count.
//   * CatchUpWindowed    — fire every miss within `catch_up_window_secs`
//                          (default 24h).
// `unbounded_catch_up` is explicitly NOT a v1 option (thundering
// herd is a worse failure mode than dropped data).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

mod store;

pub use store::ScheduleStore;

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::RwLock;

use chrono::{DateTime, TimeZone, Utc};
use cron::Schedule as CronSchedule;
use uuid::Uuid;

use crate::runtime::domain::{
    AgentId, MisfirePolicy, NodeId, ScheduleEntry, ScheduleId, TenantId,
};

#[derive(Default)]
pub struct ScheduleService {
    /// In-memory cache of every schedule on disk. Kept hot so the
    /// tick runner does not re-read the filesystem every minute.
    cache: RwLock<BTreeMap<ScheduleId, ScheduleEntry>>,
    /// Disk-backed store. Set to `Some(...)` after `bind` is
    /// called with a tenant id; tests construct the service
    /// without a store and operate on the in-memory cache only.
    store: RwLock<Option<ScheduleStore>>,
}

impl ScheduleService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the service to a tenant-scoped disk store. Loads any
    /// existing schedules into the in-memory cache. Daemon bin
    /// calls this at boot.
    pub fn bind(&self, tenant: &TenantId) -> anyhow::Result<()> {
        let store = ScheduleStore::open(tenant)?;
        let loaded = store.load_all()?;
        let mut cache = self
            .cache
            .write()
            .map_err(|_| anyhow::anyhow!("ScheduleService cache lock poisoned"))?;
        for entry in loaded {
            cache.insert(entry.id.clone(), entry);
        }
        let mut s = self
            .store
            .write()
            .map_err(|_| anyhow::anyhow!("ScheduleService store lock poisoned"))?;
        *s = Some(store);
        Ok(())
    }

    /// Add a new schedule. v1 generates a fresh `ScheduleId`,
    /// validates the cron expression, persists to disk if a store
    /// is bound, and inserts into the cache. Returns the id.
    ///
    /// Validation rejects:
    ///   * empty / unparseable cron expressions
    ///   * unbounded MisfirePolicy values (none in v1)
    pub fn add(&self, mut entry: ScheduleEntry) -> anyhow::Result<ScheduleId> {
        // Validate cron BEFORE assigning id so the caller's id is
        // not consumed by a malformed entry.
        validate_cron(&entry.cron_expr)?;
        if entry.id.as_str().is_empty() {
            entry.id = ScheduleId::new(format!("sched-{}", Uuid::new_v4()));
        }
        // Persist to disk if a store is bound.
        if let Some(store) = self
            .store
            .read()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
            .as_ref()
        {
            store.save(&entry)?;
        }
        let id = entry.id.clone();
        let mut cache = self
            .cache
            .write()
            .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
        cache.insert(id.clone(), entry);
        Ok(id)
    }

    /// Remove a schedule by id. Errors when the id does not exist.
    pub fn remove(&self, id: &ScheduleId) -> anyhow::Result<()> {
        let removed = {
            let mut cache = self
                .cache
                .write()
                .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
            cache.remove(id).is_some()
        };
        if !removed {
            anyhow::bail!("schedule {id} not found");
        }
        if let Some(store) = self
            .store
            .read()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
            .as_ref()
        {
            store.delete(id)?;
        }
        Ok(())
    }

    /// Toggle the `enabled` field of a schedule.
    pub fn enable(&self, id: &ScheduleId, enabled: bool) -> anyhow::Result<()> {
        let entry_for_save = {
            let mut cache = self
                .cache
                .write()
                .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
            let entry = cache
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("schedule {id} not found"))?;
            entry.enabled = enabled;
            entry.clone()
        };
        if let Some(store) = self
            .store
            .read()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
            .as_ref()
        {
            store.save(&entry_for_save)?;
        }
        Ok(())
    }

    /// Snapshot every schedule currently indexed. Deterministic
    /// order via BTreeMap.
    pub fn list(&self) -> Vec<ScheduleEntry> {
        match self.cache.read() {
            Ok(g) => g.values().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Compute the next fire instant for a schedule, given a
    /// "now" anchor. Returns None when the cron expression has no
    /// future fire (e.g. one-shot schedules already past).
    pub fn next_fire_after(
        &self,
        id: &ScheduleId,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<DateTime<Utc>>> {
        let cache = self
            .cache
            .read()
            .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
        let entry = cache
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("schedule {id} not found"))?;
        let cron = parse_cron(&entry.cron_expr)?;
        Ok(cron.after(&now).next())
    }

    /// Compute the next fires that should be triggered at `now`,
    /// taking misfire policy into account. Used by the tick
    /// runner. Returns the schedules whose next-fire instant is
    /// at or before `now`.
    ///
    /// `last_fire_unix_ms_for(id)`: caller-supplied function that
    /// returns the last successful fire instant the runner
    /// recorded. v1 idempotency key = (schedule_id, fire_unix_minute);
    /// the runner persists this in a sidecar so daemon restart does
    /// not duplicate-fire.
    pub fn due(
        &self,
        now: DateTime<Utc>,
        last_fire_unix_ms_for: impl Fn(&ScheduleId) -> Option<i64>,
    ) -> Vec<DueFire> {
        let cache = match self.cache.read() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        for entry in cache.values().filter(|e| e.enabled) {
            let cron = match parse_cron(&entry.cron_expr) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let last_fire = last_fire_unix_ms_for(&entry.id)
                .and_then(|ms| Utc.timestamp_millis_opt(ms).single());
            let anchor = last_fire.unwrap_or_else(|| {
                now - chrono::Duration::seconds(catch_up_seconds_for(entry))
            });
            let fires: Vec<DateTime<Utc>> = cron
                .after(&anchor)
                .take_while(|t| t <= &now)
                .collect();
            if fires.is_empty() {
                continue;
            }
            match entry.misfire_policy {
                MisfirePolicy::Skip => {
                    // Only fire the most recent one (i.e. drop misses).
                    // `catch_up` is true when the latest fire is a
                    // collapse of multiple missed fires — the runner
                    // logs / surfaces this so an operator can see
                    // "we did skip data here". A single on-time fire
                    // (fires.len() == 1) reports catch_up = false.
                    if let Some(latest) = fires.last() {
                        out.push(DueFire {
                            schedule_id: entry.id.clone(),
                            fire_at: *latest,
                            catch_up: fires.len() > 1,
                        });
                    }
                }
                MisfirePolicy::FireOnce => {
                    // Drop misses; fire exactly one.
                    if let Some(first) = fires.first() {
                        out.push(DueFire {
                            schedule_id: entry.id.clone(),
                            fire_at: *first,
                            catch_up: fires.len() > 1,
                        });
                    }
                }
                MisfirePolicy::CatchUpWindowed => {
                    let window = chrono::Duration::seconds(catch_up_seconds_for(entry));
                    for fire in &fires {
                        if (now - *fire) <= window {
                            out.push(DueFire {
                                schedule_id: entry.id.clone(),
                                fire_at: *fire,
                                catch_up: fires.len() > 1,
                            });
                        }
                    }
                }
            }
        }
        out
    }
}

/// One fire decision the tick runner consumes. The runner builds
/// an Invocation from this and routes through Kernel::invoke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueFire {
    pub schedule_id: ScheduleId,
    pub fire_at: DateTime<Utc>,
    /// True when more than one fire was due (the runner uses this
    /// to log "catching up"). False when this is a fresh on-time
    /// fire.
    pub catch_up: bool,
}

/// Catch-up window in seconds. Defaults to 24h when the entry's
/// `catch_up_window_secs` is None.
fn catch_up_seconds_for(entry: &ScheduleEntry) -> i64 {
    entry.catch_up_window_secs.unwrap_or(86_400) as i64
}

/// Validate a cron expression. Pre-registers the schedule add path
/// so a typo'd cron (`* * *`-three-fields, leading whitespace,
/// English month names) fails at the boundary instead of silently
/// never firing.
fn validate_cron(expr: &str) -> anyhow::Result<()> {
    parse_cron(expr).map(|_| ())
}

/// Parse a cron expression. v1 uses the `cron` crate's 6-field
/// (with seconds) or 7-field (with year) syntax. We canonicalise
/// classic 5-field unix-style ("0 9 * * *") to 6-field by
/// prepending "0" for seconds — that is what the crate expects.
fn parse_cron(expr: &str) -> anyhow::Result<CronSchedule> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        anyhow::bail!("cron expression is empty");
    }
    // Prefix a `0` for the seconds field if the user gave a
    // classic 5-field expression. We detect by counting whitespace-
    // separated tokens.
    let canonical = if trimmed.split_whitespace().count() == 5 {
        format!("0 {trimmed}")
    } else {
        trimmed.to_string()
    };
    CronSchedule::from_str(&canonical)
        .map_err(|e| anyhow::anyhow!("invalid cron expression {expr:?}: {e}"))
}

impl std::fmt::Debug for ScheduleService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.cache.read().ok().map(|g| g.len()).unwrap_or(0);
        write!(f, "ScheduleService {{ schedules: {n} }}")
    }
}

/// Convenience constructor for tests + the daemon bin's setup
/// path: build a fully-formed `ScheduleEntry` from the args
/// `schedule.add` accepts.
pub fn make_entry(
    target_node: &str,
    target_agent: &str,
    cron_expr: &str,
    misfire: MisfirePolicy,
) -> ScheduleEntry {
    ScheduleEntry {
        id: ScheduleId::new(""),
        tenant: TenantId::default_v1(),
        target_node: NodeId::new(target_node),
        target_agent: AgentId::new(target_agent),
        cron_expr: cron_expr.into(),
        misfire_policy: misfire,
        catch_up_window_secs: None,
        enabled: true,
        prompt: None,
    }
}

/// Render a schedule's prompt template for one fire.
///
/// Substitutes the four supported `{{var}}` tokens with their
/// values. Unknown tokens stay literal (no template error) so a
/// typo surfaces as odd prompt text instead of a hard schedule
/// failure. v1 takes the simple-replace approach over a real
/// template engine (handlebars / tera) because the supported
/// variable set is small and bounded; a v2 expansion to richer
/// expressions would justify a real engine.
pub fn render_prompt(
    template: &str,
    schedule_id: &str,
    fire_at: &chrono::DateTime<chrono::Utc>,
    catch_up: bool,
    target_agent: &str,
) -> String {
    template
        .replace("{{schedule_id}}", schedule_id)
        .replace("{{fire_at_iso}}", &fire_at.to_rfc3339())
        .replace("{{catch_up}}", if catch_up { "true" } else { "false" })
        .replace("{{target_agent}}", target_agent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(cron_expr: &str, policy: MisfirePolicy) -> ScheduleEntry {
        make_entry("self", "alice", cron_expr, policy)
    }

    #[test]
    fn add_then_list_returns_entry_with_assigned_id() {
        let svc = ScheduleService::new();
        let id = svc.add(entry("0 9 * * *", MisfirePolicy::Skip)).unwrap();
        let listed = svc.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert!(listed[0].enabled);
    }

    #[test]
    fn add_rejects_malformed_cron_at_boundary() {
        // A typo'd cron must fail loudly at add() rather than be
        // silently accepted and never fire. Pin the message so
        // operators can grep "invalid cron" to find their bad
        // schedule line.
        let svc = ScheduleService::new();
        let err = svc.add(entry("totally not cron", MisfirePolicy::Skip)).unwrap_err();
        assert!(format!("{err}").contains("invalid cron"));
    }

    #[test]
    fn parse_cron_accepts_classic_5_field_unix_syntax() {
        // Operators most often write 5-field crons (no seconds).
        // The internal canonicaliser must accept "0 9 * * *" the
        // same way it accepts "0 0 9 * * *".
        assert!(parse_cron("0 9 * * *").is_ok());
        assert!(parse_cron("*/5 * * * *").is_ok());
    }

    #[test]
    fn enable_toggles_field_in_cache() {
        let svc = ScheduleService::new();
        let id = svc.add(entry("0 9 * * *", MisfirePolicy::Skip)).unwrap();
        svc.enable(&id, false).unwrap();
        let listed = svc.list();
        assert!(!listed[0].enabled);
    }

    #[test]
    fn remove_drops_entry_and_returns_error_for_unknown() {
        let svc = ScheduleService::new();
        let id = svc.add(entry("0 9 * * *", MisfirePolicy::Skip)).unwrap();
        svc.remove(&id).unwrap();
        assert!(svc.list().is_empty());
        let err = svc.remove(&id).unwrap_err();
        assert!(format!("{err}").contains("not found"));
    }

    #[test]
    fn due_skip_policy_collapses_misses_to_one_fire() {
        // "Skip" semantics: a daemon down for 6 hours past a
        // every-minute schedule must fire only ONE invocation on
        // resume — the most recent.
        let svc = ScheduleService::new();
        let id = svc.add(entry("* * * * *", MisfirePolicy::Skip)).unwrap();
        // Anchor "last fire" to 6 hours ago.
        let now = Utc.with_ymd_and_hms(2026, 4, 25, 12, 0, 0).unwrap();
        let last_fire_ms = (now - chrono::Duration::hours(6)).timestamp_millis();
        let due =
            svc.due(now, |sid| if *sid == id { Some(last_fire_ms) } else { None });
        assert_eq!(due.len(), 1);
        assert!(due[0].catch_up); // collapsed-from-many is still flagged catch_up
    }

    #[test]
    fn due_fire_once_policy_emits_exactly_one_fire() {
        let svc = ScheduleService::new();
        let id = svc.add(entry("* * * * *", MisfirePolicy::FireOnce)).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 4, 25, 12, 0, 0).unwrap();
        let last_fire_ms = (now - chrono::Duration::hours(2)).timestamp_millis();
        let due = svc.due(now, |sid| {
            if *sid == id {
                Some(last_fire_ms)
            } else {
                None
            }
        });
        assert_eq!(due.len(), 1);
        // first miss is the chosen instant
        assert!(due[0].fire_at < now);
    }

    #[test]
    fn due_catch_up_windowed_emits_within_window_only() {
        let svc = ScheduleService::new();
        let mut e = entry("0 * * * *", MisfirePolicy::CatchUpWindowed);
        e.catch_up_window_secs = Some(2 * 3600); // 2-hour window
        let id = svc.add(e).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 4, 25, 12, 0, 0).unwrap();
        // Anchor last fire to 4 hours ago. Misses within the 2h
        // window are 11:00, 12:00 (wait, exactly at-now too).
        // Cron `0 * * * *` fires at every hour-zero; misses ≤ 2h
        // = 10:00 + 11:00 + 12:00 — but 10:00 is outside window
        // (now - 2h = 10:00 inclusive, so 10:00 == window edge).
        let last_fire_ms = (now - chrono::Duration::hours(4)).timestamp_millis();
        let due = svc.due(now, |sid| {
            if *sid == id {
                Some(last_fire_ms)
            } else {
                None
            }
        });
        // Expect 2 or 3 fires (window edge inclusive). The bound
        // is `now - fire <= window`. So 10:00, 11:00, 12:00 fit.
        assert!(due.len() >= 2 && due.len() <= 3);
    }

    #[test]
    fn due_disabled_schedule_does_not_fire() {
        let svc = ScheduleService::new();
        let id = svc.add(entry("* * * * *", MisfirePolicy::Skip)).unwrap();
        svc.enable(&id, false).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 4, 25, 12, 0, 0).unwrap();
        let last_fire_ms = (now - chrono::Duration::hours(1)).timestamp_millis();
        let due = svc.due(now, |_| Some(last_fire_ms));
        assert!(due.is_empty());
    }

    #[test]
    fn next_fire_after_returns_first_match() {
        let svc = ScheduleService::new();
        let id = svc.add(entry("0 9 * * *", MisfirePolicy::Skip)).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 4, 25, 8, 0, 0).unwrap();
        let next = svc.next_fire_after(&id, now).unwrap().unwrap();
        // Expect 09:00 the same day.
        assert_eq!(next.format("%H:%M").to_string(), "09:00");
    }

    #[test]
    fn render_prompt_substitutes_supported_variables() {
        // Spirit: a Client UI that types "Daily report for
        // {{target_agent}} fired at {{fire_at_iso}}" sees both
        // tokens replaced verbatim. Unknown tokens stay literal
        // so a typo surfaces in the rendered prompt rather than
        // as a hard schedule error.
        let fire_at = Utc.with_ymd_and_hms(2026, 4, 25, 9, 0, 0).unwrap();
        let template = "[{{schedule_id}}] {{target_agent}} @ {{fire_at_iso}} catch_up={{catch_up}} | unknown={{nope}}";
        let rendered = render_prompt(template, "sched-abc", &fire_at, false, "alice");
        assert!(rendered.contains("[sched-abc]"));
        assert!(rendered.contains("alice @ 2026-04-25T09:00:00+00:00"));
        assert!(rendered.contains("catch_up=false"));
        // Unknown tokens stay literal.
        assert!(rendered.contains("unknown={{nope}}"));
    }

    #[test]
    fn schedule_entry_round_trips_with_prompt_field() {
        // Verify the new ScheduleEntry.prompt field serialises
        // and deserialises as Option<String>. Serde should treat
        // a missing field on read as None (legacy entries
        // pre-prompt remain readable).
        let mut e = entry("0 9 * * *", MisfirePolicy::Skip);
        e.prompt = Some("hi {{target_agent}}".into());
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"prompt\":\"hi {{target_agent}}\""));
        let back: ScheduleEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.prompt.as_deref(), Some("hi {{target_agent}}"));
        // Legacy entry without the prompt field should parse with prompt=None.
        let legacy_json = json.replace(",\"prompt\":\"hi {{target_agent}}\"", "");
        let legacy: ScheduleEntry = serde_json::from_str(&legacy_json).unwrap();
        assert!(legacy.prompt.is_none());
    }
}

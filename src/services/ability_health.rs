// EasyNet CLI — ability service-health monitor (daemon background service)
// ========================================================================
//
// File: src/services/ability_health.rs
// Description: Probes the external services behind manifest abilities
//              that declare a `[health]` section, runs their `[boot]`
//              script when the service is down, and publishes the
//              result as an in-memory record keyed by canonical
//              ability URA. `meta.list_abilities` reads the store and
//              stamps `health_status` / `health_detail` /
//              `health_checked_unix_ms` metadata on the matching
//              descriptors, so discovery surfaces reflect the REAL
//              backing-service state instead of just owner presence.
//
// What this is (and is not)
// -------------------------
// This is service-discovery health metadata — an advisory signal for
// catalog surfaces and operators. It is NOT an execution-correctness
// gate: the invoke path never consults the store, because a stale
// probe must not veto a real call (the call itself is the freshest
// probe there is).
//
// Scope: only manifest abilities that opt in via `[health]` are
// probed. Chat-based abilities, daemon plugins, and built-in system
// abilities have no probe — their availability IS the daemon's own
// presence, which the directory plane already reports honestly.
// Abilities that declare `cost.kind = "external_metered"` without a
// `[health]` probe are surfaced as `unmonitored`, making the gap
// visible without blocking anything.
//
// Scheduling (record-driven, per CTO review 2026-06-12)
// -----------------------------------------------------
// Each record carries its own `next_probe_unix_ms` — the loop's tick
// only asks "is any record due", all cadence policy lives in the
// record transitions:
//
//   * healthy   → next probe after `interval_seconds` (default 30 s)
//   * unhealthy → exponential backoff: 5 s, 10 s, 20 s, … capped at
//                 max(interval, 60 s), so a dead upstream is not
//                 hammered every tick while still converging fast on
//                 the first failure
//   * booting   → one short grace (5 s) then re-probe
//
// Boot runs are additionally gated by a 5-minute cooldown so a
// service that crash-loops does not get `docker start`-ed in a busy
// loop.
//
// Restart semantics: the store is in-memory only. After a daemon
// restart the first tick probes from scratch — until that first probe
// completes no record exists and no metadata is stamped, so a stale
// "healthy" can never be replayed from a previous daemon life.
//
// Key discipline: records are keyed by canonical ability URA built
// with the same `crate::ura::owner_ability_ura` builder the
// descriptor's `canonical_ability_ura()` uses — never by the bare
// `<agent>.<verb>` string, which collides across hosted-synth paths
// and breaks on agent rename.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::process::Stdio;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use crate::core::ability_spec::{AbilityManifest, BootSpec, CostKind, HealthSpec};

/// Scan cadence. The tick only *checks due-ness*; per-record cadence
/// is owned by `next_probe_unix_ms`, so a shorter tick sharpens
/// scheduling granularity without changing probe pressure.
const SCAN_TICK: Duration = Duration::from_secs(5);
/// Probe interval while healthy, when the manifest omits
/// `interval_seconds`.
pub const DEFAULT_PROBE_INTERVAL_SECS: u64 = 30;
/// Per-probe timeout when the manifest omits `timeout_seconds`.
pub const DEFAULT_PROBE_TIMEOUT_SECS: u64 = 10;
/// Boot-script timeout when the manifest omits `timeout_seconds`.
pub const DEFAULT_BOOT_TIMEOUT_SECS: u64 = 60;
/// Minimum spacing between two boot attempts for one ability.
pub const BOOT_COOLDOWN_SECS: u64 = 300;
/// First-failure retry delay; doubles per consecutive failure.
const FAILURE_BACKOFF_BASE_SECS: u64 = 5;
/// The backoff cap never drops below this even for short intervals.
const FAILURE_BACKOFF_CAP_FLOOR_SECS: u64 = 60;
/// Re-probe delay after a boot script ran (service start-up grace).
const BOOT_REPROBE_GRACE_SECS: u64 = 5;
/// Stored detail strings are capped so a chatty probe cannot bloat
/// every catalog response that embeds the detail.
const DETAIL_CAP_BYTES: usize = 512;

// ── Public read model ───────────────────────────────────────────────

/// Wire-stable status vocabulary. `as_wire_str` values are consumed
/// by the backend catalog and the Frontend badge — changing one is a
/// cross-repo change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// Last probe exited 0.
    Healthy,
    /// Last probe failed (non-zero exit, spawn error, or timeout).
    Unhealthy,
    /// A boot script ran and the service is inside its start-up
    /// grace; the next probe decides healthy/unhealthy.
    Booting,
    /// Declared `cost.kind = "external_metered"` but no `[health]`
    /// probe — the external dependency is invisible to the monitor.
    Unmonitored,
}

impl HealthStatus {
    pub fn as_wire_str(self) -> &'static str {
        match self {
            HealthStatus::Healthy => "healthy",
            HealthStatus::Unhealthy => "unhealthy",
            HealthStatus::Booting => "booting",
            HealthStatus::Unmonitored => "unmonitored",
        }
    }
}

/// One ability's monitor state. The scheduling fields
/// (`next_probe_unix_ms`, `consecutive_failures`, `last_boot_unix_ms`)
/// live on the record so the loop has no hidden side tables — a
/// snapshot IS the full monitor state for that ability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityHealthRecord {
    pub status: HealthStatus,
    /// Failure context (probe stderr tail / exit code / boot error).
    /// Empty when healthy.
    pub detail: String,
    /// When the last probe finished (or the unmonitored mark was set).
    pub checked_unix_ms: i64,
    /// Consecutive failed probes; drives the backoff. Reset on
    /// success.
    pub consecutive_failures: u32,
    /// When the boot script last ran (success or not); drives the
    /// boot cooldown.
    pub last_boot_unix_ms: Option<i64>,
    /// Next time this ability's probe is due.
    pub next_probe_unix_ms: i64,
}

/// Read one record by canonical ability URA. Used by
/// `meta.list_abilities` to stamp health metadata on descriptors.
pub fn snapshot(ability_ura: &str) -> Option<AbilityHealthRecord> {
    store().read().ok()?.get(ability_ura).cloned()
}

/// Test-only seeding hook for other modules' tests (the
/// `meta.list_abilities` catalog pass). The store is process-wide
/// and shared across parallel tests — seed under a test-unique URA
/// only.
#[cfg(test)]
pub fn seed_for_tests(ability_ura: &str, record: AbilityHealthRecord) {
    upsert(ability_ura, record);
}

// ── Store ───────────────────────────────────────────────────────────

fn store() -> &'static RwLock<BTreeMap<String, AbilityHealthRecord>> {
    static STORE: OnceLock<RwLock<BTreeMap<String, AbilityHealthRecord>>> = OnceLock::new();
    STORE.get_or_init(|| RwLock::new(BTreeMap::new()))
}

fn upsert(ability_ura: &str, record: AbilityHealthRecord) {
    if let Ok(mut map) = store().write() {
        map.insert(ability_ura.to_string(), record);
    }
}

/// Drop records whose ability disappeared from the scan (manifest
/// deleted, agent removed) so the catalog never shows health for an
/// ability that no longer exists.
fn retain_live(live: &BTreeSet<String>) {
    if let Ok(mut map) = store().write() {
        retain_in(&mut map, live);
    }
}

/// Retention rule, factored over a plain map so it is testable
/// without racing other tests on the global store.
fn retain_in(map: &mut BTreeMap<String, AbilityHealthRecord>, live: &BTreeSet<String>) {
    map.retain(|key, _| live.contains(key));
}

// ── Scan plan ───────────────────────────────────────────────────────

/// How the monitor treats one manifest. Pure classification so the
/// rule is testable without touching the registry or the filesystem.
#[derive(Debug, PartialEq, Eq)]
enum ManifestHealthClass {
    /// `[health]` declared — probe it.
    Monitored,
    /// External-metered dependency with no probe — mark, don't probe.
    UnmonitoredExternal,
    /// No external dependency declared — not the monitor's business.
    Unmanaged,
}

fn classify_manifest(manifest: &AbilityManifest) -> ManifestHealthClass {
    if manifest.health().is_some() {
        return ManifestHealthClass::Monitored;
    }
    if manifest
        .cost()
        .is_some_and(|cost| cost.kind == CostKind::ExternalMetered)
    {
        return ManifestHealthClass::UnmonitoredExternal;
    }
    ManifestHealthClass::Unmanaged
}

/// One probe-able ability discovered by the scan.
struct MonitoredAbility {
    ability_ura: String,
    health: HealthSpec,
    boot: Option<BootSpec>,
}

struct ScanPlan {
    monitored: Vec<MonitoredAbility>,
    unmonitored: Vec<String>,
    /// Every ability URA the scan saw — the store retains only these.
    live: BTreeSet<String>,
}

/// Walk the hosted-agent registry and classify every on-disk
/// manifest. Canonical ability URAs are built with the same
/// `owner_ability_ura` builder the descriptor path uses, so store
/// keys and catalog keys can never drift apart.
fn scan() -> ScanPlan {
    let mut plan = ScanPlan {
        monitored: Vec::new(),
        unmonitored: Vec::new(),
        live: BTreeSet::new(),
    };
    let Ok(registry) = crate::registry::agents::load_agents() else {
        return plan;
    };
    let local = crate::persistence::local_agents::load().unwrap_or_default();
    for (agent_name, entry) in &registry.agents {
        let Some(owner_ura) =
            crate::persistence::local_agents::lookup_hosted_ura(&local, "llm", agent_name)
        else {
            continue;
        };
        for manifest in crate::runtime::agent_ability_specs::manifests_for(agent_name, entry) {
            let class = classify_manifest(&manifest);
            if class == ManifestHealthClass::Unmanaged {
                continue;
            }
            let qualified = manifest.qualified_name(agent_name);
            let public_name = crate::ura::owner_local_ability_name(&owner_ura, &qualified);
            let Some(ability_ura) = crate::ura::owner_ability_ura(&owner_ura, &public_name) else {
                continue;
            };
            plan.live.insert(ability_ura.clone());
            match class {
                ManifestHealthClass::Monitored => plan.monitored.push(MonitoredAbility {
                    ability_ura,
                    // classify_manifest returned Monitored, so
                    // health() is Some by construction.
                    health: manifest
                        .health()
                        .cloned()
                        .expect("Monitored implies health"),
                    boot: manifest.boot().cloned(),
                }),
                ManifestHealthClass::UnmonitoredExternal => plan.unmonitored.push(ability_ura),
                ManifestHealthClass::Unmanaged => unreachable!("filtered above"),
            }
        }
    }
    plan
}

// ── Scheduling rules (pure) ─────────────────────────────────────────

/// Backoff after `consecutive_failures` failed probes: 5 s doubling
/// per failure, capped at max(interval, 60 s).
fn backoff_secs(consecutive_failures: u32, interval_secs: u64) -> u64 {
    let cap = interval_secs.max(FAILURE_BACKOFF_CAP_FLOOR_SECS);
    // Exponent clamp is overflow protection only (5 << 32 is far past
    // any real cap); the effective ceiling must come from `cap`, not
    // from the clamp, so long-interval manifests back off all the way
    // up to their own interval.
    let exponent = consecutive_failures.saturating_sub(1).min(32);
    (FAILURE_BACKOFF_BASE_SECS << exponent).min(cap)
}

/// Is this ability's probe due? Unmonitored records never probe.
fn probe_due(prev: Option<&AbilityHealthRecord>, now_ms: i64) -> bool {
    match prev {
        None => true,
        Some(record) if record.status == HealthStatus::Unmonitored => false,
        Some(record) => now_ms >= record.next_probe_unix_ms,
    }
}

/// Boot-cooldown gate: a boot may run when none ran before or the
/// last one is at least `BOOT_COOLDOWN_SECS` old.
fn boot_due(last_boot_unix_ms: Option<i64>, now_ms: i64) -> bool {
    match last_boot_unix_ms {
        None => true,
        Some(last) => now_ms.saturating_sub(last) >= (BOOT_COOLDOWN_SECS as i64) * 1000,
    }
}

// ── Script runner ───────────────────────────────────────────────────

/// Run one lifecycle script: exit 0 → `Ok`, anything else (non-zero
/// exit, spawn failure, timeout kill) → `Err(detail)`.
///
/// stderr is piped and read only after exit; a script that writes
/// more than the OS pipe buffer (~64 KiB) before exiting will block
/// and get killed at the deadline — acceptable for probes, which are
/// expected to be near-silent. stdout is discarded.
fn run_argv(argv: &[String], timeout: Duration) -> Result<(), String> {
    let Some((program, args)) = argv.split_first() else {
        return Err("empty argv".to_string());
    };
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {program:?}: {e}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stderr_tail = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let mut buf = Vec::new();
                    let _ = pipe.read_to_end(&mut buf);
                    stderr_tail = String::from_utf8_lossy(&buf).trim().to_string();
                }
                if status.success() {
                    return Ok(());
                }
                let exit = status
                    .code()
                    .map(|c| format!("exit {c}"))
                    .unwrap_or_else(|| "killed by signal".to_string());
                if stderr_tail.is_empty() {
                    return Err(exit);
                }
                return Err(cap_detail(&format!("{exit}: {stderr_tail}")));
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    // Reap — a kill without wait leaks a zombie and
                    // its fds for the daemon's lifetime.
                    let _ = child.wait();
                    return Err(format!("timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("wait failed: {e}"));
            }
        }
    }
}

fn cap_detail(detail: &str) -> String {
    if detail.len() <= DETAIL_CAP_BYTES {
        return detail.to_string();
    }
    let mut s: String = detail.chars().take(DETAIL_CAP_BYTES / 2).collect();
    s.push('…');
    s
}

fn now_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

// ── Monitor loop ────────────────────────────────────────────────────

/// Spawn the monitor thread. Called once from daemon boot; a spawn
/// failure is logged and the daemon continues without service-health
/// metadata (catalog falls back to presence-only, same as before
/// this feature existed).
pub fn spawn() {
    if let Err(e) = std::thread::Builder::new()
        .name("ability-health".into())
        .spawn(run_loop)
    {
        eprintln!("[ability-health] failed to spawn: {e}");
    }
}

fn run_loop() {
    crate::op_event!(component = ability_health, kind = monitor_started);
    loop {
        tick(now_unix_ms());
        std::thread::sleep(SCAN_TICK);
    }
}

fn tick(now_ms: i64) {
    let plan = scan();
    for ability_ura in &plan.unmonitored {
        mark_unmonitored(ability_ura, now_ms);
    }
    for ability in &plan.monitored {
        let prev = snapshot(&ability.ability_ura);
        if !probe_due(prev.as_ref(), now_ms) {
            continue;
        }
        probe_one(ability, prev, now_ms);
    }
    retain_live(&plan.live);
}

fn mark_unmonitored(ability_ura: &str, now_ms: i64) {
    if snapshot(ability_ura).is_some_and(|r| r.status == HealthStatus::Unmonitored) {
        return;
    }
    upsert(
        ability_ura,
        AbilityHealthRecord {
            status: HealthStatus::Unmonitored,
            detail: "external_metered ability declares no [health] probe".to_string(),
            checked_unix_ms: now_ms,
            consecutive_failures: 0,
            last_boot_unix_ms: None,
            next_probe_unix_ms: i64::MAX,
        },
    );
}

fn probe_one(ability: &MonitoredAbility, prev: Option<AbilityHealthRecord>, now_ms: i64) {
    let interval = ability
        .health
        .interval_seconds
        .unwrap_or(DEFAULT_PROBE_INTERVAL_SECS);
    let probe_timeout = Duration::from_secs(
        ability
            .health
            .timeout_seconds
            .unwrap_or(DEFAULT_PROBE_TIMEOUT_SECS),
    );
    let was_failing = prev
        .as_ref()
        .is_some_and(|r| matches!(r.status, HealthStatus::Unhealthy | HealthStatus::Booting));
    let last_boot = prev.as_ref().and_then(|r| r.last_boot_unix_ms);

    match run_argv(&ability.health.argv, probe_timeout) {
        Ok(()) => {
            if was_failing {
                crate::op_event!(
                    component = ability_health,
                    kind = probe_recovered,
                    ability_ura = ability.ability_ura,
                );
            }
            upsert(
                &ability.ability_ura,
                AbilityHealthRecord {
                    status: HealthStatus::Healthy,
                    detail: String::new(),
                    checked_unix_ms: now_unix_ms(),
                    consecutive_failures: 0,
                    last_boot_unix_ms: last_boot,
                    next_probe_unix_ms: now_unix_ms() + (interval as i64) * 1000,
                },
            );
        }
        Err(probe_detail) => {
            let failures = prev
                .as_ref()
                .map(|r| r.consecutive_failures)
                .unwrap_or(0)
                .saturating_add(1);
            if !was_failing {
                crate::op_event!(
                    component = ability_health,
                    kind = probe_failed,
                    ability_ura = ability.ability_ura,
                    detail = probe_detail,
                );
            }
            let boot_allowed = ability.boot.is_some() && boot_due(last_boot, now_ms);
            if let (Some(boot), true) = (&ability.boot, boot_allowed) {
                run_boot(ability, boot, &probe_detail, failures, now_ms);
            } else {
                upsert(
                    &ability.ability_ura,
                    AbilityHealthRecord {
                        status: HealthStatus::Unhealthy,
                        detail: cap_detail(&probe_detail),
                        checked_unix_ms: now_unix_ms(),
                        consecutive_failures: failures,
                        last_boot_unix_ms: last_boot,
                        next_probe_unix_ms: now_unix_ms()
                            + (backoff_secs(failures, interval) as i64) * 1000,
                    },
                );
            }
        }
    }
}

fn run_boot(
    ability: &MonitoredAbility,
    boot: &BootSpec,
    probe_detail: &str,
    failures: u32,
    now_ms: i64,
) {
    // Publish `booting` BEFORE the script runs: boots can take tens
    // of seconds and a catalog read during that window should say so.
    upsert(
        &ability.ability_ura,
        AbilityHealthRecord {
            status: HealthStatus::Booting,
            detail: cap_detail(&format!("boot script running; probe: {probe_detail}")),
            checked_unix_ms: now_ms,
            consecutive_failures: failures,
            last_boot_unix_ms: Some(now_ms),
            next_probe_unix_ms: now_ms + (BOOT_REPROBE_GRACE_SECS as i64) * 1000,
        },
    );
    crate::op_event!(
        component = ability_health,
        kind = boot_started,
        ability_ura = ability.ability_ura,
    );
    let boot_timeout =
        Duration::from_secs(boot.timeout_seconds.unwrap_or(DEFAULT_BOOT_TIMEOUT_SECS));
    match run_argv(&boot.argv, boot_timeout) {
        Ok(()) => {
            crate::op_event!(
                component = ability_health,
                kind = boot_ok,
                ability_ura = ability.ability_ura,
            );
            // Stay `booting`; the grace-delayed re-probe decides.
        }
        Err(boot_detail) => {
            crate::op_event!(
                component = ability_health,
                kind = boot_failed,
                ability_ura = ability.ability_ura,
                detail = boot_detail,
            );
            let interval = ability
                .health
                .interval_seconds
                .unwrap_or(DEFAULT_PROBE_INTERVAL_SECS);
            upsert(
                &ability.ability_ura,
                AbilityHealthRecord {
                    status: HealthStatus::Unhealthy,
                    detail: cap_detail(&format!(
                        "boot failed: {boot_detail}; probe: {probe_detail}"
                    )),
                    checked_unix_ms: now_unix_ms(),
                    consecutive_failures: failures,
                    last_boot_unix_ms: Some(now_ms),
                    next_probe_unix_ms: now_unix_ms()
                        + (backoff_secs(failures, interval) as i64) * 1000,
                },
            );
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Scheduling and classification are pure functions tested
    //! directly; the global store is only touched with test-unique
    //! URA keys so parallel tests cannot interfere (per the
    //! no-lock-around-parallel-tests convention).

    use super::*;

    // ── backoff ─────────────────────────────────────────────────────

    #[test]
    fn backoff_doubles_from_base_and_caps_at_floor_for_short_intervals() {
        // interval 30 s → cap = max(30, 60) = 60 s
        assert_eq!(backoff_secs(1, 30), 5);
        assert_eq!(backoff_secs(2, 30), 10);
        assert_eq!(backoff_secs(3, 30), 20);
        assert_eq!(backoff_secs(4, 30), 40);
        assert_eq!(backoff_secs(5, 30), 60, "capped");
        assert_eq!(backoff_secs(50, 30), 60, "large counts stay capped");
    }

    #[test]
    fn backoff_cap_follows_interval_when_interval_exceeds_floor() {
        // interval 600 s → cap 600 s
        assert_eq!(backoff_secs(8, 600), 600);
    }

    // ── probe_due ───────────────────────────────────────────────────

    fn record(status: HealthStatus, next_probe_unix_ms: i64) -> AbilityHealthRecord {
        AbilityHealthRecord {
            status,
            detail: String::new(),
            checked_unix_ms: 0,
            consecutive_failures: 0,
            last_boot_unix_ms: None,
            next_probe_unix_ms,
        }
    }

    #[test]
    fn probe_due_is_immediate_for_unseen_abilities() {
        assert!(probe_due(None, 1_000));
    }

    #[test]
    fn probe_due_respects_next_probe_timestamp() {
        let r = record(HealthStatus::Healthy, 5_000);
        assert!(!probe_due(Some(&r), 4_999));
        assert!(probe_due(Some(&r), 5_000));
    }

    #[test]
    fn probe_due_never_fires_for_unmonitored_records() {
        let r = record(HealthStatus::Unmonitored, 0);
        assert!(!probe_due(Some(&r), i64::MAX));
    }

    // ── boot_due ────────────────────────────────────────────────────

    #[test]
    fn boot_due_allows_first_boot_and_respects_cooldown() {
        assert!(boot_due(None, 0));
        let cooldown_ms = (BOOT_COOLDOWN_SECS as i64) * 1000;
        assert!(!boot_due(Some(1_000), 1_000 + cooldown_ms - 1));
        assert!(boot_due(Some(1_000), 1_000 + cooldown_ms));
    }

    // ── classification ──────────────────────────────────────────────

    #[test]
    fn classify_health_manifest_as_monitored() {
        let toml = r#"
name = "x"
description = ""
[input_schema]
type = "object"
[health]
argv = ["svc-probe"]
"#;
        let m = AbilityManifest::from_toml_str(toml).unwrap();
        assert_eq!(classify_manifest(&m), ManifestHealthClass::Monitored);
    }

    #[test]
    fn classify_external_metered_without_health_as_unmonitored() {
        let toml = r#"
name = "x"
description = ""
[input_schema]
type = "object"
[cost]
kind = "external_metered"
"#;
        let m = AbilityManifest::from_toml_str(toml).unwrap();
        assert_eq!(
            classify_manifest(&m),
            ManifestHealthClass::UnmonitoredExternal
        );
    }

    #[test]
    fn classify_plain_manifest_as_unmanaged() {
        let toml = r#"
name = "x"
description = ""
[input_schema]
type = "object"
"#;
        let m = AbilityManifest::from_toml_str(toml).unwrap();
        assert_eq!(classify_manifest(&m), ManifestHealthClass::Unmanaged);
    }

    // ── run_argv ────────────────────────────────────────────────────

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn run_argv_exit_zero_is_ok() {
        assert!(run_argv(&argv(&["true"]), Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn run_argv_nonzero_exit_reports_code() {
        let err = run_argv(&argv(&["false"]), Duration::from_secs(5)).unwrap_err();
        assert!(err.contains("exit 1"), "got: {err}");
    }

    #[test]
    fn run_argv_kills_at_deadline() {
        let started = Instant::now();
        let err = run_argv(&argv(&["sleep", "10"]), Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("timed out"), "got: {err}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "kill must not wait for the child's natural exit"
        );
    }

    #[test]
    fn run_argv_spawn_failure_is_an_error_not_a_panic() {
        let err = run_argv(
            &argv(&["/nonexistent/easynet-test-binary"]),
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(err.contains("spawn"), "got: {err}");
    }

    // ── store round-trip (test-unique key) ──────────────────────────

    #[test]
    fn store_upsert_snapshot_round_trip() {
        let key = "easynet:///r/test/ability/store-round-trip.probe";
        let rec = record(HealthStatus::Healthy, 42);
        upsert(key, rec.clone());
        assert_eq!(snapshot(key), Some(rec));
    }

    #[test]
    fn retain_in_drops_vanished_abilities() {
        let kept = "easynet:///r/test/ability/retain.kept".to_string();
        let dropped = "easynet:///r/test/ability/retain.dropped".to_string();
        let mut map = BTreeMap::from([
            (kept.clone(), record(HealthStatus::Healthy, 0)),
            (dropped.clone(), record(HealthStatus::Healthy, 0)),
        ]);
        let live = BTreeSet::from([kept.clone()]);
        retain_in(&mut map, &live);
        assert!(map.contains_key(&kept));
        assert!(!map.contains_key(&dropped));
    }
}

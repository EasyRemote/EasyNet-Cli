// EasyNet CLI — desktop companion heartbeat observer
// ==================================================
//
// File: src/daemon/plugins/companion/heartbeat.rs
// Description: Shared status-file health classification for desktop companions.

use std::path::Path;

use super::planner::DesktopCompanionPlan;
use super::status::{CompanionObservation, CompanionObservedState};

const HEARTBEAT_FRESH_MS: u64 = 60_000;

/// Reads and classifies companion status files without owning any platform
/// supervisor behavior.
#[derive(Clone, Copy, Debug)]
pub struct CompanionStatusFileObserver {
    now_unix_ms: u64,
}

impl CompanionStatusFileObserver {
    pub fn current() -> Self {
        Self {
            now_unix_ms: current_unix_ms(),
        }
    }

    #[cfg(test)]
    pub fn at(now_unix_ms: u64) -> Self {
        Self { now_unix_ms }
    }

    pub fn observe_path(
        &self,
        plan: &DesktopCompanionPlan,
        path: &Path,
    ) -> Option<CompanionObservation> {
        let body = std::fs::read_to_string(path).ok()?;
        let value = match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(value) => value,
            Err(_) => return Some(self.health_error(plan, "status_file_invalid")),
        };

        match value.get("package_id").and_then(serde_json::Value::as_str) {
            Some(package_id) if package_id == plan.package_id => {}
            _ => return Some(self.health_error(plan, "status_file_invalid")),
        }

        match value
            .get("schema_version")
            .and_then(serde_json::Value::as_str)
        {
            Some("1") => {}
            _ => return Some(self.health_error(plan, "status_file_invalid")),
        }

        match value
            .get("package_version")
            .and_then(serde_json::Value::as_str)
        {
            Some(package_version) if package_version == plan.package_version => {}
            Some(_) => return Some(self.health_error(plan, "version_mismatch")),
            None => return Some(self.health_error(plan, "status_file_invalid")),
        }

        let pid = match value.get("pid").and_then(serde_json::Value::as_u64) {
            Some(pid) => pid,
            None => return Some(self.health_error(plan, "status_file_invalid")),
        };
        if value
            .get("started_at_unix_ms")
            .and_then(serde_json::Value::as_u64)
            .is_none()
        {
            return Some(self.health_error(plan, "status_file_invalid"));
        }
        let last_seen = value
            .get("last_seen_unix_ms")
            .and_then(serde_json::Value::as_u64);
        let observed_state = match last_seen {
            Some(last_seen) if self.now_unix_ms.saturating_sub(last_seen) <= HEARTBEAT_FRESH_MS => {
                CompanionObservedState::Running
            }
            Some(_) => CompanionObservedState::Stale,
            None => CompanionObservedState::HealthError,
        };

        Some(CompanionObservation {
            observed_state,
            pid: Some(pid),
            version: value
                .get("package_version")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            last_seen_unix_ms: last_seen,
            launch_method: Some(plan.spec.launch_method().to_string()),
            error: matches!(observed_state, CompanionObservedState::HealthError)
                .then(|| "status_file_invalid".to_string()),
        })
    }

    fn health_error(&self, plan: &DesktopCompanionPlan, code: &str) -> CompanionObservation {
        let observed_state = match code {
            "version_mismatch" => CompanionObservedState::VersionMismatch,
            _ => CompanionObservedState::HealthError,
        };
        CompanionObservation {
            observed_state,
            launch_method: Some(plan.spec.launch_method().to_string()),
            error: Some(code.to_string()),
            ..Default::default()
        }
    }
}

fn current_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use crate::daemon::plugins::companion::planner::{DesktopCompanionPlan, PlatformCompanionSpec};
    use crate::daemon::plugins::companion::status::CompanionObservedState;
    use crate::daemon::plugins::manifest::{
        PluginCompanionBootPolicy, PluginCompanionHealthMode, PluginCompanionStopPolicy,
    };

    use super::*;

    #[test]
    fn fresh_status_file_projects_running() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("status.json");
        write_status(&path, "0.1.0", 1234, 1_000);

        let observation = CompanionStatusFileObserver::at(1_500)
            .observe_path(&test_plan(), &path)
            .expect("observation");

        assert_eq!(observation.observed_state, CompanionObservedState::Running);
        assert_eq!(observation.pid, Some(1234));
        assert_eq!(observation.version.as_deref(), Some("0.1.0"));
        assert_eq!(observation.last_seen_unix_ms, Some(1_000));
    }

    #[test]
    fn stale_status_file_projects_stale() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("status.json");
        write_status(&path, "0.1.0", 1234, 1_000);

        let observation = CompanionStatusFileObserver::at(62_001)
            .observe_path(&test_plan(), &path)
            .expect("observation");

        assert_eq!(observation.observed_state, CompanionObservedState::Stale);
    }

    #[test]
    fn invalid_status_file_projects_health_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("status.json");
        std::fs::write(&path, "{broken").expect("write");

        let observation = CompanionStatusFileObserver::at(1_500)
            .observe_path(&test_plan(), &path)
            .expect("observation");

        assert_eq!(
            observation.observed_state,
            CompanionObservedState::HealthError
        );
        assert_eq!(observation.error.as_deref(), Some("status_file_invalid"));
    }

    #[test]
    fn mismatched_version_projects_version_mismatch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("status.json");
        write_status(&path, "0.2.0", 1234, 1_000);

        let observation = CompanionStatusFileObserver::at(1_500)
            .observe_path(&test_plan(), &path)
            .expect("observation");

        assert_eq!(
            observation.observed_state,
            CompanionObservedState::VersionMismatch
        );
        assert_eq!(observation.error.as_deref(), Some("version_mismatch"));
    }

    fn write_status(path: &Path, version: &str, pid: u64, last_seen_unix_ms: u64) {
        let body = serde_json::to_string(&json!({
            "schema_version": "1",
            "package_id": "test.desktop.companion",
            "package_version": version,
            "pid": pid,
            "started_at_unix_ms": 900,
            "last_seen_unix_ms": last_seen_unix_ms,
        }))
        .expect("json");
        std::fs::write(path, body).expect("write");
    }

    #[test]
    fn missing_schema_version_projects_health_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("status.json");
        std::fs::write(
            &path,
            serde_json::to_string(&json!({
                "package_id": "test.desktop.companion",
                "package_version": "0.1.0",
                "pid": 1234,
                "started_at_unix_ms": 900,
                "last_seen_unix_ms": 1_000,
            }))
            .expect("json"),
        )
        .expect("write");

        let observation = CompanionStatusFileObserver::at(1_500)
            .observe_path(&test_plan(), &path)
            .expect("observation");

        assert_eq!(
            observation.observed_state,
            CompanionObservedState::HealthError
        );
        assert_eq!(observation.error.as_deref(), Some("status_file_invalid"));
    }

    #[test]
    fn missing_pid_projects_health_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("status.json");
        std::fs::write(
            &path,
            serde_json::to_string(&json!({
                "schema_version": "1",
                "package_id": "test.desktop.companion",
                "package_version": "0.1.0",
                "started_at_unix_ms": 900,
                "last_seen_unix_ms": 1_000,
            }))
            .expect("json"),
        )
        .expect("write");

        let observation = CompanionStatusFileObserver::at(1_500)
            .observe_path(&test_plan(), &path)
            .expect("observation");

        assert_eq!(
            observation.observed_state,
            CompanionObservedState::HealthError
        );
        assert_eq!(observation.error.as_deref(), Some("status_file_invalid"));
    }

    #[test]
    fn missing_started_at_projects_health_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("status.json");
        std::fs::write(
            &path,
            serde_json::to_string(&json!({
                "schema_version": "1",
                "package_id": "test.desktop.companion",
                "package_version": "0.1.0",
                "pid": 1234,
                "last_seen_unix_ms": 1_000,
            }))
            .expect("json"),
        )
        .expect("write");

        let observation = CompanionStatusFileObserver::at(1_500)
            .observe_path(&test_plan(), &path)
            .expect("observation");

        assert_eq!(
            observation.observed_state,
            CompanionObservedState::HealthError
        );
        assert_eq!(observation.error.as_deref(), Some("status_file_invalid"));
    }

    fn test_plan() -> DesktopCompanionPlan {
        DesktopCompanionPlan {
            package_id: "test.desktop.companion".to_string(),
            package_version: "0.1.0".to_string(),
            display_name: "Test Companion".to_string(),
            package_root: PathBuf::from("/tmp/package"),
            user_home: PathBuf::from("/tmp/home"),
            platform: "windows".to_string(),
            spec: PlatformCompanionSpec::Windows {
                exe: PathBuf::from("TestCompanion.exe"),
                task_name: "TestCompanion".to_string(),
                session: "interactive_desktop".to_string(),
            },
            boot_policy: PluginCompanionBootPolicy::EnsureRunningAfterDaemonReady,
            stop_policy: PluginCompanionStopPolicy::KeepRunning,
            health: PluginCompanionHealthMode::StatusFile,
            status_file: None,
        }
    }
}

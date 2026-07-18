// EasyNet CLI — Windows desktop companion supervisor
// ==================================================

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::daemon::plugins::errors::{PluginHostError, Result};

use super::artifact::copy_dir_replacing;
use super::heartbeat::CompanionStatusFileObserver;
use super::planner::{DesktopCompanionPlan, PlatformCompanionSpec};
use super::status::{
    CompanionObservation, CompanionObservedState, CompanionSessionStatus, CompanionSupervisorState,
};
use super::{CompanionActionReport, DesktopCompanionSupervisor};

#[derive(Default)]
pub struct WindowsDesktopCompanionSupervisor;

impl WindowsDesktopCompanionSupervisor {
    pub const fn new() -> Self {
        Self
    }
}

impl DesktopCompanionSupervisor for WindowsDesktopCompanionSupervisor {
    fn platform(&self) -> &'static str {
        "windows"
    }

    fn probe_session(&self) -> CompanionSessionStatus {
        #[cfg(target_os = "windows")]
        {
            CompanionSessionStatus::Available
        }
        #[cfg(not(target_os = "windows"))]
        {
            CompanionSessionStatus::Unsupported {
                reason: "not running on Windows".to_string(),
            }
        }
    }

    fn install(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        let PlatformCompanionSpec::Windows { exe, .. } = &plan.spec else {
            return Ok(CompanionActionReport::unchanged(
                "not a Windows companion plan",
            ));
        };
        let source_dir = exe
            .parent()
            .ok_or_else(|| PluginHostError::InvalidCompanionManifest {
                id: plan.package_id.clone(),
                reason: "Windows companion executable has no parent directory".to_string(),
            })?;
        let target_dir = installed_windows_app_dir(plan, exe);
        copy_dir_replacing(source_dir, &target_dir)?;
        Ok(CompanionActionReport::changed(format!(
            "installed Windows companion at {}",
            target_dir.display()
        )))
    }

    fn enable(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        #[cfg(target_os = "windows")]
        {
            let PlatformCompanionSpec::Windows { exe, task_name, .. } = &plan.spec else {
                return Ok(CompanionActionReport::unchanged(
                    "not a Windows companion plan",
                ));
            };
            let installed_exe = installed_windows_exe_path(plan, exe);
            let installed_exe_arg = installed_exe.display().to_string();
            let status = Command::new("reg")
                .args([
                    "add",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    task_name,
                    "/t",
                    "REG_SZ",
                    "/d",
                    installed_exe_arg.as_str(),
                    "/f",
                ])
                .status()
                .map_err(|source| PluginHostError::WriteFailed {
                    path: std::path::PathBuf::from("reg"),
                    source,
                })?;
            if !status.success() {
                return Err(PluginHostError::InvalidCompanionManifest {
                    id: plan.package_id.clone(),
                    reason: format!("registry enable failed with {status}"),
                });
            }
            Ok(CompanionActionReport::changed("registered HKCU Run entry"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = plan;
            Ok(CompanionActionReport::unchanged("unsupported platform"))
        }
    }

    fn disable(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        #[cfg(target_os = "windows")]
        {
            let PlatformCompanionSpec::Windows { exe, task_name, .. } = &plan.spec else {
                return Ok(CompanionActionReport::unchanged(
                    "not a Windows companion plan",
                ));
            };
            let expected_exe = installed_windows_exe_path(plan, exe);
            if !windows_startup_entry_matches(task_name, &expected_exe) {
                return Ok(CompanionActionReport::unchanged(
                    "HKCU Run entry already absent or points at a different companion version",
                ));
            }
            let _ = Command::new("reg")
                .args([
                    "delete",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    task_name,
                    "/f",
                ])
                .status();
            Ok(CompanionActionReport::changed("removed HKCU Run entry"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = plan;
            Ok(CompanionActionReport::unchanged("unsupported platform"))
        }
    }

    fn remove(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        self.disable(plan)?;
        if let PlatformCompanionSpec::Windows { exe, .. } = &plan.spec {
            let target_dir = installed_windows_app_dir(plan, exe);
            if target_dir.exists() {
                std::fs::remove_dir_all(&target_dir).map_err(|source| {
                    PluginHostError::WriteFailed {
                        path: target_dir.clone(),
                        source,
                    }
                })?;
            }
        }
        if let Some(status_file) = &plan.status_file {
            let _ = std::fs::remove_file(status_file);
        }
        Ok(CompanionActionReport::changed(
            "removed Windows companion state",
        ))
    }

    fn start(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        let PlatformCompanionSpec::Windows { exe, .. } = &plan.spec else {
            return Ok(CompanionActionReport::unchanged(
                "not a Windows companion plan",
            ));
        };
        let installed_exe = installed_windows_exe_path(plan, exe);
        Command::new(&installed_exe)
            .spawn()
            .map_err(|source| PluginHostError::WriteFailed {
                path: installed_exe.clone(),
                source,
            })?;
        Ok(CompanionActionReport::changed("started Windows companion"))
    }

    fn stop(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        #[cfg(target_os = "windows")]
        {
            if let Some(pid) = self.observe(plan).pid {
                return stop_windows_pid(plan, pid);
            }
            let PlatformCompanionSpec::Windows { exe, .. } = &plan.spec else {
                return Ok(CompanionActionReport::unchanged(
                    "not a Windows companion plan",
                ));
            };
            let Some(image_name) = exe.file_name().and_then(|name| name.to_str()) else {
                return Err(PluginHostError::InvalidCompanionManifest {
                    id: plan.package_id.clone(),
                    reason: "Windows companion executable has no image name".to_string(),
                });
            };
            stop_windows_image(plan, image_name)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = plan;
            Ok(CompanionActionReport::unchanged("unsupported platform"))
        }
    }

    fn supervisor_state(&self, plan: &DesktopCompanionPlan) -> CompanionSupervisorState {
        if !self.probe_session().is_available() {
            return CompanionSupervisorState::UnsupportedSession;
        }
        let PlatformCompanionSpec::Windows { exe, task_name, .. } = &plan.spec else {
            return CompanionSupervisorState::UnsupportedPlatform;
        };
        if !installed_windows_exe_path(plan, exe).exists() {
            return CompanionSupervisorState::NotInstalled;
        }
        if windows_startup_entry_matches(task_name, &installed_windows_exe_path(plan, exe)) {
            CompanionSupervisorState::InstalledEnabled
        } else {
            CompanionSupervisorState::InstalledDisabled
        }
    }

    fn observe(&self, plan: &DesktopCompanionPlan) -> CompanionObservation {
        if let Some(path) = plan.status_file.as_deref() {
            if let Some(observation) =
                CompanionStatusFileObserver::current().observe_path(plan, path)
            {
                return observation;
            }
        }
        if let PlatformCompanionSpec::Windows { exe, .. } = &plan.spec {
            if let Some(pid) = find_windows_process_by_image(exe) {
                return CompanionObservation {
                    observed_state: CompanionObservedState::Running,
                    pid: Some(pid),
                    launch_method: Some(plan.spec.launch_method().to_string()),
                    ..Default::default()
                };
            }
        }
        CompanionObservation {
            observed_state: CompanionObservedState::NotRunning,
            launch_method: Some(plan.spec.launch_method().to_string()),
            ..Default::default()
        }
    }
}

fn installed_windows_exe_path(plan: &DesktopCompanionPlan, source_exe: &Path) -> PathBuf {
    installed_windows_app_dir(plan, source_exe).join(source_exe.file_name().unwrap_or_default())
}

fn installed_windows_app_dir(plan: &DesktopCompanionPlan, source_exe: &Path) -> PathBuf {
    let app_name = source_exe.file_stem().unwrap_or_default();
    plan.user_home
        .join(".easynet/apps")
        .join(&plan.package_id)
        .join(&plan.package_version)
        .join(app_name)
}

#[cfg(target_os = "windows")]
fn windows_startup_entry_matches(task_name: &str, expected_exe: &Path) -> bool {
    Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            task_name,
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|output| windows_startup_query_matches(&output, expected_exe))
}

#[cfg(not(target_os = "windows"))]
fn windows_startup_entry_matches(_task_name: &str, _expected_exe: &Path) -> bool {
    false
}

#[cfg(any(target_os = "windows", test))]
fn windows_startup_query_matches(output: &str, expected_exe: &Path) -> bool {
    let expected = expected_exe.display().to_string();
    output.lines().any(|line| {
        let normalized = line.trim().trim_matches('"');
        normalized == expected || normalized.ends_with(&expected)
    })
}

#[cfg(target_os = "windows")]
fn find_windows_process_by_image(source_exe: &Path) -> Option<u64> {
    let image_name = source_exe.file_name()?.to_string_lossy().to_string();
    let filter = format!("IMAGENAME eq {image_name}");
    let output = Command::new("tasklist")
        .args(["/FI", filter.as_str(), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_tasklist_csv_first_pid(&text, &image_name)
}

#[cfg(not(target_os = "windows"))]
fn find_windows_process_by_image(_source_exe: &Path) -> Option<u64> {
    None
}

#[cfg(any(target_os = "windows", test))]
fn parse_tasklist_csv_first_pid(output: &str, image_name: &str) -> Option<u64> {
    output.lines().find_map(|line| {
        let fields = parse_csv_record(line)?;
        if fields.len() < 2 || !fields[0].eq_ignore_ascii_case(image_name) {
            return None;
        }
        fields[1].parse::<u64>().ok()
    })
}

#[cfg(any(target_os = "windows", test))]
fn parse_csv_record(line: &str) -> Option<Vec<String>> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars().peekable();
    let mut quoted = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                let _ = chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(field);
                field = String::new();
            }
            _ => field.push(ch),
        }
    }
    if quoted {
        return None;
    }
    fields.push(field);
    Some(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasklist_parser_extracts_matching_pid() {
        let output = "\"Image Name\",\"PID\",\"Session Name\"\r\n\
                      \"EasyNetTray.exe\",\"4321\",\"Console\"\r\n\
                      \"other.exe\",\"9\",\"Console\"\r\n";

        assert_eq!(
            parse_tasklist_csv_first_pid(output, "EasyNetTray.exe"),
            Some(4321)
        );
    }

    #[test]
    fn tasklist_parser_is_case_insensitive_and_skips_other_images() {
        let output = "\"other.exe\",\"9\",\"Console\"\r\n\
                      \"EASYNETTRAY.EXE\",\"7654\",\"Console\"\r\n";

        assert_eq!(
            parse_tasklist_csv_first_pid(output, "EasyNetTray.exe"),
            Some(7654)
        );
    }

    #[test]
    fn installed_windows_paths_use_app_directory_by_exe_stem() {
        let source = PathBuf::from("dist/windows/EasyNetTray/EasyNetTray.exe");
        let plan = test_plan("0.1.0", source.clone());
        let app_dir = installed_windows_app_dir(&plan, &source);
        let exe_path = installed_windows_exe_path(&plan, &source);

        assert!(app_dir.ends_with(".easynet/apps/easynet.desktop.menubar/0.1.0/EasyNetTray"));
        assert!(exe_path
            .ends_with(".easynet/apps/easynet.desktop.menubar/0.1.0/EasyNetTray/EasyNetTray.exe"));
    }

    #[test]
    fn windows_startup_query_match_is_version_specific() {
        let source = PathBuf::from("dist/windows/EasyNetTray/EasyNetTray.exe");
        let plan = test_plan("0.1.0", source.clone());
        let expected = installed_windows_exe_path(&plan, &source);
        let matching = format!("EasyNetTray    REG_SZ    {}\r\n", expected.display());
        assert!(windows_startup_query_matches(&matching, &expected));

        let other_plan = test_plan("0.2.0", source.clone());
        let other = installed_windows_exe_path(&other_plan, &source);
        let mismatched = format!("EasyNetTray    REG_SZ    {}\r\n", other.display());
        assert!(!windows_startup_query_matches(&mismatched, &expected));
    }

    fn test_plan(version: &str, exe: PathBuf) -> DesktopCompanionPlan {
        DesktopCompanionPlan {
            package_id: "easynet.desktop.menubar".to_string(),
            package_version: version.to_string(),
            display_name: "EasyNet Tray".to_string(),
            package_root: PathBuf::from("/tmp/pkg"),
            user_home: PathBuf::from("/tmp/home"),
            platform: "windows".to_string(),
            spec: PlatformCompanionSpec::Windows {
                exe,
                task_name: "EasyNetTray".to_string(),
                session: "interactive_desktop".to_string(),
            },
            boot_policy: crate::daemon::plugins::manifest::PluginCompanionBootPolicy::EnsureRunningAfterDaemonReady,
            stop_policy: crate::daemon::plugins::manifest::PluginCompanionStopPolicy::KeepRunning,
            health: crate::daemon::plugins::manifest::PluginCompanionHealthMode::StatusFile,
            status_file: None,
        }
    }
}

#[cfg(target_os = "windows")]
fn stop_windows_pid(plan: &DesktopCompanionPlan, pid: u64) -> Result<CompanionActionReport> {
    let pid_arg = pid.to_string();
    let status = Command::new("taskkill")
        .args(["/PID", pid_arg.as_str(), "/T", "/F"])
        .status()
        .map_err(|source| PluginHostError::WriteFailed {
            path: std::path::PathBuf::from("taskkill"),
            source,
        })?;
    if status.success() {
        Ok(CompanionActionReport::changed(
            "stopped Windows companion pid",
        ))
    } else {
        Err(PluginHostError::InvalidCompanionManifest {
            id: plan.package_id.clone(),
            reason: format!("taskkill by pid failed with {status}"),
        })
    }
}

#[cfg(target_os = "windows")]
fn stop_windows_image(
    plan: &DesktopCompanionPlan,
    image_name: &str,
) -> Result<CompanionActionReport> {
    let status = Command::new("taskkill")
        .args(["/IM", image_name, "/T", "/F"])
        .status()
        .map_err(|source| PluginHostError::WriteFailed {
            path: std::path::PathBuf::from("taskkill"),
            source,
        })?;
    if status.success() {
        Ok(CompanionActionReport::changed(
            "stopped Windows companion image",
        ))
    } else {
        Err(PluginHostError::InvalidCompanionManifest {
            id: plan.package_id.clone(),
            reason: format!("taskkill by image failed with {status}"),
        })
    }
}

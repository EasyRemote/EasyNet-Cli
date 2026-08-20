// EasyNet CLI — macOS desktop companion supervisor
// =================================================

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
pub struct MacosDesktopCompanionSupervisor;

impl MacosDesktopCompanionSupervisor {
    pub const fn new() -> Self {
        Self
    }
}

impl DesktopCompanionSupervisor for MacosDesktopCompanionSupervisor {
    fn platform(&self) -> &'static str {
        "macos"
    }

    fn probe_session(&self) -> CompanionSessionStatus {
        let uid = current_uid();
        let status = Command::new("launchctl")
            .arg("print")
            .arg(format!("gui/{uid}"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match status {
            Ok(status) if status.success() => CompanionSessionStatus::Available,
            Ok(status) => CompanionSessionStatus::Unsupported {
                reason: format!("launchctl gui/{uid} unavailable: {status}"),
            },
            Err(err) => CompanionSessionStatus::Unsupported {
                reason: format!("launchctl unavailable: {err}"),
            },
        }
    }

    fn install(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        let PlatformCompanionSpec::Macos { app_bundle, .. } = &plan.spec else {
            return Ok(CompanionActionReport::unchanged(
                "not a macOS companion plan",
            ));
        };
        let target = installed_app_path(plan, app_bundle)?;
        stop_companion_processes(plan);
        copy_dir_replacing(app_bundle, &target)?;
        Ok(CompanionActionReport::changed(format!(
            "installed app bundle at {}",
            target.display()
        )))
    }

    fn enable(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        let plist = launch_agent_path(plan)?;
        if let Some(parent) = plist.parent() {
            std::fs::create_dir_all(parent).map_err(|source| PluginHostError::WriteFailed {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::write(&plist, render_launch_agent_plist(plan)?).map_err(|source| {
            PluginHostError::WriteFailed {
                path: plist.clone(),
                source,
            }
        })?;
        Ok(CompanionActionReport::changed(format!(
            "wrote LaunchAgent {}",
            plist.display()
        )))
    }

    fn disable(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        let plist = launch_agent_path(plan)?;
        if !plist.exists() {
            return Ok(CompanionActionReport::unchanged(
                "LaunchAgent already absent",
            ));
        }
        if !launch_agent_points_at_plan(&plist, plan)? {
            return Ok(CompanionActionReport::unchanged(
                "LaunchAgent points at a different companion version",
            ));
        }
        std::fs::remove_file(&plist).map_err(|source| PluginHostError::WriteFailed {
            path: plist.clone(),
            source,
        })?;
        Ok(CompanionActionReport::changed("removed LaunchAgent"))
    }

    fn remove(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        self.disable(plan)?;
        let PlatformCompanionSpec::Macos { app_bundle, .. } = &plan.spec else {
            return Ok(CompanionActionReport::unchanged(
                "not a macOS companion plan",
            ));
        };
        let target = installed_app_path(plan, app_bundle)?;
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(|source| PluginHostError::WriteFailed {
                path: target.clone(),
                source,
            })?;
        }
        if let Some(status_file) = &plan.status_file {
            let _ = std::fs::remove_file(status_file);
        }
        Ok(CompanionActionReport::changed(
            "removed macOS companion artifacts",
        ))
    }

    fn start(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        let plist = launch_agent_path(plan)?;
        let uid = current_uid();
        let label = launch_agent_label(plan)?;
        stop_stale_companion_processes(plan);
        let domain = format!("gui/{uid}");
        let service = format!("{domain}/{label}");
        let _ = Command::new("launchctl")
            .arg("bootout")
            .arg(&service)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let bootstrap = Command::new("launchctl")
            .arg("bootstrap")
            .arg(&domain)
            .arg(&plist)
            .status()
            .map_err(|source| PluginHostError::WriteFailed {
                path: PathBuf::from("launchctl"),
                source,
            })?;
        if !bootstrap.success() {
            return Err(PluginHostError::InvalidCompanionManifest {
                id: plan.package_id.clone(),
                reason: format!("launchctl bootstrap failed with {bootstrap}"),
            });
        }
        let status = Command::new("launchctl")
            .arg("kickstart")
            .arg("-k")
            .arg(&service)
            .status()
            .map_err(|source| PluginHostError::WriteFailed {
                path: PathBuf::from("launchctl"),
                source,
            })?;
        if status.success() {
            Ok(CompanionActionReport::changed("started LaunchAgent"))
        } else {
            Err(PluginHostError::InvalidCompanionManifest {
                id: plan.package_id.clone(),
                reason: format!("launchctl kickstart failed with {status}"),
            })
        }
    }

    fn stop(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        let label = launch_agent_label(plan)?;
        let uid = current_uid();
        let _ = Command::new("launchctl")
            .arg("bootout")
            .arg(format!("gui/{uid}/{label}"))
            .status();
        stop_companion_processes(plan);
        Ok(CompanionActionReport::changed("requested LaunchAgent stop"))
    }

    fn supervisor_state(&self, plan: &DesktopCompanionPlan) -> CompanionSupervisorState {
        let PlatformCompanionSpec::Macos { app_bundle, .. } = &plan.spec else {
            return CompanionSupervisorState::UnsupportedPlatform;
        };
        let installed_app = match installed_app_path(plan, app_bundle) {
            Ok(path) => path,
            Err(_) => return CompanionSupervisorState::InstallError,
        };
        let plist = match launch_agent_path(plan) {
            Ok(path) => path,
            Err(_) => return CompanionSupervisorState::InstallError,
        };
        supervisor_state_for_paths(plan, &installed_app, &plist)
    }

    fn observe(&self, plan: &DesktopCompanionPlan) -> CompanionObservation {
        observe_status_file_or_process(plan)
    }
}

fn supervisor_state_for_paths(
    plan: &DesktopCompanionPlan,
    installed_app: &Path,
    plist: &Path,
) -> CompanionSupervisorState {
    if !installed_app.exists() {
        return CompanionSupervisorState::NotInstalled;
    }
    if !plist.exists() {
        return CompanionSupervisorState::InstalledDisabled;
    }
    match launch_agent_points_at_plan(plist, plan) {
        Ok(true) => CompanionSupervisorState::InstalledEnabled,
        Ok(false) => CompanionSupervisorState::InstalledDisabled,
        Err(_) => CompanionSupervisorState::EnableError,
    }
}

pub(crate) fn render_launch_agent_plist(plan: &DesktopCompanionPlan) -> Result<String> {
    let PlatformCompanionSpec::Macos {
        app_bundle,
        launch_agent_label,
        ..
    } = &plan.spec
    else {
        return Err(PluginHostError::InvalidCompanionManifest {
            id: plan.package_id.clone(),
            reason: "not a macOS companion plan".to_string(),
        });
    };
    let installed_app = installed_app_path(plan, app_bundle)?;
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/open</string>
    <string>-g</string>
    <string>{}</string>
  </array>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <false/>
</dict>
</plist>
"#,
        xml_escape(launch_agent_label),
        xml_escape(&installed_app.display().to_string())
    ))
}

fn observe_status_file_or_process(plan: &DesktopCompanionPlan) -> CompanionObservation {
    if let Some(observation) = read_status_file(plan) {
        return observation;
    }
    let process_name = plan.spec.executable_name();
    let pid = process_name.as_deref().and_then(find_process_by_name);
    CompanionObservation {
        observed_state: if pid.is_some() {
            CompanionObservedState::Running
        } else {
            CompanionObservedState::NotRunning
        },
        pid,
        launch_method: Some(plan.spec.launch_method().to_string()),
        ..Default::default()
    }
}

fn read_status_file(plan: &DesktopCompanionPlan) -> Option<CompanionObservation> {
    let path = plan.status_file.as_deref()?;
    CompanionStatusFileObserver::current().observe_path(plan, path)
}

fn launch_agent_label(plan: &DesktopCompanionPlan) -> Result<&str> {
    match &plan.spec {
        PlatformCompanionSpec::Macos {
            launch_agent_label, ..
        } => Ok(launch_agent_label),
        _ => Err(PluginHostError::InvalidCompanionManifest {
            id: plan.package_id.clone(),
            reason: "not a macOS companion plan".to_string(),
        }),
    }
}

fn launch_agent_path(plan: &DesktopCompanionPlan) -> Result<PathBuf> {
    Ok(plan
        .user_home
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", launch_agent_label(plan)?)))
}

fn installed_app_path(plan: &DesktopCompanionPlan, source_app_bundle: &Path) -> Result<PathBuf> {
    let bundle_name =
        source_app_bundle
            .file_name()
            .ok_or_else(|| PluginHostError::InvalidCompanionManifest {
                id: plan.package_id.clone(),
                reason: format!(
                    "macOS companion app_bundle `{}` has no artifact name",
                    source_app_bundle.display()
                ),
            })?;
    Ok(plan
        .user_home
        .join(".easynet/apps")
        .join(&plan.package_id)
        .join(&plan.package_version)
        .join(bundle_name))
}

fn app_executable_path(plan: &DesktopCompanionPlan, app_bundle: &Path) -> Result<PathBuf> {
    let stem = app_bundle
        .file_stem()
        .map(|stem| stem.to_os_string())
        .ok_or_else(|| PluginHostError::InvalidCompanionManifest {
            id: plan.package_id.clone(),
            reason: format!(
                "macOS companion app_bundle `{}` has no executable stem",
                app_bundle.display()
            ),
        })?;
    Ok(app_bundle.join("Contents/MacOS").join(stem))
}

fn expected_app_bundle_path(plan: &DesktopCompanionPlan) -> Result<PathBuf> {
    match &plan.spec {
        PlatformCompanionSpec::Macos { app_bundle, .. } => installed_app_path(plan, app_bundle),
        _ => Err(PluginHostError::InvalidCompanionManifest {
            id: plan.package_id.clone(),
            reason: "not a macOS companion plan".to_string(),
        }),
    }
}

fn launch_agent_points_at_plan(plist: &Path, plan: &DesktopCompanionPlan) -> Result<bool> {
    let expected = expected_app_bundle_path(plan)?.display().to_string();
    let body = std::fs::read_to_string(plist).map_err(|source| PluginHostError::ReadFailed {
        path: plist.to_path_buf(),
        source,
    })?;
    Ok(body.contains(&expected))
}

fn stop_stale_companion_processes(plan: &DesktopCompanionPlan) {
    let PlatformCompanionSpec::Macos { app_bundle, .. } = &plan.spec else {
        return;
    };
    let expected = match installed_app_path(plan, app_bundle)
        .and_then(|installed_app| app_executable_path(plan, &installed_app))
    {
        Ok(path) => path.display().to_string(),
        Err(_) => return,
    };
    let Some(process_name) = plan.spec.executable_name() else {
        return;
    };
    for (pid, command) in find_processes_by_name(&process_name) {
        if command.contains(&expected) {
            continue;
        }
        let _ = Command::new("kill")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

fn stop_companion_processes(plan: &DesktopCompanionPlan) {
    let Some(process_name) = plan.spec.executable_name() else {
        return;
    };
    for (pid, _) in find_processes_by_name(&process_name) {
        let _ = Command::new("kill")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

fn find_process_by_name(name: &str) -> Option<u64> {
    find_processes_by_name(name)
        .into_iter()
        .next()
        .map(|(pid, _)| pid)
}

fn find_processes_by_name(name: &str) -> Vec<(u64, String)> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,comm="])
        .output()
        .ok();
    let Some(output) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter_map(|line| {
            let mut parts = line.trim().splitn(2, char::is_whitespace);
            let pid = parts.next()?.trim().parse::<u64>().ok()?;
            let command = parts.next().unwrap_or_default().to_string();
            command.contains(name).then_some((pid, command))
        })
        .collect()
}

fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::getuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::plugins::companion::planner::{DesktopCompanionPlan, PlatformCompanionSpec};
    use crate::daemon::plugins::manifest::{
        PluginCompanionBootPolicy, PluginCompanionHealthMode, PluginCompanionStopPolicy,
    };

    #[test]
    fn launch_agent_plist_opens_installed_app_bundle() {
        let plan = DesktopCompanionPlan {
            package_id: "easynet.desktop.menubar".to_string(),
            package_version: "0.1.0".to_string(),
            display_name: "EasyNet Menu Bar".to_string(),
            package_root: PathBuf::from("/tmp/pkg"),
            user_home: PathBuf::from("/tmp/home"),
            platform: "macos".to_string(),
            spec: PlatformCompanionSpec::Macos {
                bundle_id: "tech.silan.easynet.menubar".to_string(),
                app_bundle: PathBuf::from("/tmp/pkg/dist/macos/EasyNetMenuBar.app"),
                launch_agent_label: "tech.silan.easynet.menubar".to_string(),
                session: "aqua".to_string(),
            },
            boot_policy: PluginCompanionBootPolicy::EnsureRunningAfterDaemonReady,
            stop_policy: PluginCompanionStopPolicy::KeepRunning,
            health: PluginCompanionHealthMode::StatusFile,
            status_file: None,
        };

        let plist = render_launch_agent_plist(&plan).expect("plist");

        assert!(plist.contains("/usr/bin/open"));
        assert!(plist.contains("<string>-g</string>"));
        assert!(plist.contains("EasyNetMenuBar.app"));
        assert!(plist.contains(".easynet/apps/easynet.desktop.menubar/0.1.0"));
        assert!(!plist.contains("Contents/MacOS/EasyNetMenuBar"));
        assert!(plist.contains("LimitLoadToSessionType"));
    }

    #[test]
    fn launch_agent_target_check_is_version_specific() {
        let temp = tempfile::tempdir().expect("tempdir");
        let plan = test_plan("0.1.0");
        let matching = temp.path().join("matching.plist");
        std::fs::write(&matching, render_launch_agent_plist(&plan).expect("plist"))
            .expect("write matching plist");
        assert!(launch_agent_points_at_plan(&matching, &plan).expect("match"));

        let other = temp.path().join("other.plist");
        std::fs::write(
            &other,
            render_launch_agent_plist(&test_plan("0.2.0")).expect("plist"),
        )
        .expect("write other plist");
        assert!(!launch_agent_points_at_plan(&other, &plan).expect("no match"));
    }

    #[test]
    fn installed_app_path_rejects_missing_bundle_name_before_version_dir_fallback() {
        let plan = test_plan("0.1.0");
        let error = installed_app_path(&plan, Path::new("/"))
            .expect_err("bundle path without file name must fail closed");

        assert!(
            error.to_string().contains("has no artifact name"),
            "wrong error: {error}"
        );
    }

    #[test]
    fn supervisor_state_is_specific_to_installed_app_and_plist_target() {
        let temp = tempfile::tempdir().expect("tempdir");
        let plan = test_plan("0.1.0");
        let installed_app = temp.path().join("EasyNetMenuBar.app");
        let plist = temp.path().join("tech.silan.easynet.menubar.plist");

        assert_eq!(
            supervisor_state_for_paths(&plan, &installed_app, &plist),
            CompanionSupervisorState::NotInstalled
        );

        std::fs::create_dir_all(&installed_app).expect("installed app");
        assert_eq!(
            supervisor_state_for_paths(&plan, &installed_app, &plist),
            CompanionSupervisorState::InstalledDisabled
        );

        std::fs::write(
            &plist,
            render_launch_agent_plist(&test_plan("0.2.0")).expect("other plist"),
        )
        .expect("write mismatched plist");
        assert_eq!(
            supervisor_state_for_paths(&plan, &installed_app, &plist),
            CompanionSupervisorState::InstalledDisabled
        );

        std::fs::write(
            &plist,
            render_launch_agent_plist(&plan).expect("matching plist"),
        )
        .expect("write matching plist");
        assert_eq!(
            supervisor_state_for_paths(&plan, &installed_app, &plist),
            CompanionSupervisorState::InstalledEnabled
        );
    }

    fn test_plan(version: &str) -> DesktopCompanionPlan {
        DesktopCompanionPlan {
            package_id: "easynet.desktop.menubar".to_string(),
            package_version: version.to_string(),
            display_name: "EasyNet Menu Bar".to_string(),
            package_root: PathBuf::from("/tmp/pkg"),
            user_home: PathBuf::from("/tmp/home"),
            platform: "macos".to_string(),
            spec: PlatformCompanionSpec::Macos {
                bundle_id: "tech.silan.easynet.menubar".to_string(),
                app_bundle: PathBuf::from("/tmp/pkg/dist/macos/EasyNetMenuBar.app"),
                launch_agent_label: "tech.silan.easynet.menubar".to_string(),
                session: "aqua".to_string(),
            },
            boot_policy: PluginCompanionBootPolicy::EnsureRunningAfterDaemonReady,
            stop_policy: PluginCompanionStopPolicy::KeepRunning,
            health: PluginCompanionHealthMode::StatusFile,
            status_file: None,
        }
    }
}

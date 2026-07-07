// EasyNet CLI — desktop companion session probe
// =============================================
//
// File: src/daemon/plugins/companion/session.rs
// Description: Shared user-session availability checks for companion planning.

use super::status::CompanionSessionStatus;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopCompanionSessionProbe {
    linux: CompanionSessionStatus,
}

impl DesktopCompanionSessionProbe {
    pub fn current() -> Self {
        Self {
            linux: linux_session_from_current_env(),
        }
    }

    pub fn probe(&self, platform: &str) -> CompanionSessionStatus {
        match platform {
            "linux" => self.linux.clone(),
            _ => CompanionSessionStatus::Available,
        }
    }

    #[cfg(test)]
    pub fn with_linux_status(status: CompanionSessionStatus) -> Self {
        Self { linux: status }
    }
}

fn linux_session_from_current_env() -> CompanionSessionStatus {
    if std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
        CompanionSessionStatus::Available
    } else {
        CompanionSessionStatus::Unsupported {
            reason: "no DISPLAY or WAYLAND_DISPLAY in environment".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_linux_platforms_are_load_planner_available() {
        let probe =
            DesktopCompanionSessionProbe::with_linux_status(CompanionSessionStatus::Unsupported {
                reason: "headless".to_string(),
            });

        assert_eq!(probe.probe("macos"), CompanionSessionStatus::Available);
        assert_eq!(probe.probe("windows"), CompanionSessionStatus::Available);
    }

    #[test]
    fn linux_platform_uses_recorded_graphical_session_status() {
        let probe =
            DesktopCompanionSessionProbe::with_linux_status(CompanionSessionStatus::Unsupported {
                reason: "headless".to_string(),
            });

        assert_eq!(
            probe.probe("linux"),
            CompanionSessionStatus::Unsupported {
                reason: "headless".to_string()
            }
        );
    }
}

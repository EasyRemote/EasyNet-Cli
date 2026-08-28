// EasyNet RemoteApp native host
// =============================
//
// Owns the killable side of RemoteApp's private native-observation boundary.
// This process has no Runtime, identity, authority, session, resource, receipt,
// or WebRTC dependency. It receives a bounded versioned request, performs one
// host snapshot, validates the result, and returns one bounded response.

use std::io::{self, Read};
#[cfg(any(unix, feature = "remoteapp-e2e-fault-injection"))]
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use easynet_remoteapp_native_protocol::PARENT_LIVENESS_FD_ENV;
use easynet_remoteapp_native_protocol::{
    read_frame, write_frame, Request, Response, TargetObservationSample,
};

#[cfg(feature = "remoteapp-e2e-fault-injection")]
const TEST_FAULT_ENV: &str = "EASYNET_REMOTEAPP_NATIVE_HOST_TEST_FAULT";

pub fn run() -> anyhow::Result<()> {
    #[cfg(unix)]
    start_parent_liveness_watchdog()?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    run_protocol(stdin.lock(), stdout.lock())
}

fn run_protocol(mut reader: impl Read, mut writer: impl io::Write) -> anyhow::Result<()> {
    while let Some(request) = read_frame::<Request>(&mut reader)
        .map_err(|error| anyhow::anyhow!("read native-host request: {error}"))?
    {
        request.validate()?;
        let started_at_ms = now_ms();
        #[cfg(feature = "remoteapp-e2e-fault-injection")]
        apply_fault_injection()?;
        let observation = sample_platform_target_observations();
        observation.validate()?;
        let response = Response::target_inventory(
            &request,
            started_at_ms,
            now_ms().max(started_at_ms),
            observation,
        );
        write_frame(&mut writer, &response)
            .map_err(|error| anyhow::anyhow!("write native-host response: {error}"))?;
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(unix)]
fn start_parent_liveness_watchdog() -> anyhow::Result<()> {
    use std::os::fd::FromRawFd;

    let fd = std::env::var(PARENT_LIVENESS_FD_ENV)
        .map_err(|_| anyhow::anyhow!("RemoteApp native host missing parent liveness descriptor"))?
        .parse::<i32>()
        .map_err(|error| anyhow::anyhow!("invalid parent liveness descriptor: {error}"))?;
    anyhow::ensure!(fd >= 0, "parent liveness descriptor must be non-negative");
    let mut liveness = unsafe { std::fs::File::from_raw_fd(fd) };
    thread::Builder::new()
        .name("easynet-rd-parent-liveness".into())
        .spawn(move || {
            let mut byte = [0_u8; 1];
            loop {
                match liveness.read(&mut byte) {
                    Ok(0) | Err(_) => unsafe { libc::_exit(125) },
                    Ok(_) => continue,
                }
            }
        })
        .map_err(|error| anyhow::anyhow!("spawn parent liveness watchdog: {error}"))?;
    Ok(())
}

#[cfg(feature = "remoteapp-e2e-fault-injection")]
fn apply_fault_injection() -> anyhow::Result<()> {
    let Ok(fault) = std::env::var(TEST_FAULT_ENV) else {
        return Ok(());
    };
    if let Some(marker) = fault.strip_prefix("hang_once:") {
        anyhow::ensure!(!marker.is_empty(), "hang_once fault marker is empty");
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)
        {
            Ok(_) => loop {
                thread::park();
            },
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
            Err(error) => return Err(anyhow::anyhow!("create hang_once marker: {error}")),
        }
    }
    if fault == "hang_always" {
        loop {
            thread::park();
        }
    }
    anyhow::bail!("unsupported RemoteApp native-host fault injection {fault:?}")
}

#[cfg(feature = "native-media")]
fn sample_platform_target_observations() -> TargetObservationSample {
    #[cfg(target_os = "macos")]
    if !screen_capture_permission_granted() {
        return TargetObservationSample::permission_revoked(
            "macOS Screen Recording permission is no longer granted",
            now_ms(),
        );
    }
    match sample_xcap_target_observations() {
        Ok(observation) => observation,
        Err(error) => TargetObservationSample::snapshot_failed(
            format!("host target snapshot failed: {error}"),
            now_ms(),
        ),
    }
}

#[cfg(not(feature = "native-media"))]
fn sample_platform_target_observations() -> TargetObservationSample {
    TargetObservationSample::unsupported_platform()
}

#[cfg(feature = "native-media")]
fn sample_xcap_target_observations() -> anyhow::Result<TargetObservationSample> {
    use std::collections::BTreeSet;

    use easynet_remoteapp_native_protocol::{Geometry, ObservedWindow, VisibilityState};

    #[cfg(target_os = "linux")]
    let x11_owner_resolver = linux::LinuxX11WindowOwnerResolver::connect().ok();
    let windows = xcap::Window::all()
        .map_err(|error| anyhow::anyhow!("xcap Window::all failed: {error}"))?
        .into_iter()
        .filter_map(|window| {
            let window_id = u64::from(window.id().ok()?);
            let width = window.width().ok()?;
            let height = window.height().ok()?;
            if window_id == 0 || width == 0 || height == 0 {
                return None;
            }
            let pid = {
                #[cfg(target_os = "linux")]
                {
                    x11_owner_resolver
                        .as_ref()
                        .and_then(|resolver| {
                            resolver
                                .resolve_local_client_pid(window_id as u32)
                                .ok()
                                .flatten()
                        })
                        .or_else(|| window.pid().ok())
                        .map(i64::from)
                }
                #[cfg(not(target_os = "linux"))]
                {
                    window.pid().ok().map(i64::from)
                }
            };
            #[cfg(target_os = "linux")]
            let process_instance_id = pid.and_then(|pid| {
                u32::try_from(pid)
                    .ok()
                    .and_then(|pid| linux::LinuxProcessInstance::resolve(pid).ok())
                    .map(|instance| instance.stable_id())
            });
            #[cfg(target_os = "windows")]
            let process_instance_id = pid.and_then(|pid| {
                u32::try_from(pid)
                    .ok()
                    .and_then(|pid| windows_process_instance_id(pid).ok())
            });
            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            let process_instance_id = None;
            #[cfg(target_os = "macos")]
            let bundle_id = pid
                .and_then(|pid| u32::try_from(pid).ok())
                .and_then(bundle_id_for_pid);
            #[cfg(not(target_os = "macos"))]
            let bundle_id = window
                .app_name()
                .ok()
                .filter(|name| !name.trim().is_empty());
            #[cfg(target_os = "macos")]
            let display_id = window
                .current_monitor()
                .ok()
                .and_then(|monitor| monitor.id().ok())
                .map(u64::from);
            #[cfg(not(target_os = "macos"))]
            let display_id = None;
            Some(ObservedWindow {
                window_id,
                pid,
                process_instance_id,
                bundle_id,
                display_id,
                title: window.title().ok().filter(|title| !title.trim().is_empty()),
                focused: window.is_focused().ok() == Some(true),
                geometry: Geometry {
                    x: window.x().ok().map(f64::from),
                    y: window.y().ok().map(f64::from),
                    width: Some(f64::from(width)),
                    height: Some(f64::from(height)),
                },
                visibility_state: if window.is_minimized().ok() == Some(true) {
                    VisibilityState::Minimized
                } else {
                    VisibilityState::Visible
                },
            })
        })
        .collect();
    let display_ids = xcap::Monitor::all()
        .map_err(|error| anyhow::anyhow!("xcap Monitor::all failed: {error}"))?
        .into_iter()
        .filter_map(|monitor| monitor.id().ok().map(u64::from))
        .collect::<BTreeSet<_>>();
    Ok(TargetObservationSample::host_snapshot(windows, display_ids))
}

#[cfg(all(feature = "native-media", target_os = "windows"))]
fn windows_process_instance_id(pid: u32) -> anyhow::Result<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(anyhow::anyhow!(
            "open Windows process {pid} for creation-time proof: {}",
            io::Error::last_os_error()
        ));
    }
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let succeeded =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    unsafe { CloseHandle(process) };
    if succeeded == 0 {
        return Err(anyhow::anyhow!(
            "read Windows process {pid} creation time: {}",
            io::Error::last_os_error()
        ));
    }
    let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    anyhow::ensure!(
        ticks != 0,
        "Windows process {pid} returned a zero creation time"
    );
    Ok(format!("windows:{pid}:{ticks}"))
}

#[cfg(all(target_os = "macos", feature = "native-media"))]
fn screen_capture_permission_granted() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    unsafe { CGPreflightScreenCaptureAccess() }
}

#[cfg(all(target_os = "macos", feature = "native-media"))]
fn bundle_id_for_pid(pid: u32) -> Option<String> {
    use objc2_app_kit::NSRunningApplication;

    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as libc::pid_t)?;
    app.bundleIdentifier()
        .map(|bundle_id| bundle_id.to_string())
        .map(|bundle_id| bundle_id.trim().to_string())
        .filter(|bundle_id| !bundle_id.is_empty())
}

#[cfg(all(target_os = "linux", feature = "native-media"))]
mod linux {
    use xcb::res;

    pub(super) struct LinuxProcessInstance {
        pid: u32,
        start_ticks: u64,
        boot_id: String,
    }

    impl LinuxProcessInstance {
        pub(super) fn resolve(pid: u32) -> anyhow::Result<Self> {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map_err(|error| anyhow::anyhow!("read /proc/{pid}/stat: {error}"))?;
            let start_ticks = parse_linux_process_start_ticks(&stat)?;
            let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .map_err(|error| anyhow::anyhow!("read Linux boot id: {error}"))?
                .trim()
                .to_string();
            anyhow::ensure!(!boot_id.is_empty(), "Linux boot id is empty");
            Ok(Self {
                pid,
                start_ticks,
                boot_id,
            })
        }

        pub(super) fn stable_id(&self) -> String {
            format!("linux:{}:{}:{}", self.boot_id, self.pid, self.start_ticks)
        }
    }

    fn parse_linux_process_start_ticks(stat: &str) -> anyhow::Result<u64> {
        let command_end = stat
            .rfind(')')
            .ok_or_else(|| anyhow::anyhow!("Linux process stat is missing command terminator"))?;
        let fields = stat[command_end + 1..]
            .split_whitespace()
            .collect::<Vec<_>>();
        let start_ticks = fields
            .get(19)
            .ok_or_else(|| anyhow::anyhow!("Linux process stat is missing starttime field"))?
            .parse::<u64>()
            .map_err(|error| anyhow::anyhow!("parse Linux process starttime: {error}"))?;
        anyhow::ensure!(start_ticks > 0, "Linux process starttime must be positive");
        Ok(start_ticks)
    }

    pub(super) struct LinuxX11WindowOwnerResolver {
        connection: xcb::Connection,
    }

    impl LinuxX11WindowOwnerResolver {
        pub(super) fn connect() -> anyhow::Result<Self> {
            let (connection, _) =
                xcb::Connection::connect_with_extensions(None, &[xcb::Extension::Res], &[])
                    .map_err(|error| {
                        anyhow::anyhow!("connect to X server with X-Resource: {error}")
                    })?;
            let version = connection
                .wait_for_reply(connection.send_request(&res::QueryVersion {
                    client_major: 1,
                    client_minor: 2,
                }))
                .map_err(|error| anyhow::anyhow!("query X-Resource version: {error}"))?;
            if (version.server_major(), version.server_minor()) < (1, 2) {
                anyhow::bail!("X-Resource 1.2 is required for local client PID resolution");
            }
            Ok(Self { connection })
        }

        pub(super) fn resolve_local_client_pid(
            &self,
            window_id: u32,
        ) -> anyhow::Result<Option<u32>> {
            let specs = [res::ClientIdSpec {
                client: window_id,
                mask: res::ClientIdMask::LOCAL_CLIENT_PID,
            }];
            let reply = self
                .connection
                .wait_for_reply(
                    self.connection
                        .send_request(&res::QueryClientIds { specs: &specs }),
                )
                .map_err(|error| anyhow::anyhow!("query X-Resource PID: {error}"))?;
            Ok(reply.ids().find_map(|client_id| {
                client_id
                    .spec()
                    .mask
                    .contains(res::ClientIdMask::LOCAL_CLIENT_PID)
                    .then(|| client_id.value().first().copied())
                    .flatten()
            }))
        }
    }
}

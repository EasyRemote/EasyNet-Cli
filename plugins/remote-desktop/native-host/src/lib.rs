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

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows")
))]
use easynet_remoteapp_native_platform::PlatformWindowProcessIdentityProvider;
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

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let process_identity_provider = PlatformWindowProcessIdentityProvider::connect()?;
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
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            let process_instance = process_identity_provider
                .resolve_window(window_id)
                .ok()
                .flatten()?;
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            let pid = Some(i64::from(process_instance.pid()));
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            let process_instance_id = Some(process_instance.stable_id().to_string());
            #[cfg(target_os = "macos")]
            let pid = window.pid().ok().map(i64::from);
            #[cfg(target_os = "macos")]
            let process_instance_id = None;
            #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
            let pid = window.pid().ok().map(i64::from);
            #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
            let process_instance_id = None;
            #[cfg(target_os = "macos")]
            let bundle_id = pid
                .and_then(|pid| u32::try_from(pid).ok())
                .and_then(bundle_id_for_pid);
            #[cfg(not(target_os = "macos"))]
            let bundle_id = None;
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

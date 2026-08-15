// EasyNet CLI — remote desktop target observation provider
// ========================================================
//
// File: plugins/remote-desktop/src/target_observer.rs
// Description: Platform target observation seam for remote app/window sessions.
//
// Boundary:
// - Providers inspect local OS state and return TargetObservation values.
// - Providers do not mutate resources.json, session state, media streams, or
//   input state.
// - RemoteDesktopSession remains the only committed target lifecycle writer.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::daemon::plugins::remote_desktop::session::now_ms;
use crate::daemon::plugins::remote_desktop::session::TargetMediaSourceLost;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::target::{
    AppWindowSetProof, NativeAppIdentityCandidate, RemoteAppTargetBinding, RemoteDesktopTargetKind,
    TargetGeometry, TargetResolutionError,
};
use crate::daemon::plugins::remote_desktop::target_tracking::{
    TargetObservation, TargetTrackerSnapshot, TargetVisibilityState,
};

const PLATFORM_TARGET_SNAPSHOT_MIN_REFRESH: Duration = Duration::from_millis(250);

pub(in crate::daemon::plugins::remote_desktop) trait TargetObservationProvider {
    fn observe(
        &self,
        binding: &RemoteAppTargetBinding,
        snapshot: &TargetTrackerSnapshot,
    ) -> Option<TargetObservation>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetObservationPollResult {
    pub(in crate::daemon::plugins::remote_desktop) keep_tracking: bool,
    pub(in crate::daemon::plugins::remote_desktop) media_source_lost: Option<TargetMediaSourceLost>,
}

impl TargetObservationPollResult {
    fn keep_tracking() -> Self {
        Self {
            keep_tracking: true,
            media_source_lost: None,
        }
    }

    fn stop_tracking() -> Self {
        Self {
            keep_tracking: false,
            media_source_lost: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(in crate::daemon::plugins::remote_desktop) struct PlatformTargetObservationProvider;

#[derive(Debug, Clone)]
struct ObservedWindow {
    window_id: u64,
    pid: Option<i64>,
    bundle_id: Option<String>,
    display_id: Option<u64>,
    title: Option<String>,
    focused: bool,
    geometry: TargetGeometry,
    visibility_state: TargetVisibilityState,
}

#[derive(Debug, Clone)]
struct HostTargetSnapshot {
    windows: Vec<ObservedWindow>,
    display_ids: BTreeSet<u64>,
}

trait HostTargetSnapshotProvider {
    fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot>;
}

impl<T> HostTargetSnapshotProvider for &T
where
    T: HostTargetSnapshotProvider + ?Sized,
{
    fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot> {
        (*self).snapshot()
    }
}

#[derive(Debug)]
struct SharedHostTargetSnapshotProvider<P> {
    source: P,
    min_refresh_interval: Duration,
    cache: Mutex<Option<CachedHostTargetSnapshot>>,
}

#[derive(Debug, Clone)]
struct CachedHostTargetSnapshot {
    captured_at: Instant,
    snapshot: HostTargetSnapshot,
}

impl<P> SharedHostTargetSnapshotProvider<P> {
    fn new(source: P, min_refresh_interval: Duration) -> Self {
        Self {
            source,
            min_refresh_interval,
            cache: Mutex::new(None),
        }
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, Option<CachedHostTargetSnapshot>> {
        match self.cache.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl<P> HostTargetSnapshotProvider for SharedHostTargetSnapshotProvider<P>
where
    P: HostTargetSnapshotProvider,
{
    fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot> {
        let now = Instant::now();
        let mut cache = self.lock_cache();
        if let Some(cached) = cache.as_ref() {
            if now.duration_since(cached.captured_at) < self.min_refresh_interval {
                return Ok(cached.snapshot.clone());
            }
        }
        // The cache guard is intentionally held across enumeration. This is a
        // single-flight boundary: concurrent session ticks share one bounded
        // host snapshot instead of multiplying OS work by session count.
        let snapshot = self.source.snapshot()?;
        *cache = Some(CachedHostTargetSnapshot {
            captured_at: Instant::now(),
            snapshot: snapshot.clone(),
        });
        Ok(snapshot)
    }
}

#[derive(Debug)]
struct SnapshotBackedTargetObservationProvider<P> {
    snapshots: P,
}

impl<P> SnapshotBackedTargetObservationProvider<P> {
    fn new(snapshots: P) -> Self {
        Self { snapshots }
    }
}

impl<P> TargetObservationProvider for SnapshotBackedTargetObservationProvider<P>
where
    P: HostTargetSnapshotProvider,
{
    fn observe(
        &self,
        binding: &RemoteAppTargetBinding,
        snapshot: &TargetTrackerSnapshot,
    ) -> Option<TargetObservation> {
        let host_snapshot = match self.snapshots.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Some(TargetObservation::Lost {
                    reason: TargetResolutionError::CaptureBackendUnavailable,
                    detail: format!("host target snapshot failed: {error}"),
                    observed_at_ms: now_ms(),
                });
            }
        };
        observe_binding_against_host_snapshot(binding, snapshot, &host_snapshot)
    }
}

/// Sample one session target independently from media transport state.
///
/// Returns false only when the session is missing or terminal, allowing the
/// plugin-owned TargetMonitor to stop tracking without relying on WebRTC epoch.
pub(in crate::daemon::plugins::remote_desktop) fn observe_bound_session_target_once<P>(
    sessions: &RemoteDesktopSessionStore,
    session_id: &str,
    provider: &P,
) -> TargetObservationPollResult
where
    P: TargetObservationProvider,
{
    let Some(inputs) = sessions.target_observation_inputs_for_session(session_id) else {
        return TargetObservationPollResult::stop_tracking();
    };
    let Some(observation) = provider.observe(&inputs.binding, &inputs.snapshot) else {
        return TargetObservationPollResult::keep_tracking();
    };
    let media_source_lost = sessions.record_target_observation_for_session(
        session_id,
        &inputs.binding_id,
        inputs.binding_epoch,
        observation,
    );
    TargetObservationPollResult {
        keep_tracking: true,
        media_source_lost,
    }
}

fn observe_binding_against_host_snapshot(
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
    host_snapshot: &HostTargetSnapshot,
) -> Option<TargetObservation> {
    if let Some(selected_display_id) = binding.native_locator().display_id() {
        if !host_snapshot.display_ids.contains(&selected_display_id) {
            return Some(TargetObservation::DisplayTopologyChanged {
                available_display_ids: host_snapshot.display_ids.iter().copied().collect(),
                selected_display_available: false,
                observed_at_ms: now_ms(),
            });
        }
    }
    match binding.target_kind() {
        RemoteDesktopTargetKind::Display => None,
        RemoteDesktopTargetKind::Window => {
            observe_window(binding, snapshot, &host_snapshot.windows)
        }
        RemoteDesktopTargetKind::Application => {
            observe_application(binding, snapshot, &host_snapshot.windows)
        }
    }
}

#[cfg(any(test, not(target_os = "macos")))]
fn unsupported_platform_target_observation(
    binding: &RemoteAppTargetBinding,
) -> Option<TargetObservation> {
    match binding.target_kind() {
        RemoteDesktopTargetKind::Display => None,
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            Some(TargetObservation::Lost {
                reason: TargetResolutionError::UnsupportedCaptureScope,
                detail: format!(
                    "platform target observer cannot validate {} scoped capture",
                    binding.target_kind().as_str()
                ),
                observed_at_ms: now_ms(),
            })
        }
    }
}

fn observe_window(
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
    windows: &[ObservedWindow],
) -> Option<TargetObservation> {
    let locator = binding.native_locator();
    let expected_id = locator.window_id()?;
    let Some(window) = windows
        .iter()
        .find(|window| window.window_id == expected_id)
    else {
        return Some(lost(
            TargetResolutionError::TargetNotFound,
            "bound window is no longer present in host target snapshot",
        ));
    };
    if !owner_matches(binding, window) {
        return Some(lost(
            TargetResolutionError::TargetIdentityMismatch,
            "bound window owner identity changed",
        ));
    }
    if window.visibility_state != TargetVisibilityState::Visible {
        return Some(TargetObservation::VisibilityChanged {
            visibility_state: window.visibility_state,
            target_geometry_revision: snapshot.target_geometry_revision() + 1,
            observed_at_ms: now_ms(),
        });
    }
    if snapshot.title() != window.title.as_deref() {
        return Some(TargetObservation::TitleChanged {
            title: window.title.clone(),
            observed_at_ms: now_ms(),
        });
    }
    if snapshot.focused() != Some(window.focused) {
        return Some(TargetObservation::FocusChanged {
            focused: window.focused,
            observed_at_ms: now_ms(),
        });
    }
    geometry_observation(snapshot, window.geometry.clone())
}

fn observe_application(
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
    windows: &[ObservedWindow],
) -> Option<TargetObservation> {
    let locator = binding.native_locator();
    let expected_display = locator.display_id()?;
    let Some(committed_window_set) = binding.committed_app_window_set() else {
        return Some(lost(
            TargetResolutionError::TargetMetadataIncomplete,
            "application target binding has no committed display-scoped window set",
        ));
    };
    let matching: Vec<&ObservedWindow> = windows
        .iter()
        .filter(|window| app_owner_matches(binding, window))
        .collect();
    if matching.is_empty() {
        return Some(lost(
            TargetResolutionError::TargetNotFound,
            "bound application has no visible windows in host target snapshot",
        ));
    }
    let displays: BTreeSet<u64> = matching
        .iter()
        .filter_map(|window| window.display_id)
        .collect();
    if displays.len() > 1 {
        return Some(lost(
            TargetResolutionError::TargetMultiDisplayUnsupported,
            "bound application spans multiple displays but session is display-scoped",
        ));
    }
    let selected_display_windows: Vec<&ObservedWindow> = matching
        .into_iter()
        .filter(|window| window.display_id == Some(expected_display))
        .collect();
    if selected_display_windows.is_empty() {
        return Some(lost(
            TargetResolutionError::TargetDisplayUnavailable,
            "bound application has no windows on the selected display",
        ));
    }
    let selected_display_window_ids: BTreeSet<u64> = selected_display_windows
        .iter()
        .map(|window| window.window_id)
        .collect();
    let visible_selected_display_windows: Vec<&ObservedWindow> = selected_display_windows
        .iter()
        .copied()
        .filter(|window| window.visibility_state == TargetVisibilityState::Visible)
        .collect();
    if visible_selected_display_windows.is_empty() {
        let visibility_state = if selected_display_windows
            .iter()
            .any(|window| window.visibility_state == TargetVisibilityState::Minimized)
        {
            TargetVisibilityState::Minimized
        } else {
            TargetVisibilityState::Hidden
        };
        return Some(TargetObservation::VisibilityChanged {
            visibility_state,
            target_geometry_revision: snapshot.target_geometry_revision() + 1,
            observed_at_ms: now_ms(),
        });
    }
    let Some(geometry) = union_geometry(&visible_selected_display_windows) else {
        return Some(lost(
            TargetResolutionError::TargetMetadataIncomplete,
            "bound application window set has incomplete geometry in host target snapshot",
        ));
    };
    let current_window_set = AppWindowSetProof::new(
        expected_display,
        locator.bundle_id().map(str::to_string),
        locator.pid(),
        selected_display_window_ids.into_iter().collect(),
    );
    if &current_window_set != committed_window_set {
        return Some(TargetObservation::ApplicationWindowSetChanged {
            target_identity_epoch: current_window_set.window_set_epoch(),
            app_window_set: current_window_set,
            geometry,
            target_geometry_revision: snapshot.target_geometry_revision() + 1,
            observed_at_ms: now_ms(),
        });
    }
    geometry_observation(snapshot, geometry)
}

fn geometry_observation(
    snapshot: &TargetTrackerSnapshot,
    geometry: TargetGeometry,
) -> Option<TargetObservation> {
    if snapshot.geometry() == &geometry {
        return Some(TargetObservation::VisibilityChanged {
            visibility_state: TargetVisibilityState::Visible,
            target_geometry_revision: snapshot.target_geometry_revision(),
            observed_at_ms: now_ms(),
        });
    }
    Some(TargetObservation::GeometryChanged {
        geometry,
        target_geometry_revision: snapshot.target_geometry_revision() + 1,
        observed_at_ms: now_ms(),
    })
}

fn lost(reason: TargetResolutionError, detail: &'static str) -> TargetObservation {
    TargetObservation::Lost {
        reason,
        detail: detail.to_string(),
        observed_at_ms: now_ms(),
    }
}

fn owner_matches(binding: &RemoteAppTargetBinding, window: &ObservedWindow) -> bool {
    app_owner_matches(binding, window)
}

fn app_owner_matches(binding: &RemoteAppTargetBinding, window: &ObservedWindow) -> bool {
    binding
        .native_locator()
        .app_identity_expectation()
        .evaluate(NativeAppIdentityCandidate::new(
            window.pid,
            window.bundle_id.as_deref(),
            None,
        ))
        .matched()
}

fn union_geometry(windows: &[&ObservedWindow]) -> Option<TargetGeometry> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for window in windows {
        let x = finite_dimension(window.geometry.x)?;
        let y = finite_dimension(window.geometry.y)?;
        let width = positive_dimension(window.geometry.width)?;
        let height = positive_dimension(window.geometry.height)?;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + width);
        max_y = max_y.max(y + height);
    }
    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return None;
    }
    Some(TargetGeometry {
        x: Some(min_x),
        y: Some(min_y),
        width: Some((max_x - min_x).max(0.0)),
        height: Some((max_y - min_y).max(0.0)),
    })
}

fn finite_dimension(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite())
}

fn positive_dimension(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

#[cfg(target_os = "macos")]
mod platform {
    use std::collections::BTreeSet;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::ptr;
    use std::sync::OnceLock;

    use objc2_app_kit::{NSRunningApplication, NSWorkspace};

    use super::{
        HostTargetSnapshot, HostTargetSnapshotProvider, ObservedWindow,
        PlatformTargetObservationProvider, SharedHostTargetSnapshotProvider,
        SnapshotBackedTargetObservationProvider, TargetObservationProvider,
        PLATFORM_TARGET_SNAPSHOT_MIN_REFRESH,
    };
    use crate::daemon::plugins::remote_desktop::target::{RemoteAppTargetBinding, TargetGeometry};
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        TargetObservation, TargetTrackerSnapshot, TargetVisibilityState,
    };

    type CFArrayRef = *const c_void;
    type CFBooleanRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type CFIndex = isize;
    type CFNumberRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFTypeID = usize;
    type CFTypeRef = *const c_void;
    type CGDirectDisplayID = u32;
    type CGError = i32;
    type CGWindowID = u32;

    const KCG_NULL_WINDOW_ID: CGWindowID = 0;
    const KCG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    const KCF_NUMBER_DOUBLE_TYPE: i32 = 13;
    const KCF_NUMBER_SINT64_TYPE: i32 = 4;
    const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relativeToWindow: CGWindowID) -> CFArrayRef;
        fn CGGetDisplaysWithRect(
            rect: CGRect,
            maxDisplays: u32,
            displays: *mut CGDirectDisplayID,
            matchingDisplayCount: *mut u32,
        ) -> CGError;
        fn CGGetActiveDisplayList(
            maxDisplays: u32,
            activeDisplays: *mut CGDirectDisplayID,
            displayCount: *mut u32,
        ) -> CGError;
        fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CGRect) -> u8;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: CFIndex) -> *const c_void;
        fn CFBooleanGetTypeID() -> CFTypeID;
        fn CFBooleanGetValue(boolean: CFBooleanRef) -> u8;
        fn CFDictionaryGetValueIfPresent(
            dict: CFDictionaryRef,
            key: *const c_void,
            value: *mut *const c_void,
        ) -> u8;
        fn CFGetTypeID(value: CFTypeRef) -> CFTypeID;
        fn CFNumberGetTypeID() -> CFTypeID;
        fn CFNumberGetValue(number: CFNumberRef, theType: i32, valuePtr: *mut c_void) -> u8;
        fn CFRelease(value: *const c_void);
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            cStr: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetCString(
            theString: CFStringRef,
            buffer: *mut c_char,
            bufferSize: CFIndex,
            encoding: u32,
        ) -> u8;
        fn CFStringGetCStringPtr(theString: CFStringRef, encoding: u32) -> *const c_char;
        fn CFStringGetTypeID() -> CFTypeID;
    }

    struct CfOwned(*const c_void);

    impl CfOwned {
        fn new_string(value: &str) -> anyhow::Result<Self> {
            let value = CString::new(value)?;
            let ptr = unsafe {
                CFStringCreateWithCString(ptr::null(), value.as_ptr(), KCF_STRING_ENCODING_UTF8)
            };
            if ptr.is_null() {
                anyhow::bail!("CFStringCreateWithCString returned null");
            }
            Ok(Self(ptr))
        }

        fn as_ptr(&self) -> *const c_void {
            self.0
        }
    }

    impl Drop for CfOwned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    struct WindowKeys {
        alpha: CfOwned,
        bounds: CfOwned,
        layer: CfOwned,
        name: CfOwned,
        number: CfOwned,
        onscreen: CfOwned,
        owner_pid: CfOwned,
    }

    impl WindowKeys {
        fn new() -> anyhow::Result<Self> {
            Ok(Self {
                alpha: CfOwned::new_string("kCGWindowAlpha")?,
                bounds: CfOwned::new_string("kCGWindowBounds")?,
                layer: CfOwned::new_string("kCGWindowLayer")?,
                name: CfOwned::new_string("kCGWindowName")?,
                number: CfOwned::new_string("kCGWindowNumber")?,
                onscreen: CfOwned::new_string("kCGWindowIsOnscreen")?,
                owner_pid: CfOwned::new_string("kCGWindowOwnerPID")?,
            })
        }
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct MacOsHostTargetSnapshotProvider;

    impl TargetObservationProvider for PlatformTargetObservationProvider {
        fn observe(
            &self,
            binding: &RemoteAppTargetBinding,
            snapshot: &TargetTrackerSnapshot,
        ) -> Option<TargetObservation> {
            if !crate::daemon::plugins::remote_desktop::screencapturekit_capture::screen_capture_permission_granted() {
                return Some(TargetObservation::PermissionRevoked {
                    detail: "macOS Screen Recording permission is no longer granted".to_string(),
                    observed_at_ms: crate::daemon::plugins::remote_desktop::session::now_ms(),
                });
            }
            static SNAPSHOTS: OnceLock<
                SharedHostTargetSnapshotProvider<MacOsHostTargetSnapshotProvider>,
            > = OnceLock::new();
            let snapshots = SNAPSHOTS.get_or_init(|| {
                SharedHostTargetSnapshotProvider::new(
                    MacOsHostTargetSnapshotProvider,
                    PLATFORM_TARGET_SNAPSHOT_MIN_REFRESH,
                )
            });
            SnapshotBackedTargetObservationProvider::new(snapshots).observe(binding, snapshot)
        }
    }

    impl HostTargetSnapshotProvider for MacOsHostTargetSnapshotProvider {
        fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot> {
            Ok(HostTargetSnapshot {
                windows: observed_windows()?,
                display_ids: active_display_ids()?,
            })
        }
    }

    fn observed_windows() -> anyhow::Result<Vec<ObservedWindow>> {
        let keys = WindowKeys::new()?;
        let array = unsafe {
            CGWindowListCopyWindowInfo(KCG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS, KCG_NULL_WINDOW_ID)
        };
        if array.is_null() {
            anyhow::bail!("CGWindowListCopyWindowInfo returned null");
        }
        let array = CfOwned(array);
        let count = unsafe { CFArrayGetCount(array.as_ptr()) };
        if count < 0 {
            anyhow::bail!("CGWindowListCopyWindowInfo returned a negative count");
        }
        let mut windows = Vec::new();
        let frontmost_pid = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|application| i64::from(application.processIdentifier()));
        for idx in 0..count {
            let dict = unsafe { CFArrayGetValueAtIndex(array.as_ptr(), idx) as CFDictionaryRef };
            if dict.is_null() {
                continue;
            }
            let Some(window_id) =
                get_i64(dict, keys.number.as_ptr()).and_then(|value| u64::try_from(value).ok())
            else {
                continue;
            };
            let Some(rect) = get_rect(dict, keys.bounds.as_ptr()) else {
                continue;
            };
            let layer = get_i64(dict, keys.layer.as_ptr()).unwrap_or(0);
            let alpha = get_f64(dict, keys.alpha.as_ptr()).unwrap_or(1.0);
            let onscreen = get_bool(dict, keys.onscreen.as_ptr()).unwrap_or(false);
            let pid = get_i64(dict, keys.owner_pid.as_ptr()).filter(|value| *value >= 0);
            let bundle_id = pid
                .and_then(|pid| u32::try_from(pid).ok())
                .and_then(bundle_id_for_pid);
            windows.push(ObservedWindow {
                window_id,
                pid,
                bundle_id,
                display_id: display_id_for_rect(rect).map(u64::from),
                title: get_string(dict, keys.name.as_ptr()),
                focused: pid.is_some() && pid == frontmost_pid,
                geometry: TargetGeometry {
                    x: Some(rect.origin.x.round()),
                    y: Some(rect.origin.y.round()),
                    width: positive_dimension(rect.size.width).map(f64::from),
                    height: positive_dimension(rect.size.height).map(f64::from),
                },
                visibility_state: if layer != 0 || alpha <= 0.01 {
                    TargetVisibilityState::Hidden
                } else if !onscreen {
                    TargetVisibilityState::Minimized
                } else {
                    TargetVisibilityState::Visible
                },
            });
        }
        Ok(windows)
    }

    fn active_display_ids() -> anyhow::Result<BTreeSet<u64>> {
        let mut displays = [0_u32; 32];
        let mut count = 0_u32;
        let error = unsafe {
            CGGetActiveDisplayList(displays.len() as u32, displays.as_mut_ptr(), &mut count)
        };
        if error != 0 {
            anyhow::bail!("CGGetActiveDisplayList failed with {error}");
        }
        Ok(
            displays[..usize::try_from(count).unwrap_or(0).min(displays.len())]
                .iter()
                .copied()
                .filter(|display_id| *display_id != 0)
                .map(u64::from)
                .collect(),
        )
    }

    fn display_id_for_rect(rect: CGRect) -> Option<u32> {
        let mut displays = [0_u32; 8];
        let mut count = 0_u32;
        let error = unsafe {
            CGGetDisplaysWithRect(
                rect,
                displays.len() as u32,
                displays.as_mut_ptr(),
                &mut count,
            )
        };
        if error != 0 || count == 0 {
            return None;
        }
        displays
            .first()
            .copied()
            .filter(|display_id| *display_id != 0)
    }

    fn bundle_id_for_pid(pid: u32) -> Option<String> {
        let app =
            NSRunningApplication::runningApplicationWithProcessIdentifier(pid as libc::pid_t)?;
        app.bundleIdentifier()
            .map(|bundle_id| bundle_id.to_string())
            .map(|bundle_id| bundle_id.trim().to_string())
            .filter(|bundle_id| !bundle_id.is_empty())
    }

    fn positive_dimension(value: f64) -> Option<u32> {
        if value.is_finite() && value > 0.0 && value <= f64::from(u32::MAX) {
            Some(value.round() as u32)
        } else {
            None
        }
    }

    fn get_value(dict: CFDictionaryRef, key: *const c_void) -> Option<*const c_void> {
        let mut value = ptr::null();
        let found = unsafe { CFDictionaryGetValueIfPresent(dict, key, &mut value) };
        (found != 0 && !value.is_null()).then_some(value)
    }

    fn get_i64(dict: CFDictionaryRef, key: *const c_void) -> Option<i64> {
        let value = get_value(dict, key)? as CFNumberRef;
        let is_number = unsafe { CFGetTypeID(value) == CFNumberGetTypeID() };
        if !is_number {
            return None;
        }
        let mut out = 0_i64;
        let ok = unsafe {
            CFNumberGetValue(
                value,
                KCF_NUMBER_SINT64_TYPE,
                &mut out as *mut i64 as *mut c_void,
            )
        };
        (ok != 0).then_some(out)
    }

    fn get_bool(dict: CFDictionaryRef, key: *const c_void) -> Option<bool> {
        let value = get_value(dict, key)? as CFBooleanRef;
        let is_boolean = unsafe { CFGetTypeID(value) == CFBooleanGetTypeID() };
        is_boolean.then(|| unsafe { CFBooleanGetValue(value) != 0 })
    }

    fn get_string(dict: CFDictionaryRef, key: *const c_void) -> Option<String> {
        let value = get_value(dict, key)? as CFStringRef;
        let is_string = unsafe { CFGetTypeID(value) == CFStringGetTypeID() };
        if !is_string {
            return None;
        }
        let ptr = unsafe { CFStringGetCStringPtr(value, KCF_STRING_ENCODING_UTF8) };
        if !ptr.is_null() {
            return unsafe { CStr::from_ptr(ptr) }
                .to_str()
                .ok()
                .map(str::to_string);
        }
        let mut buffer = [0 as c_char; 4096];
        let ok = unsafe {
            CFStringGetCString(
                value,
                buffer.as_mut_ptr(),
                buffer.len() as CFIndex,
                KCF_STRING_ENCODING_UTF8,
            )
        };
        if ok == 0 {
            return None;
        }
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_str()
            .ok()
            .map(str::to_string)
    }

    fn get_f64(dict: CFDictionaryRef, key: *const c_void) -> Option<f64> {
        let value = get_value(dict, key)? as CFNumberRef;
        let is_number = unsafe { CFGetTypeID(value) == CFNumberGetTypeID() };
        if !is_number {
            return None;
        }
        let mut out = 0.0_f64;
        let ok = unsafe {
            CFNumberGetValue(
                value,
                KCF_NUMBER_DOUBLE_TYPE,
                &mut out as *mut f64 as *mut c_void,
            )
        };
        (ok != 0).then_some(out)
    }

    fn get_rect(dict: CFDictionaryRef, key: *const c_void) -> Option<CGRect> {
        let value = get_value(dict, key)? as CFDictionaryRef;
        let mut rect = CGRect::default();
        let ok = unsafe { CGRectMakeWithDictionaryRepresentation(value, &mut rect) };
        (ok != 0).then_some(rect)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    use serde_json::json;

    use super::{
        observe_binding_against_host_snapshot, unsupported_platform_target_observation,
        HostTargetSnapshot, HostTargetSnapshotProvider, ObservedWindow,
        SharedHostTargetSnapshotProvider, SnapshotBackedTargetObservationProvider,
    };
    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::remote_desktop::constants::direct_webrtc_endpoint_ura;
    use crate::daemon::plugins::remote_desktop::session::{
        RemoteDesktopSession, RemoteDesktopState,
    };
    use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
    use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
    use crate::daemon::plugins::remote_desktop::target::{
        RemoteAppTargetBinding, RemoteAppTargetResolver, ResourceEntryTargetResolver,
        TargetGeometry, TargetResolutionError,
    };
    use crate::daemon::plugins::remote_desktop::target_observer::{
        observe_bound_session_target_once, TargetObservationProvider,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        RemoteAppTargetBindingStateMachine, TargetObservation, TargetTrackerSnapshot,
        TargetVisibilityState,
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        live_remote_target_metadata, test_session_init,
    };

    struct FakeGeometryProvider;

    struct CountingObservationProvider {
        calls: Arc<AtomicUsize>,
    }

    #[derive(Debug)]
    struct CountingSnapshotProvider {
        calls: Arc<AtomicUsize>,
    }

    #[derive(Debug)]
    struct QueuedSnapshotProvider {
        snapshots: Mutex<VecDeque<HostTargetSnapshot>>,
    }

    impl QueuedSnapshotProvider {
        fn new(snapshots: Vec<HostTargetSnapshot>) -> Self {
            Self {
                snapshots: Mutex::new(VecDeque::from(snapshots)),
            }
        }
    }

    impl HostTargetSnapshotProvider for CountingSnapshotProvider {
        fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(HostTargetSnapshot {
                windows: Vec::new(),
                display_ids: BTreeSet::new(),
            })
        }
    }

    impl HostTargetSnapshotProvider for QueuedSnapshotProvider {
        fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot> {
            self.snapshots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("queued host target snapshot exhausted"))
        }
    }

    impl TargetObservationProvider for FakeGeometryProvider {
        fn observe(
            &self,
            _binding: &RemoteAppTargetBinding,
            snapshot: &TargetTrackerSnapshot,
        ) -> Option<TargetObservation> {
            Some(TargetObservation::GeometryChanged {
                geometry: TargetGeometry {
                    x: Some(10.0),
                    y: Some(20.0),
                    width: Some(300.0),
                    height: Some(200.0),
                },
                target_geometry_revision: snapshot.target_geometry_revision() + 1,
                observed_at_ms: 123,
            })
        }
    }

    impl TargetObservationProvider for CountingObservationProvider {
        fn observe(
            &self,
            _binding: &RemoteAppTargetBinding,
            _snapshot: &TargetTrackerSnapshot,
        ) -> Option<TargetObservation> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            None
        }
    }

    struct LostTargetProvider;

    impl TargetObservationProvider for LostTargetProvider {
        fn observe(
            &self,
            _binding: &RemoteAppTargetBinding,
            _snapshot: &TargetTrackerSnapshot,
        ) -> Option<TargetObservation> {
            Some(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "target disappeared from observer".into(),
                observed_at_ms: 456,
            })
        }
    }

    struct ReplacingGeometryProvider {
        store: Arc<RemoteDesktopSessionStore>,
        session_id: &'static str,
    }

    impl TargetObservationProvider for ReplacingGeometryProvider {
        fn observe(
            &self,
            _binding: &RemoteAppTargetBinding,
            snapshot: &TargetTrackerSnapshot,
        ) -> Option<TargetObservation> {
            let replacement = RemoteDesktopSession::new(test_session_init(
                self.session_id,
                "easynet:///r/acme/resource/display.reused",
                vec!["webrtc".into()],
            ));
            self.store.with_sessions(|sessions| {
                sessions.insert(self.session_id.to_string(), replacement);
            });
            Some(TargetObservation::GeometryChanged {
                geometry: TargetGeometry {
                    x: Some(10.0),
                    y: Some(20.0),
                    width: Some(300.0),
                    height: Some(200.0),
                },
                target_geometry_revision: snapshot.target_geometry_revision() + 1,
                observed_at_ms: 123,
            })
        }
    }

    fn window_binding() -> RemoteAppTargetBinding {
        ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/window.editor".to_string(),
                    owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
                    kind: ResourceType::Window,
                    binding: ResourceBinding::LocalDevice,
                    hardware_id: "window:macos:cgwindow:9001:10".to_string(),
                    display_name: "Editor window".to_string(),
                    metadata: live_remote_target_metadata(json!({
                        "platform": "macos",
                        "backend": "macos_core_graphics",
                        "window_id": 10,
                        "pid": 9001,
                        "bundle_id": "com.example.Editor",
                        "app_identity": "com.example.Editor",
                        "app_name": "Editor",
                        "title": "Old title",
                        "x": 10,
                        "y": 20,
                        "width": 100,
                        "height": 80,
                        "geometry_revision": 1,
                    })),
                    first_seen_at: "2026-06-01T00:00:00Z".to_string(),
                },
                "interactive",
                1,
            )
            .expect("window target binding resolves")
    }

    fn application_binding() -> RemoteAppTargetBinding {
        ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/application.editor".to_string(),
                    owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
                    kind: ResourceType::Application,
                    binding: ResourceBinding::LocalDevice,
                    hardware_id: "application:macos:cgwindow:42:bundle:com.example.Editor"
                        .to_string(),
                    display_name: "Editor on display 42".to_string(),
                    metadata: live_remote_target_metadata(json!({
                        "platform": "macos",
                        "backend": "macos_core_graphics",
                        "display_id": 42,
                        "bundle_id": "com.example.Editor",
                        "app_identity": "com.example.Editor",
                        "primary_pid": 9001,
                        "resolved_window_ids": [10, 11],
                        "window_set_epoch": 123,
                        "primary_x": 10,
                        "primary_y": 20,
                        "primary_width": 100,
                        "primary_height": 80,
                    })),
                    first_seen_at: "2026-06-01T00:00:00Z".to_string(),
                },
                "view_only",
                1,
            )
            .expect("application target binding resolves")
    }

    fn visible_window_snapshot() -> HostTargetSnapshot {
        HostTargetSnapshot {
            windows: vec![ObservedWindow {
                window_id: 10,
                pid: Some(9001),
                bundle_id: Some("com.example.Editor".to_string()),
                display_id: Some(42),
                title: Some("Old title".to_string()),
                focused: false,
                geometry: TargetGeometry {
                    x: Some(10.0),
                    y: Some(20.0),
                    width: Some(100.0),
                    height: Some(80.0),
                },
                visibility_state: TargetVisibilityState::Visible,
            }],
            display_ids: BTreeSet::from([42]),
        }
    }

    fn app_window(window_id: u64, x: f64, width: f64) -> ObservedWindow {
        ObservedWindow {
            window_id,
            pid: Some(9001),
            bundle_id: Some("com.example.Editor".to_string()),
            display_id: Some(42),
            title: Some(format!("Editor window {window_id}")),
            focused: false,
            geometry: TargetGeometry {
                x: Some(x),
                y: Some(20.0),
                width: Some(width),
                height: Some(80.0),
            },
            visibility_state: TargetVisibilityState::Visible,
        }
    }

    fn no_window_snapshot() -> HostTargetSnapshot {
        HostTargetSnapshot {
            windows: Vec::new(),
            display_ids: BTreeSet::from([42]),
        }
    }

    #[test]
    fn application_observer_reports_committed_window_set_drift_as_rebind() {
        let binding = application_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let extra_window = HostTargetSnapshot {
            windows: vec![
                app_window(10, 10.0, 100.0),
                app_window(11, 120.0, 100.0),
                app_window(12, 240.0, 100.0),
            ],
            display_ids: BTreeSet::from([42]),
        };

        let extra_observation =
            observe_binding_against_host_snapshot(&binding, &snapshot, &extra_window)
                .expect("window-set drift must be reported");
        match extra_observation {
            TargetObservation::ApplicationWindowSetChanged {
                app_window_set,
                geometry,
                target_identity_epoch,
                ..
            } => {
                assert_eq!(app_window_set.resolved_window_count(), 3);
                assert_eq!(target_identity_epoch, app_window_set.window_set_epoch());
                assert_eq!(geometry.width, Some(330.0));
            }
            other => panic!("window-set expansion must project as app rebind: {other:?}"),
        }

        let missing_window = HostTargetSnapshot {
            windows: vec![app_window(10, 10.0, 100.0)],
            display_ids: BTreeSet::from([42]),
        };
        let missing_observation =
            observe_binding_against_host_snapshot(&binding, &snapshot, &missing_window)
                .expect("missing committed app window must be reported");
        match missing_observation {
            TargetObservation::ApplicationWindowSetChanged {
                app_window_set,
                geometry,
                target_identity_epoch,
                ..
            } => {
                assert_eq!(app_window_set.resolved_window_count(), 1);
                assert_eq!(target_identity_epoch, app_window_set.window_set_epoch());
                assert_eq!(geometry.width, Some(100.0));
            }
            other => panic!("window-set contraction must project as app rebind: {other:?}"),
        }
    }

    #[test]
    fn observation_provider_commits_through_session_store_boundary() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let session = RemoteDesktopSession::new(test_session_init(
            "rd-provider-observation",
            "easynet:///r/acme/resource/display.provider",
            vec!["webrtc".into()],
        ));
        store.with_sessions(|sessions| {
            sessions.insert("rd-provider-observation".to_string(), session);
        });

        assert!(
            observe_bound_session_target_once(
                &store,
                "rd-provider-observation",
                &FakeGeometryProvider,
            )
            .keep_tracking
        );

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-provider-observation").unwrap();
            assert_eq!(
                session.target_tracking_state()["target_geometry_revision"],
                json!(2)
            );
            assert!(session
                .events()
                .iter()
                .any(|event| event["event_type"] == json!("TARGET_RESIZED")));
        });
    }

    #[test]
    fn bound_session_observation_does_not_require_media_transport_epoch() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let session = RemoteDesktopSession::new(test_session_init(
            "rd-bound-observation",
            "easynet:///r/acme/resource/display.bound",
            vec!["webrtc".into()],
        ));
        store.with_sessions(|sessions| {
            sessions.insert("rd-bound-observation".to_string(), session);
        });

        assert!(
            observe_bound_session_target_once(
                &store,
                "rd-bound-observation",
                &FakeGeometryProvider,
            )
            .keep_tracking
        );

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-bound-observation").unwrap();
            assert_eq!(
                session.target_tracking_state()["target_geometry_revision"],
                json!(2)
            );
            assert!(session
                .events()
                .iter()
                .any(|event| event["event_type"] == json!("TARGET_RESIZED")));
        });
    }

    #[test]
    fn observer_stops_tracking_missing_or_terminal_sessions_without_polling_provider() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = CountingObservationProvider {
            calls: Arc::clone(&calls),
        };

        let missing = observe_bound_session_target_once(&store, "rd-missing", &provider);
        assert!(
            !missing.keep_tracking,
            "missing sessions must stop target monitor tracking"
        );
        assert!(missing.media_source_lost.is_none());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "missing sessions must not poll host target state"
        );

        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-terminal-observation",
            "easynet:///r/acme/resource/display.terminal",
            vec!["webrtc".into()],
        ));
        session.close("test_terminal");
        store.with_sessions(|sessions| {
            sessions.insert("rd-terminal-observation".to_string(), session);
        });

        let terminal =
            observe_bound_session_target_once(&store, "rd-terminal-observation", &provider);
        assert!(
            !terminal.keep_tracking,
            "terminal sessions must stop target monitor tracking"
        );
        assert!(terminal.media_source_lost.is_none());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "terminal sessions must not keep polling host target state"
        );
    }

    #[test]
    fn stale_observation_cannot_commit_after_session_binding_reuse() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let session = RemoteDesktopSession::new(test_session_init(
            "rd-reused-observation",
            "easynet:///r/acme/resource/display.original",
            vec!["webrtc".into()],
        ));
        store.with_sessions(|sessions| {
            sessions.insert("rd-reused-observation".to_string(), session);
        });

        assert!(
            observe_bound_session_target_once(
                &store,
                "rd-reused-observation",
                &ReplacingGeometryProvider {
                    store: Arc::clone(&store),
                    session_id: "rd-reused-observation",
                },
            )
            .keep_tracking
        );

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-reused-observation").unwrap();
            assert_eq!(
                session.subject_ura(),
                "easynet:///r/acme/resource/display.reused"
            );
            assert_eq!(
                session.target_tracking_state()["target_geometry_revision"],
                json!(1)
            );
            assert!(!session
                .events()
                .iter()
                .any(|event| event["event_type"] == json!("TARGET_RESIZED")));
        });
    }

    #[test]
    fn lost_observation_returns_media_source_stop_effect_after_debounce() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let epoch = TransportEpoch::new(9);
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-lost-observation",
            "easynet:///r/acme/resource/display.lost",
            vec!["webrtc".into()],
        ));
        session.begin_webrtc_negotiation(epoch);
        session.mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura("rd-lost-observation"));
        store.with_sessions(|sessions| {
            sessions.insert("rd-lost-observation".to_string(), session);
        });

        let first =
            observe_bound_session_target_once(&store, "rd-lost-observation", &LostTargetProvider);
        assert!(first.keep_tracking);
        assert!(first.media_source_lost.is_none());

        let second =
            observe_bound_session_target_once(&store, "rd-lost-observation", &LostTargetProvider);
        assert!(second.keep_tracking);
        let media_source_lost = second
            .media_source_lost
            .expect("debounced target loss must surface media source stop effect");
        assert_eq!(media_source_lost.transport_epoch, epoch);
        assert_eq!(
            media_source_lost.reason,
            TargetResolutionError::TargetNotFound
        );
    }

    #[test]
    fn snapshot_observer_reappearance_requires_explicit_rebind_policy() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let epoch = TransportEpoch::new(17);
        let mut init = test_session_init(
            "rd-window-reappear",
            "easynet:///r/acme/resource/window.editor",
            vec!["webrtc".into()],
        );
        init.target_binding = window_binding();
        init.mode = "interactive".to_string();
        let mut session = RemoteDesktopSession::new(init);
        session.begin_webrtc_negotiation(epoch);
        session
            .set_local_webrtc_answer(
                epoch,
                json!({"type": "answer", "sdp": "v=0"}),
                "sck-native",
                true,
                "easynet:///r/acme/ability/remote-desktop.transport".into(),
            )
            .expect("local answer records");
        session.mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura("rd-window-reappear"));
        assert!(session.report_client_media_state(epoch, "presenting"));
        assert!(
            session.production_media_ready(),
            "fixture starts with production media online before target loss"
        );
        store.with_sessions(|sessions| {
            sessions.insert("rd-window-reappear".to_string(), session);
        });

        let provider =
            SnapshotBackedTargetObservationProvider::new(QueuedSnapshotProvider::new(vec![
                no_window_snapshot(),
                no_window_snapshot(),
                visible_window_snapshot(),
                visible_window_snapshot(),
            ]));

        let first = observe_bound_session_target_once(&store, "rd-window-reappear", &provider);
        assert!(first.keep_tracking);
        assert!(
            first.media_source_lost.is_none(),
            "first lost snapshot is debounced"
        );

        let second = observe_bound_session_target_once(&store, "rd-window-reappear", &provider);
        assert!(second.keep_tracking);
        assert_eq!(
            second
                .media_source_lost
                .expect("second lost snapshot stops media source")
                .transport_epoch,
            epoch
        );

        std::thread::sleep(Duration::from_millis(2));

        let third = observe_bound_session_target_once(&store, "rd-window-reappear", &provider);
        assert!(third.keep_tracking);
        assert!(
            third.media_source_lost.is_none(),
            "rebind attempt must not restart or stop a second media source"
        );

        let fourth = observe_bound_session_target_once(&store, "rd-window-reappear", &provider);
        assert!(fourth.keep_tracking);
        assert!(
            fourth.media_source_lost.is_none(),
            "explicit rebind failure must not revive stale transport state"
        );

        store.with_sessions(|sessions| {
            let session = sessions
                .get("rd-window-reappear")
                .expect("session remains inspectable");
            assert_eq!(session.state(), RemoteDesktopState::Suspended);
            assert_eq!(
                session.transport_state()["primary"],
                json!("media_source_lost")
            );
            assert_eq!(
                session.target_tracking_state()["status"],
                json!("lost"),
                "same window id reappearing through the platform observer is not enough to restore the binding"
            );
            assert_eq!(
                session.target_tracking_state()["input_enabled"],
                json!(false)
            );
            assert!(!session.production_media_ready());

            let events = session.events();
            let rebind_attempted = events
                .iter()
                .find(|event| event["event_type"] == json!("TARGET_REBIND_ATTEMPTED"))
                .expect("observer-visible target reappearance attempts rebind");
            assert_eq!(
                rebind_attempted["payload"]["target_status"],
                json!("rebinding")
            );
            assert_eq!(
                rebind_attempted["payload"]["frontend_action"],
                json!("retry_session")
            );

            let rebind_failed = events
                .iter()
                .find(|event| event["event_type"] == json!("TARGET_REBIND_FAILED"))
                .expect("observer-visible reappearance without explicit rebind policy fails closed");
            assert_eq!(
                rebind_failed["payload"]["reason_code"],
                json!("explicit_rebind_required")
            );
            assert_eq!(
                rebind_failed["payload"]["frontend_action"],
                json!("refresh_targets")
            );
            assert_eq!(rebind_failed["payload"]["input_enabled"], json!(false));
        });
    }

    #[test]
    fn shared_host_snapshot_provider_coalesces_session_observer_reads() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = SharedHostTargetSnapshotProvider::new(
            CountingSnapshotProvider {
                calls: Arc::clone(&calls),
            },
            Duration::from_secs(60),
        );

        provider.snapshot().expect("first host snapshot");
        provider.snapshot().expect("cached host snapshot");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "shared target observer must not multiply OS enumeration by session count"
        );
    }

    #[test]
    fn shared_host_snapshot_provider_bounds_session_fanout_to_one_enumeration_per_tick() {
        const SESSION_COUNT: usize = 128;

        let calls = Arc::new(AtomicUsize::new(0));
        let provider = SharedHostTargetSnapshotProvider::new(
            CountingSnapshotProvider {
                calls: Arc::clone(&calls),
            },
            Duration::from_secs(60),
        );

        for _session_tick in 0..SESSION_COUNT {
            provider.snapshot().expect("shared host snapshot");
        }

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "PERF-03 shared target sampler must use one host enumeration for 128 session ticks inside the same refresh window"
        );

        let expired_provider = SharedHostTargetSnapshotProvider::new(
            CountingSnapshotProvider {
                calls: Arc::clone(&calls),
            },
            Duration::ZERO,
        );
        expired_provider
            .snapshot()
            .expect("first expired-window snapshot");
        expired_provider
            .snapshot()
            .expect("second expired-window snapshot");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "PERF-03 cache expiry must allow a new bounded host enumeration instead of pinning stale target inventory forever"
        );
    }

    #[test]
    fn unsupported_platform_observer_fails_app_window_targets_closed() {
        let window_observation = unsupported_platform_target_observation(&window_binding())
            .expect("unsupported platform must fail window target closed");
        match window_observation {
            TargetObservation::Lost { reason, detail, .. } => {
                assert_eq!(reason, TargetResolutionError::UnsupportedCaptureScope);
                assert!(detail.contains("window scoped capture"));
            }
            other => panic!("expected unsupported window target loss, got {other:?}"),
        }

        let application_observation =
            unsupported_platform_target_observation(&application_binding())
                .expect("unsupported platform must fail application target closed");
        match application_observation {
            TargetObservation::Lost { reason, detail, .. } => {
                assert_eq!(reason, TargetResolutionError::UnsupportedCaptureScope);
                assert!(detail.contains("application scoped capture"));
            }
            other => panic!("expected unsupported application target loss, got {other:?}"),
        }

        let display_binding = test_session_init(
            "rd-display-unsupported-platform-observer",
            "easynet:///r/acme/resource/display.unsupported-platform",
            vec!["webrtc".into()],
        )
        .target_binding;
        assert!(
            unsupported_platform_target_observation(&display_binding).is_none(),
            "display target observation may remain a platform no-op because display capture is not app/window-scoped"
        );
    }

    #[test]
    fn window_observation_prioritizes_visibility_loss_over_title_or_focus_changes() {
        let binding = window_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let observation = observe_binding_against_host_snapshot(
            &binding,
            &snapshot,
            &HostTargetSnapshot {
                windows: vec![ObservedWindow {
                    window_id: 10,
                    pid: Some(9001),
                    bundle_id: Some("com.example.Editor".to_string()),
                    display_id: Some(42),
                    title: Some("New title while hidden".to_string()),
                    focused: true,
                    geometry: TargetGeometry {
                        x: Some(10.0),
                        y: Some(20.0),
                        width: Some(100.0),
                        height: Some(80.0),
                    },
                    visibility_state: TargetVisibilityState::Hidden,
                }],
                display_ids: BTreeSet::from([42]),
            },
        )
        .expect("hidden window observation");

        match observation {
            TargetObservation::VisibilityChanged {
                visibility_state, ..
            } => assert_eq!(visibility_state, TargetVisibilityState::Hidden),
            other => panic!(
                "hidden/minimized target availability must outrank title/focus observations, got {other:?}"
            ),
        }

        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);
        tracker
            .commit_observation(observation)
            .expect("hidden target observation commits");
        assert_eq!(tracker.snapshot().to_value()["status"], json!("stale"));
        assert!(
            tracker.snapshot().pointer_target_value().is_none(),
            "hidden window must disable pointer mapping before lower-priority title/focus updates"
        );
    }

    #[test]
    fn application_observation_tracks_display_scoped_window_set_union() {
        let binding = application_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let observation = observe_binding_against_host_snapshot(
            &binding,
            &snapshot,
            &HostTargetSnapshot {
                windows: vec![
                    ObservedWindow {
                        window_id: 10,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(10.0),
                            y: Some(20.0),
                            width: Some(100.0),
                            height: Some(80.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                    ObservedWindow {
                        window_id: 11,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(130.0),
                            y: Some(60.0),
                            width: Some(70.0),
                            height: Some(40.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                ],
                display_ids: BTreeSet::from([42]),
            },
        )
        .expect("application observation");

        match observation {
            TargetObservation::ApplicationWindowSetChanged {
                app_window_set,
                geometry,
                target_identity_epoch,
                ..
            } => {
                assert_eq!(app_window_set.resolved_window_count(), 2);
                assert_eq!(target_identity_epoch, app_window_set.window_set_epoch());
                assert_eq!(geometry.x, Some(10.0));
                assert_eq!(geometry.y, Some(20.0));
                assert_eq!(geometry.width, Some(190.0));
                assert_eq!(geometry.height, Some(80.0));
            }
            other => panic!("expected app window-set rebind with union geometry, got {other:?}"),
        }
    }

    #[test]
    fn application_observation_rebinds_same_display_window_set_expansion() {
        let binding = application_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let observation = observe_binding_against_host_snapshot(
            &binding,
            &snapshot,
            &HostTargetSnapshot {
                windows: vec![
                    ObservedWindow {
                        window_id: 10,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(10.0),
                            y: Some(20.0),
                            width: Some(100.0),
                            height: Some(80.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                    ObservedWindow {
                        window_id: 11,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(130.0),
                            y: Some(60.0),
                            width: Some(70.0),
                            height: Some(40.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                    ObservedWindow {
                        window_id: 12,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(220.0),
                            y: Some(30.0),
                            width: Some(60.0),
                            height: Some(60.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                ],
                display_ids: BTreeSet::from([42]),
            },
        )
        .expect("application window-set expansion observation");

        match observation {
            TargetObservation::ApplicationWindowSetChanged {
                app_window_set,
                geometry,
                target_identity_epoch,
                ..
            } => {
                assert_eq!(app_window_set.resolved_window_count(), 3);
                assert_eq!(target_identity_epoch, app_window_set.window_set_epoch());
                assert_eq!(geometry.width, Some(270.0));
            }
            other => panic!(
                "same-display application window-set expansion must produce rebind evidence, got {other:?}"
            ),
        }
    }

    #[test]
    fn application_observation_rebinds_same_app_window_set_subset() {
        let binding = ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/application.editor.subset"
                        .to_string(),
                    owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
                    kind: ResourceType::Application,
                    binding: ResourceBinding::LocalDevice,
                    hardware_id: "application:macos:cgwindow:42:bundle:com.example.Editor"
                        .to_string(),
                    display_name: "Editor on display 42".to_string(),
                    metadata: live_remote_target_metadata(json!({
                        "platform": "macos",
                        "backend": "macos_core_graphics",
                        "display_id": 42,
                        "bundle_id": "com.example.Editor",
                        "app_identity": "com.example.Editor",
                        "primary_pid": 9001,
                        "resolved_window_ids": [10, 11, 12],
                        "window_set_epoch": 123,
                        "primary_x": 10,
                        "primary_y": 20,
                        "primary_width": 100,
                        "primary_height": 80,
                    })),
                    first_seen_at: "2026-06-01T00:00:00Z".to_string(),
                },
                "view_only",
                1,
            )
            .expect("application target binding resolves");
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let observation = observe_binding_against_host_snapshot(
            &binding,
            &snapshot,
            &HostTargetSnapshot {
                windows: vec![ObservedWindow {
                    window_id: 10,
                    pid: Some(9001),
                    bundle_id: Some("com.example.Editor".to_string()),
                    display_id: Some(42),
                    geometry: TargetGeometry {
                        x: Some(10.0),
                        y: Some(20.0),
                        width: Some(100.0),
                        height: Some(80.0),
                    },
                    title: None,
                    focused: false,
                    visibility_state: TargetVisibilityState::Visible,
                }],
                display_ids: BTreeSet::from([42]),
            },
        )
        .expect("application subset observation");

        match observation {
            TargetObservation::ApplicationWindowSetChanged {
                app_window_set,
                geometry,
                target_identity_epoch,
                ..
            } => {
                assert_eq!(app_window_set.resolved_window_count(), 1);
                assert_eq!(target_identity_epoch, app_window_set.window_set_epoch());
                assert_eq!(geometry.width, Some(100.0));
            }
            other => panic!(
                "same app/display window-set drift must update the application binding, got {other:?}"
            ),
        }
    }

    #[test]
    fn application_observation_rejects_multi_display_window_set() {
        let binding = application_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let observation = observe_binding_against_host_snapshot(
            &binding,
            &snapshot,
            &HostTargetSnapshot {
                windows: vec![
                    ObservedWindow {
                        window_id: 10,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(10.0),
                            y: Some(20.0),
                            width: Some(100.0),
                            height: Some(80.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                    ObservedWindow {
                        window_id: 12,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(99),
                        geometry: TargetGeometry {
                            x: Some(500.0),
                            y: Some(500.0),
                            width: Some(50.0),
                            height: Some(50.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                ],
                display_ids: BTreeSet::from([42, 99]),
            },
        )
        .expect("application observation");

        match observation {
            TargetObservation::Lost { reason, .. } => {
                assert_eq!(reason.as_str(), "target_multi_display_unsupported");
            }
            other => panic!("expected multi-display application target loss, got {other:?}"),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{
        unsupported_platform_target_observation, PlatformTargetObservationProvider,
        TargetObservationProvider,
    };
    use crate::daemon::plugins::remote_desktop::target::RemoteAppTargetBinding;
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        TargetObservation, TargetTrackerSnapshot,
    };

    impl TargetObservationProvider for PlatformTargetObservationProvider {
        fn observe(
            &self,
            binding: &RemoteAppTargetBinding,
            _snapshot: &TargetTrackerSnapshot,
        ) -> Option<TargetObservation> {
            unsupported_platform_target_observation(binding)
        }
    }
}

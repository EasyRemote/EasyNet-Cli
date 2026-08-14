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
    AppWindowSetProof, RemoteAppTargetBinding, RemoteDesktopTargetKind, TargetGeometry,
    TargetResolutionError,
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
    geometry: TargetGeometry,
    visible: bool,
}

#[derive(Debug, Clone)]
struct HostTargetSnapshot {
    windows: Vec<ObservedWindow>,
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
        {
            let cache = self.lock_cache();
            if let Some(cached) = cache.as_ref() {
                if now.duration_since(cached.captured_at) < self.min_refresh_interval {
                    return Ok(cached.snapshot.clone());
                }
            }
        }
        let snapshot = self.source.snapshot()?;
        let mut cache = self.lock_cache();
        *cache = Some(CachedHostTargetSnapshot {
            captured_at: now,
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
        let host_snapshot = self.snapshots.snapshot().ok()?;
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
    if !window.visible {
        return Some(TargetObservation::VisibilityChanged {
            visibility_state: TargetVisibilityState::Hidden,
            target_geometry_revision: snapshot.target_geometry_revision() + 1,
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
    if let Some(observation) =
        application_window_set_drift_observation(binding, &selected_display_windows)
    {
        return Some(observation);
    }
    let visible_selected_display_windows: Vec<&ObservedWindow> = selected_display_windows
        .into_iter()
        .filter(|window| window.visible)
        .collect();
    if visible_selected_display_windows.is_empty() {
        return Some(TargetObservation::VisibilityChanged {
            visibility_state: TargetVisibilityState::Hidden,
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
    geometry_observation(snapshot, geometry)
}

fn application_window_set_drift_observation(
    binding: &RemoteAppTargetBinding,
    selected_display_windows: &[&ObservedWindow],
) -> Option<TargetObservation> {
    let locator = binding.native_locator();
    let expected_display = locator.display_id()?;
    let Some(committed) = binding.committed_app_window_set() else {
        return Some(lost(
            TargetResolutionError::TargetMetadataIncomplete,
            "application target binding has no committed display-scoped window set",
        ));
    };
    let live_window_ids = selected_display_windows
        .iter()
        .map(|window| window.window_id)
        .collect::<Vec<_>>();
    let live = AppWindowSetProof::new(
        expected_display,
        locator.bundle_id().map(str::to_string),
        locator.pid(),
        live_window_ids,
    );
    if committed.matches_window_identity(&live) {
        return None;
    }
    Some(lost(
        TargetResolutionError::TargetIdentityChanged,
        "bound application window set changed after session binding",
    ))
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
    let locator = binding.native_locator();
    locator.pid().is_none_or(|pid| window.pid == Some(pid))
        && locator
            .bundle_id()
            .is_none_or(|bundle| window.bundle_id.as_deref() == Some(bundle))
        && locator
            .app_identity()
            .is_none_or(|identity| window.bundle_id.as_deref() == Some(identity))
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
    use std::ffi::{c_char, c_void, CString};
    use std::ptr;
    use std::sync::OnceLock;

    use objc2_app_kit::NSRunningApplication;

    use super::{
        HostTargetSnapshot, HostTargetSnapshotProvider, ObservedWindow,
        PlatformTargetObservationProvider, SharedHostTargetSnapshotProvider,
        SnapshotBackedTargetObservationProvider, TargetObservationProvider,
        PLATFORM_TARGET_SNAPSHOT_MIN_REFRESH,
    };
    use crate::daemon::plugins::remote_desktop::target::{RemoteAppTargetBinding, TargetGeometry};
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        TargetObservation, TargetTrackerSnapshot,
    };

    type CFArrayRef = *const c_void;
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
        fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CGRect) -> u8;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: CFIndex) -> *const c_void;
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
        number: CfOwned,
        owner_pid: CfOwned,
    }

    impl WindowKeys {
        fn new() -> anyhow::Result<Self> {
            Ok(Self {
                alpha: CfOwned::new_string("kCGWindowAlpha")?,
                bounds: CfOwned::new_string("kCGWindowBounds")?,
                layer: CfOwned::new_string("kCGWindowLayer")?,
                number: CfOwned::new_string("kCGWindowNumber")?,
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
            let pid = get_i64(dict, keys.owner_pid.as_ptr()).filter(|value| *value >= 0);
            let bundle_id = pid
                .and_then(|pid| u32::try_from(pid).ok())
                .and_then(bundle_id_for_pid);
            windows.push(ObservedWindow {
                window_id,
                pid,
                bundle_id,
                display_id: display_id_for_rect(rect).map(u64::from),
                geometry: TargetGeometry {
                    x: Some(rect.origin.x.round()),
                    y: Some(rect.origin.y.round()),
                    width: positive_dimension(rect.size.width).map(f64::from),
                    height: positive_dimension(rect.size.height).map(f64::from),
                },
                visible: layer == 0 && alpha > 0.01,
            });
        }
        Ok(windows)
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;

    use super::{
        observe_binding_against_host_snapshot, HostTargetSnapshot, HostTargetSnapshotProvider,
        ObservedWindow, SharedHostTargetSnapshotProvider,
    };
    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
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
        TargetObservation, TargetTrackerSnapshot,
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        live_remote_target_metadata, test_session_init,
    };

    struct FakeGeometryProvider;

    #[derive(Debug)]
    struct CountingSnapshotProvider {
        calls: Arc<AtomicUsize>,
    }

    impl HostTargetSnapshotProvider for CountingSnapshotProvider {
        fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(HostTargetSnapshot {
                windows: Vec::new(),
            })
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
        session.mark_webrtc_media_sending(epoch, "easynet-rd://rd-lost-observation".to_string());
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
    fn application_observation_tracks_display_scoped_window_set_union() {
        let binding = ResourceEntryTargetResolver
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
            .expect("application target binding resolves");
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
                        visible: true,
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
                        visible: true,
                    },
                ],
            },
        )
        .expect("application observation");

        match observation {
            TargetObservation::GeometryChanged { geometry, .. } => {
                assert_eq!(geometry.x, Some(10.0));
                assert_eq!(geometry.y, Some(20.0));
                assert_eq!(geometry.width, Some(190.0));
                assert_eq!(geometry.height, Some(80.0));
            }
            other => panic!("expected app window-set union geometry, got {other:?}"),
        }
    }

    #[test]
    fn application_observation_rejects_committed_window_set_drift() {
        let binding = ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/application.editor.drift".to_string(),
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
            .expect("application target binding resolves");
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
                        visible: true,
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
                        visible: true,
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
                        visible: true,
                    },
                ],
            },
        )
        .expect("application observation");

        match observation {
            TargetObservation::Lost { reason, detail, .. } => {
                assert_eq!(reason.as_str(), "target_identity_changed");
                assert!(
                    detail.contains("window set changed"),
                    "unexpected drift detail: {detail}"
                );
            }
            other => panic!("expected application window-set drift to fail closed, got {other:?}"),
        }
    }

    #[test]
    fn application_observation_rejects_multi_display_window_set() {
        let binding = ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/application.editor.multi".to_string(),
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
            .expect("application target binding resolves");
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
                        visible: true,
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
                        visible: true,
                    },
                ],
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
    use super::{PlatformTargetObservationProvider, TargetObservationProvider};
    use crate::daemon::plugins::remote_desktop::target::RemoteAppTargetBinding;
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        TargetObservation, TargetTrackerSnapshot,
    };

    impl TargetObservationProvider for PlatformTargetObservationProvider {
        fn observe(
            &self,
            _binding: &RemoteAppTargetBinding,
            _snapshot: &TargetTrackerSnapshot,
        ) -> Option<TargetObservation> {
            None
        }
    }
}

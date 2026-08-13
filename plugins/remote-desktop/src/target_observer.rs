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
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, RemoteDesktopTargetKind, TargetGeometry, TargetResolutionError,
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

/// Sample one session target without holding the session-store lock while the
/// platform provider enumerates OS state.
pub(in crate::daemon::plugins::remote_desktop) fn observe_session_target_once<P>(
    sessions: &RemoteDesktopSessionStore,
    session_id: &str,
    epoch: TransportEpoch,
    provider: &P,
) where
    P: TargetObservationProvider,
{
    let Some((binding, snapshot)) = sessions.target_observation_inputs(session_id, epoch) else {
        return;
    };
    let Some(observation) = provider.observe(&binding, &snapshot) else {
        return;
    };
    sessions.record_target_observation(session_id, epoch, observation);
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
    let Some(window) = matching
        .into_iter()
        .filter(|window| window.display_id == Some(expected_display))
        .max_by_key(|window| geometry_area(&window.geometry))
    else {
        return Some(lost(
            TargetResolutionError::TargetDisplayUnavailable,
            "bound application has no windows on the selected display",
        ));
    };
    geometry_observation(snapshot, window.geometry.clone())
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

fn geometry_area(geometry: &TargetGeometry) -> u64 {
    let width = geometry.width.unwrap_or(0.0).max(0.0).round() as u64;
    let height = geometry.height.unwrap_or(0.0).max(0.0).round() as u64;
    width.saturating_mul(height)
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

    use super::{HostTargetSnapshot, HostTargetSnapshotProvider, SharedHostTargetSnapshotProvider};
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
    use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
    use crate::daemon::plugins::remote_desktop::target::{RemoteAppTargetBinding, TargetGeometry};
    use crate::daemon::plugins::remote_desktop::target_observer::{
        observe_session_target_once, TargetObservationProvider,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        TargetObservation, TargetTrackerSnapshot,
    };
    use crate::daemon::plugins::remote_desktop::test_support::test_session_init;

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

    #[test]
    fn observation_provider_commits_through_session_store_boundary() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        store.with_sessions(|sessions| {
            let mut session = RemoteDesktopSession::new(test_session_init(
                "rd-provider-observation",
                "easynet:///r/acme/resource/display.provider",
                vec!["webrtc".into()],
            ));
            session.begin_webrtc_negotiation(TransportEpoch::new(1));
            sessions.insert("rd-provider-observation".to_string(), session);
        });

        observe_session_target_once(
            &store,
            "rd-provider-observation",
            TransportEpoch::new(1),
            &FakeGeometryProvider,
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

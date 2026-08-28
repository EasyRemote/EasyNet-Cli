// EasyNet CLI — authorized RemoteApp target focus
// =================================================
//
// Focus is a low-frequency, consent-bound host state transition. It is kept
// separate from the lossy WebRTC input data plane so pointer/key frames cannot
// implicitly mutate foreground application state or bypass invocation audit.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use axon_sdk::invocation::{AxonError, AxonErrorKind, ErrorCode, ErrorStage, SecurityClass};

use crate::daemon::plugins::remote_desktop::constants::ABILITY_FOCUS_TARGET;
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, RemoteDesktopTargetKind,
};
use crate::daemon::plugins::remote_desktop::target_observer::{
    resolve_target_focus_window_id, TargetInputGuardFailure,
};
use crate::daemon::plugins::remote_desktop::target_observer::{
    validate_target_focus_observation, PlatformTargetObservationSample,
};
use crate::daemon::plugins::remote_desktop::target_snapshot::{
    TargetSnapshotDeadlineError, TargetSnapshotDeadlineExecutor, TargetSnapshotSample,
};
use crate::daemon::plugins::remote_desktop::target_tracking::TargetTrackerSnapshot;

const FOCUS_PROVIDER_DEADLINE: Duration = Duration::from_millis(250);
const FOCUS_VERIFICATION_DEADLINE: Duration = Duration::from_secs(2);
const FOCUS_VERIFICATION_INTERVAL_MS: u64 = 25;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteAppTargetFocusProof {
    platform_backend: &'static str,
    observed_at_ms: u64,
}

impl RemoteAppTargetFocusProof {
    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) const fn for_test(
        platform_backend: &'static str,
        observed_at_ms: u64,
    ) -> Self {
        Self {
            platform_backend,
            observed_at_ms,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn platform_backend(
        &self,
    ) -> &'static str {
        self.platform_backend
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn observed_at_ms(&self) -> u64 {
        self.observed_at_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TargetFocusFailureReason {
    RemoteFocusNotConsented,
    TargetFocusPermissionMissing,
    TargetFocusUnsupported,
    TargetFocusFailed,
    TargetFocusUnverified,
    TargetFocusStale,
}

impl TargetFocusFailureReason {
    pub(in crate::daemon::plugins::remote_desktop) const fn as_str(self) -> &'static str {
        match self {
            Self::RemoteFocusNotConsented => "remote_focus_not_consented",
            Self::TargetFocusPermissionMissing => "target_focus_permission_missing",
            Self::TargetFocusUnsupported => "target_focus_unsupported",
            Self::TargetFocusFailed => "target_focus_failed",
            Self::TargetFocusUnverified => "target_focus_unverified",
            Self::TargetFocusStale => "target_focus_stale",
        }
    }

    const fn frontend_action(self) -> &'static str {
        match self {
            Self::RemoteFocusNotConsented => "request_consent",
            Self::TargetFocusPermissionMissing => "request_permission",
            Self::TargetFocusUnsupported => "show_unsupported",
            Self::TargetFocusFailed | Self::TargetFocusUnverified => "retry_session",
            Self::TargetFocusStale => "refresh_targets",
        }
    }

    const fn target_event_type(self) -> &'static str {
        match self {
            Self::RemoteFocusNotConsented => "TARGET_FOCUS_DENIED",
            Self::TargetFocusPermissionMissing => "TARGET_FOCUS_PERMISSION_DENIED",
            Self::TargetFocusUnsupported => "TARGET_FOCUS_UNSUPPORTED",
            Self::TargetFocusFailed | Self::TargetFocusUnverified => "TARGET_FOCUS_FAILED",
            Self::TargetFocusStale => "TARGET_FOCUS_STALE",
        }
    }

    const fn axon_projection(self) -> (AxonErrorKind, ErrorCode, ErrorStage, SecurityClass) {
        match self {
            Self::RemoteFocusNotConsented => (
                AxonErrorKind::PermissionDenied,
                ErrorCode::AbilityForbidden,
                ErrorStage::AbilityPolicy,
                SecurityClass::Authorization,
            ),
            Self::TargetFocusPermissionMissing => (
                AxonErrorKind::PermissionDenied,
                ErrorCode::AuthorityRequired,
                ErrorStage::AuthorityValidation,
                SecurityClass::Authority,
            ),
            Self::TargetFocusStale => (
                AxonErrorKind::InvalidArgument,
                ErrorCode::RequestMetadataInvalid,
                ErrorStage::Execution,
                SecurityClass::Resource,
            ),
            Self::TargetFocusUnsupported
            | Self::TargetFocusFailed
            | Self::TargetFocusUnverified => (
                AxonErrorKind::Unavailable,
                ErrorCode::ExecutionFailed,
                ErrorStage::Execution,
                SecurityClass::Resource,
            ),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{detail}; reason={reason_code}; frontend_action={frontend_action}")]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteAppTargetFocusError {
    reason: TargetFocusFailureReason,
    reason_code: &'static str,
    frontend_action: &'static str,
    detail: String,
}

impl RemoteAppTargetFocusError {
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        reason: TargetFocusFailureReason,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            reason,
            reason_code: reason.as_str(),
            frontend_action: reason.frontend_action(),
            detail: detail.into(),
        }
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) const fn reason(
        &self,
    ) -> TargetFocusFailureReason {
        self.reason
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_axon(&self) -> AxonError {
        let (kind, code, stage, security_class) = self.reason.axon_projection();
        AxonError::new(kind)
            .with_code(code)
            .with_reason(self.reason.as_str())
            .with_stage(stage)
            .with_security_class(security_class)
            .with_context("ability", ABILITY_FOCUS_TARGET)
            .with_context("target_reason", self.reason.as_str())
            .with_context("frontend_action", self.reason.frontend_action())
            .with_context("target_event_type", self.reason.target_event_type())
            .with_message(self.to_string())
    }
}

pub(in crate::daemon::plugins::remote_desktop) trait RemoteAppTargetFocusController:
    Send + Sync
{
    fn focus_exact_target(
        &self,
        binding: &RemoteAppTargetBinding,
        snapshot: &TargetTrackerSnapshot,
    ) -> Result<RemoteAppTargetFocusProof, RemoteAppTargetFocusError>;
}

#[derive(Debug)]
pub(in crate::daemon::plugins::remote_desktop) struct PlatformRemoteAppTargetFocusController {
    snapshot_executor: Arc<TargetSnapshotDeadlineExecutor>,
}

impl PlatformRemoteAppTargetFocusController {
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        snapshot_executor: Arc<TargetSnapshotDeadlineExecutor>,
    ) -> Self {
        Self { snapshot_executor }
    }
}

impl RemoteAppTargetFocusController for PlatformRemoteAppTargetFocusController {
    fn focus_exact_target(
        &self,
        binding: &RemoteAppTargetBinding,
        snapshot: &TargetTrackerSnapshot,
    ) -> Result<RemoteAppTargetFocusProof, RemoteAppTargetFocusError> {
        if binding.target_kind() == RemoteDesktopTargetKind::Display {
            return Err(RemoteAppTargetFocusError::new(
                TargetFocusFailureReason::TargetFocusUnsupported,
                "display-global input has no target-local focus transition",
            ));
        }
        let preflight = bounded_target_snapshot(
            &self.snapshot_executor,
            FOCUS_PROVIDER_DEADLINE,
            "pre_focus_resolution",
        )?;
        let window_id = resolve_exact_focus_window_id(preflight.observation(), binding, snapshot)?;
        let platform_backend = platform::request_focus(binding, snapshot, window_id)?;
        let verification_deadline = Instant::now() + FOCUS_VERIFICATION_DEADLINE;
        loop {
            let remaining = verification_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let sample = bounded_target_snapshot(
                &self.snapshot_executor,
                remaining.min(FOCUS_PROVIDER_DEADLINE),
                "post_focus_verification",
            )?;
            if validate_target_focus_observation(sample.observation(), binding, snapshot).is_ok() {
                return Ok(RemoteAppTargetFocusProof {
                    platform_backend,
                    observed_at_ms: sample.completed_at_ms(),
                });
            }
            let remaining = verification_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            thread::sleep(remaining.min(Duration::from_millis(FOCUS_VERIFICATION_INTERVAL_MS)));
        }
        Err(RemoteAppTargetFocusError::new(
            TargetFocusFailureReason::TargetFocusUnverified,
            "host activation completed but a fresh snapshot did not prove the exact selected target focused",
        ))
    }
}

fn bounded_target_snapshot(
    executor: &TargetSnapshotDeadlineExecutor,
    timeout: Duration,
    stage: &'static str,
) -> Result<TargetSnapshotSample, RemoteAppTargetFocusError> {
    executor.sample_for_input(timeout).map_err(|error| {
        let reason = match error {
            TargetSnapshotDeadlineError::DeadlineExceeded { .. }
            | TargetSnapshotDeadlineError::QueueFull { .. } => {
                TargetFocusFailureReason::TargetFocusUnverified
            }
            TargetSnapshotDeadlineError::SpawnFailed(_)
            | TargetSnapshotDeadlineError::SequenceExhausted(_)
            | TargetSnapshotDeadlineError::ProcessUnavailable { .. }
            | TargetSnapshotDeadlineError::ProtocolFailed { .. }
            | TargetSnapshotDeadlineError::WorkerFailed { .. } => {
                TargetFocusFailureReason::TargetFocusFailed
            }
        };
        RemoteAppTargetFocusError::new(
            reason,
            format!("bounded host target snapshot failed during {stage}: {error}"),
        )
    })
}

fn resolve_exact_focus_window_id(
    observation: &PlatformTargetObservationSample,
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
) -> Result<u64, RemoteAppTargetFocusError> {
    resolve_target_focus_window_id(observation, binding, snapshot).map_err(|reason| {
        let focus_reason = match reason {
            TargetInputGuardFailure::UnsupportedPlatform => {
                TargetFocusFailureReason::TargetFocusUnsupported
            }
            TargetInputGuardFailure::DisplayUnavailable
            | TargetInputGuardFailure::TargetNotFound
            | TargetInputGuardFailure::IdentityMismatch
            | TargetInputGuardFailure::GeometryStale
            | TargetInputGuardFailure::WindowSetStale => TargetFocusFailureReason::TargetFocusStale,
            TargetInputGuardFailure::SnapshotFailed
            | TargetInputGuardFailure::NotVisible
            | TargetInputGuardFailure::FocusNotCommitted
            | TargetInputGuardFailure::NotFocused
            | TargetInputGuardFailure::PointerOutsideTargetSurface
            | TargetInputGuardFailure::PointerOccluded => {
                TargetFocusFailureReason::TargetFocusFailed
            }
        };
        RemoteAppTargetFocusError::new(
            focus_reason,
            format!(
                "fresh host snapshot could not resolve an exact focus target: {}",
                reason.as_str()
            ),
        )
    })
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::ptr;

    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};

    use super::{
        RemoteAppTargetBinding, RemoteAppTargetFocusError, RemoteDesktopTargetKind,
        TargetFocusFailureReason, TargetTrackerSnapshot,
    };

    type AXUIElementRef = *mut c_void;
    type AXValueRef = *const c_void;
    type CFArrayRef = *const c_void;
    type CFIndex = isize;
    type CFStringRef = *const c_void;
    type CFTypeID = usize;
    type CFTypeRef = *const c_void;

    const AX_ERROR_SUCCESS: i32 = 0;
    const AX_VALUE_CGPOINT_TYPE: u32 = 1;
    const AX_VALUE_CGSIZE_TYPE: u32 = 2;
    const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const GEOMETRY_TOLERANCE_POINTS: f64 = 2.0;

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

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXUIElementCreateApplication(pid: libc::pid_t) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: CFTypeRef,
        ) -> i32;
        fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> i32;
        fn AXValueGetType(value: AXValueRef) -> u32;
        fn AXValueGetValue(value: AXValueRef, value_type: u32, value_ptr: *mut c_void) -> u8;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
        fn CFArrayGetTypeID() -> CFTypeID;
        fn CFArrayGetValueAtIndex(array: CFArrayRef, index: CFIndex) -> *const c_void;
        fn CFGetTypeID(value: CFTypeRef) -> CFTypeID;
        fn CFRelease(value: CFTypeRef);
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            value: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetCString(
            value: CFStringRef,
            buffer: *mut c_char,
            buffer_size: CFIndex,
            encoding: u32,
        ) -> u8;
        fn CFStringGetLength(value: CFStringRef) -> CFIndex;
        fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
        fn CFStringGetTypeID() -> CFTypeID;
        static kCFBooleanTrue: CFTypeRef;
    }

    struct CfOwned(CFTypeRef);

    impl CfOwned {
        fn string(value: &str) -> Result<Self, RemoteAppTargetFocusError> {
            let value = CString::new(value).map_err(|_| focus_failed("invalid AX attribute"))?;
            let raw = unsafe {
                CFStringCreateWithCString(ptr::null(), value.as_ptr(), KCF_STRING_ENCODING_UTF8)
            };
            if raw.is_null() {
                return Err(focus_failed(
                    "CoreFoundation could not allocate AX attribute",
                ));
            }
            Ok(Self(raw))
        }

        fn as_ptr(&self) -> CFTypeRef {
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

    pub(super) fn request_focus(
        binding: &RemoteAppTargetBinding,
        snapshot: &TargetTrackerSnapshot,
        _window_id: u64,
    ) -> Result<&'static str, RemoteAppTargetFocusError> {
        if !unsafe { AXIsProcessTrusted() } {
            return Err(RemoteAppTargetFocusError::new(
                TargetFocusFailureReason::TargetFocusPermissionMissing,
                "macOS Accessibility permission is required to focus the selected remote target",
            ));
        }
        let pid = binding
            .native_locator()
            .pid()
            .and_then(|pid| libc::pid_t::try_from(pid).ok())
            .filter(|pid| *pid > 0)
            .ok_or_else(|| {
                RemoteAppTargetFocusError::new(
                    TargetFocusFailureReason::TargetFocusUnsupported,
                    "selected target has no valid native process identifier",
                )
            })?;
        let application = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .ok_or_else(|| {
                RemoteAppTargetFocusError::new(
                    TargetFocusFailureReason::TargetFocusFailed,
                    format!("selected target process {pid} is no longer running"),
                )
            })?;
        let app_element = CfOwned(unsafe { AXUIElementCreateApplication(pid) });
        if app_element.as_ptr().is_null() {
            return Err(focus_failed(format!(
                "Accessibility could not bind selected target process {pid}"
            )));
        }

        // AppKit participates in activation bookkeeping; Accessibility owns
        // the authorized foreground mutation. The latter is also what makes
        // an exact window raise possible without synthesizing an extra click.
        let _ = application.activateWithOptions(NSApplicationActivationOptions::empty());
        set_ax_boolean(app_element.as_ptr().cast_mut(), "AXFrontmost")?;
        match binding.target_kind() {
            RemoteDesktopTargetKind::Window => raise_exact_window(&app_element, snapshot)?,
            RemoteDesktopTargetKind::Application => {}
            RemoteDesktopTargetKind::Display => {
                return Err(RemoteAppTargetFocusError::new(
                    TargetFocusFailureReason::TargetFocusUnsupported,
                    "display-global input has no Accessibility focus target",
                ));
            }
        }
        Ok("macos_accessibility_verified_snapshot")
    }

    fn raise_exact_window(
        app_element: &CfOwned,
        snapshot: &TargetTrackerSnapshot,
    ) -> Result<(), RemoteAppTargetFocusError> {
        let windows = copy_ax_attribute(app_element.as_ptr().cast_mut(), "AXWindows")?;
        if unsafe { CFGetTypeID(windows.as_ptr()) } != unsafe { CFArrayGetTypeID() } {
            return Err(focus_failed("Accessibility AXWindows was not an array"));
        }
        let count = unsafe { CFArrayGetCount(windows.as_ptr()) };
        let mut matched = None;
        let mut match_count = 0usize;
        for index in 0..count {
            let window =
                unsafe { CFArrayGetValueAtIndex(windows.as_ptr(), index) } as AXUIElementRef;
            if window.is_null() || !window_matches_snapshot(window, snapshot) {
                continue;
            }
            matched = Some(window);
            match_count += 1;
        }
        let window = match (match_count, matched) {
            (1, Some(window)) => window,
            (0, _) => {
                return Err(RemoteAppTargetFocusError::new(
                    TargetFocusFailureReason::TargetFocusUnverified,
                    "Accessibility did not find a window matching the committed title and geometry",
                ));
            }
            _ => {
                return Err(RemoteAppTargetFocusError::new(
                    TargetFocusFailureReason::TargetFocusUnverified,
                    "Accessibility window match was ambiguous for the committed title and geometry",
                ));
            }
        };
        let raise = CfOwned::string("AXRaise")?;
        let result = unsafe { AXUIElementPerformAction(window, raise.as_ptr()) };
        if result != AX_ERROR_SUCCESS {
            return Err(focus_failed(format!(
                "Accessibility AXRaise failed with AXError {result}"
            )));
        }
        // These writes are advisory across applications. AXRaise plus the
        // exact fresh CGWindow snapshot below is the authoritative proof.
        let _ = set_ax_boolean(window, "AXMain");
        let _ = set_ax_boolean(window, "AXFocused");
        Ok(())
    }

    fn window_matches_snapshot(window: AXUIElementRef, snapshot: &TargetTrackerSnapshot) -> bool {
        let Some((position, size)) = ax_window_geometry(window) else {
            return false;
        };
        let expected = snapshot.geometry();
        let geometry_matches = [
            (expected.x, position.x),
            (expected.y, position.y),
            (expected.width, size.width),
            (expected.height, size.height),
        ]
        .into_iter()
        .all(|(expected, actual)| {
            expected.is_some_and(|expected| {
                expected.is_finite()
                    && actual.is_finite()
                    && (expected - actual).abs() <= GEOMETRY_TOLERANCE_POINTS
            })
        });
        if !geometry_matches {
            return false;
        }
        snapshot.title().is_none_or(|expected| {
            copy_ax_attribute(window, "AXTitle")
                .ok()
                .and_then(|title| cf_string(&title))
                .is_some_and(|actual| actual == expected)
        })
    }

    fn ax_window_geometry(window: AXUIElementRef) -> Option<(CGPoint, CGSize)> {
        let position = copy_ax_attribute(window, "AXPosition").ok()?;
        let size = copy_ax_attribute(window, "AXSize").ok()?;
        if unsafe { AXValueGetType(position.as_ptr()) } != AX_VALUE_CGPOINT_TYPE
            || unsafe { AXValueGetType(size.as_ptr()) } != AX_VALUE_CGSIZE_TYPE
        {
            return None;
        }
        let mut point = CGPoint::default();
        let mut dimensions = CGSize::default();
        let point_ok = unsafe {
            AXValueGetValue(
                position.as_ptr(),
                AX_VALUE_CGPOINT_TYPE,
                (&mut point as *mut CGPoint).cast(),
            )
        } != 0;
        let size_ok = unsafe {
            AXValueGetValue(
                size.as_ptr(),
                AX_VALUE_CGSIZE_TYPE,
                (&mut dimensions as *mut CGSize).cast(),
            )
        } != 0;
        (point_ok && size_ok).then_some((point, dimensions))
    }

    fn set_ax_boolean(
        element: AXUIElementRef,
        attribute_name: &str,
    ) -> Result<(), RemoteAppTargetFocusError> {
        let attribute = CfOwned::string(attribute_name)?;
        let result =
            unsafe { AXUIElementSetAttributeValue(element, attribute.as_ptr(), kCFBooleanTrue) };
        if result != AX_ERROR_SUCCESS {
            return Err(focus_failed(format!(
                "Accessibility could not set {attribute_name} (AXError {result})"
            )));
        }
        Ok(())
    }

    fn copy_ax_attribute(
        element: AXUIElementRef,
        attribute_name: &str,
    ) -> Result<CfOwned, RemoteAppTargetFocusError> {
        let attribute = CfOwned::string(attribute_name)?;
        let mut value: CFTypeRef = ptr::null();
        let result =
            unsafe { AXUIElementCopyAttributeValue(element, attribute.as_ptr(), &mut value) };
        if result != AX_ERROR_SUCCESS || value.is_null() {
            return Err(focus_failed(format!(
                "Accessibility could not read {attribute_name} (AXError {result})"
            )));
        }
        Ok(CfOwned(value))
    }

    fn cf_string(value: &CfOwned) -> Option<String> {
        if unsafe { CFGetTypeID(value.as_ptr()) } != unsafe { CFStringGetTypeID() } {
            return None;
        }
        let length = unsafe { CFStringGetLength(value.as_ptr()) };
        let capacity =
            unsafe { CFStringGetMaximumSizeForEncoding(length, KCF_STRING_ENCODING_UTF8) }
                .checked_add(1)?;
        let capacity = usize::try_from(capacity).ok()?;
        let mut buffer = vec![0_u8; capacity];
        if unsafe {
            CFStringGetCString(
                value.as_ptr(),
                buffer.as_mut_ptr().cast(),
                CFIndex::try_from(capacity).ok()?,
                KCF_STRING_ENCODING_UTF8,
            )
        } == 0
        {
            return None;
        }
        unsafe { CStr::from_ptr(buffer.as_ptr().cast()) }
            .to_str()
            .ok()
            .map(str::to_string)
    }

    fn focus_failed(detail: impl Into<String>) -> RemoteAppTargetFocusError {
        RemoteAppTargetFocusError::new(TargetFocusFailureReason::TargetFocusFailed, detail)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    use super::{
        bounded_target_snapshot, TargetFocusFailureReason, TargetSnapshotDeadlineExecutor,
    };
    use crate::daemon::plugins::remote_desktop::target_observer::PlatformTargetObservationSample;
    use crate::daemon::plugins::remote_desktop::target_snapshot::TargetObservationSampler;

    struct BlockingTargetSampler {
        calls: AtomicUsize,
        released: Mutex<bool>,
        release_signal: Condvar,
    }

    impl BlockingTargetSampler {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                released: Mutex::new(false),
                release_signal: Condvar::new(),
            }
        }

        fn release(&self) {
            let mut released = self
                .released
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *released = true;
            self.release_signal.notify_all();
        }
    }

    impl TargetObservationSampler for BlockingTargetSampler {
        fn sample(&self) -> PlatformTargetObservationSample {
            self.calls.fetch_add(1, Ordering::AcqRel);
            let mut released = self
                .released
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while !*released {
                released = self
                    .release_signal
                    .wait(released)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            PlatformTargetObservationSample::unavailable_for_test()
        }
    }

    #[test]
    fn focus_snapshot_wait_is_deadline_bounded() {
        let sampler = Arc::new(BlockingTargetSampler::new());
        let sampler_port: Arc<dyn TargetObservationSampler> = sampler.clone();
        let executor = TargetSnapshotDeadlineExecutor::new(sampler_port);
        let started = Instant::now();

        let error = match bounded_target_snapshot(
            &executor,
            Duration::from_millis(20),
            "focus_deadline_test",
        ) {
            Ok(_) => panic!("hung focus snapshot must fail by deadline"),
            Err(error) => error,
        };

        assert_eq!(
            error.reason(),
            TargetFocusFailureReason::TargetFocusUnverified
        );
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "focus snapshot timeout must not wait for the native provider"
        );
        assert_eq!(sampler.calls.load(Ordering::Acquire), 1);
        sampler.release();
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        SetForegroundWindow,
    };

    use super::{
        RemoteAppTargetBinding, RemoteAppTargetFocusError, TargetFocusFailureReason,
        TargetTrackerSnapshot,
    };

    pub(super) fn request_focus(
        binding: &RemoteAppTargetBinding,
        _snapshot: &TargetTrackerSnapshot,
        window_id: u64,
    ) -> Result<&'static str, RemoteAppTargetFocusError> {
        let hwnd = window_id as usize as HWND;
        if hwnd.is_null() || unsafe { IsWindow(hwnd) } == 0 {
            return Err(stale(format!(
                "selected native window {window_id} is no longer valid"
            )));
        }
        let mut observed_pid = 0_u32;
        if unsafe { GetWindowThreadProcessId(hwnd, &mut observed_pid) } == 0 {
            return Err(stale(format!(
                "selected native window {window_id} has no owning process"
            )));
        }
        if binding
            .native_locator()
            .pid()
            .and_then(|pid| u32::try_from(pid).ok())
            .is_some_and(|expected_pid| expected_pid != observed_pid)
        {
            return Err(stale(format!(
                "selected native window {window_id} changed owner process"
            )));
        }
        if unsafe { IsWindowVisible(hwnd) } == 0 || unsafe { IsIconic(hwnd) } != 0 {
            return Err(RemoteAppTargetFocusError::new(
                TargetFocusFailureReason::TargetFocusFailed,
                "selected native window is hidden or minimized",
            ));
        }
        if unsafe { GetForegroundWindow() } == hwnd {
            return Ok("windows_user32_verified_snapshot");
        }
        if unsafe { SetForegroundWindow(hwnd) } == 0 {
            return Err(RemoteAppTargetFocusError::new(
                TargetFocusFailureReason::TargetFocusFailed,
                "Windows denied foreground activation for the exact selected window",
            ));
        }
        Ok("windows_user32_verified_snapshot")
    }

    fn stale(detail: impl Into<String>) -> RemoteAppTargetFocusError {
        RemoteAppTargetFocusError::new(TargetFocusFailureReason::TargetFocusStale, detail)
    }
}

#[cfg(target_os = "linux")]
#[path = "target_focus_linux.rs"]
mod platform;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod platform {
    use super::{
        RemoteAppTargetBinding, RemoteAppTargetFocusError, TargetFocusFailureReason,
        TargetTrackerSnapshot,
    };

    pub(super) fn request_focus(
        _binding: &RemoteAppTargetBinding,
        _snapshot: &TargetTrackerSnapshot,
        _window_id: u64,
    ) -> Result<&'static str, RemoteAppTargetFocusError> {
        Err(RemoteAppTargetFocusError::new(
            TargetFocusFailureReason::TargetFocusUnsupported,
            "this build has no exact RemoteApp target focus adapter",
        ))
    }
}

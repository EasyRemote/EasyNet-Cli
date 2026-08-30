// EasyNet CLI — canonical media-host one-shot capture client
// ==========================================================
//
// Converts daemon-owned committed target bindings into the private media-host
// contract. The helper performs native proof/capture; this module only owns
// bounded process supervision and projection back into daemon domain types.

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(all(target_os = "macos", not(test)))]
use base64::Engine;
#[cfg(target_os = "macos")]
use easynet_remoteapp_native_protocol::capture_probe::{Operation, Outcome, Request, Response};
#[cfg(target_os = "macos")]
use easynet_remoteapp_native_protocol::media_session::FailureReason;
use easynet_remoteapp_native_protocol::media_session::{
    ApplicationSurface, ApplicationWindowSet, CaptureBackend, CaptureProof, NativeTargetPlan,
    TargetKind,
};

#[cfg(all(target_os = "macos", not(test)))]
use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    EncodedFrame, ScreenCaptureOptions,
};
#[cfg(target_os = "macos")]
use crate::daemon::plugins::remote_desktop::native_host_process::execute_one_shot_native_host;
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, RemoteDesktopTargetKind, ResolvedCaptureTargetProof,
};
#[cfg(target_os = "macos")]
use crate::daemon::plugins::remote_desktop::target::{RemoteAppTargetError, TargetResolutionError};
#[cfg(target_os = "macos")]
use crate::daemon::plugins::remote_desktop::MEDIA_HOST_EXECUTABLE;

// The probe entry points below serve the macOS verify/diagnostic paths;
// Linux and Windows native builds compile this module only for the
// cross-platform plan/proof projection helpers.
#[cfg(target_os = "macos")]
const VERIFY_DEADLINE: Duration = Duration::from_secs(15);
#[cfg(all(target_os = "macos", not(test)))]
const DIAGNOSTIC_DEADLINE: Duration = Duration::from_secs(20);
#[cfg(target_os = "macos")]
static NEXT_CAPTURE_PROBE_GENERATION: AtomicU64 = AtomicU64::new(1);

#[cfg(target_os = "macos")]
pub(super) fn verify_binding(
    ability: &'static str,
    binding: &RemoteAppTargetBinding,
) -> Result<ResolvedCaptureTargetProof, RemoteAppTargetError> {
    let plan = target_plan(binding).map_err(|error| {
        RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetMetadataIncomplete,
            error.to_string(),
        )
    })?;
    let response = execute_probe(plan.clone(), Operation::VerifyTarget, VERIFY_DEADLINE)
        .map_err(|error| probe_transport_error(ability, error))?;
    match response.outcome {
        Outcome::Verified { capture_proof } => {
            if capture_proof.backend != CaptureBackend::ScreenCaptureKit {
                return Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::CaptureBackendUnavailable,
                    "macOS media-host returned a non-ScreenCaptureKit proof",
                ));
            }
            project_capture_proof(binding, &plan, capture_proof).map_err(|error| {
                RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetIdentityChanged,
                    error.to_string(),
                )
            })
        }
        Outcome::Failed { reason, detail } => Err(probe_failure(ability, reason, detail)),
        Outcome::DiagnosticJpeg { .. } => Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::CaptureBackendUnavailable,
            "media-host returned diagnostic output for a verification request",
        )),
    }
}

#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn capture_diagnostic_jpeg(
    ability: &'static str,
    binding: &RemoteAppTargetBinding,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<EncodedFrame> {
    anyhow::ensure!(
        options.region.is_none(),
        "{ability}: native target diagnostic preview does not accept a display-coordinate crop"
    );
    let plan = target_plan(binding)?;
    let native = binding
        .require_capture_proof(ability)?
        .native_dimensions()
        .ok_or_else(|| {
            anyhow::anyhow!("{ability}: binding capture proof has no native dimensions")
        })?;
    let (width, height) = options.output_dimensions(native.0 as u32, native.1 as u32);
    let request_operation = Operation::DiagnosticJpeg { width, height };
    let response = execute_probe(plan, request_operation, DIAGNOSTIC_DEADLINE)
        .map_err(|error| anyhow::Error::new(probe_transport_error(ability, error)))?;
    match response.outcome {
        Outcome::DiagnosticJpeg {
            width,
            height,
            jpeg_base64,
            ..
        } => {
            let jpeg_bytes = base64::engine::general_purpose::STANDARD
                .decode(jpeg_base64)
                .map_err(|error| anyhow::anyhow!("decode media-host diagnostic JPEG: {error}"))?;
            anyhow::ensure!(
                !jpeg_bytes.is_empty()
                    && jpeg_bytes.len()
                        <= easynet_remoteapp_native_protocol::capture_probe::MAX_DIAGNOSTIC_JPEG_BYTES,
                "media-host diagnostic JPEG violates the bounded payload contract"
            );
            Ok(EncodedFrame {
                jpeg_bytes,
                width,
                height,
            })
        }
        Outcome::Failed { reason, detail } => Err(probe_failure(ability, reason, detail).into()),
        Outcome::Verified { .. } => anyhow::bail!(
            "{ability}: media-host returned verification output for a diagnostic request"
        ),
    }
}

#[cfg(target_os = "macos")]
fn execute_probe(
    target: NativeTargetPlan,
    operation: Operation,
    deadline: Duration,
) -> Result<Response, String> {
    let generation = NEXT_CAPTURE_PROBE_GENERATION.fetch_add(1, Ordering::Relaxed);
    if generation == 0 || generation == u64::MAX {
        return Err("capture-probe generation exhausted".into());
    }
    let request = Request::new(generation, generation, target, operation);
    let response: Response = execute_one_shot_native_host(
        generation,
        MEDIA_HOST_EXECUTABLE,
        "media-capture-probe",
        &[],
        &request,
        deadline,
    )?;
    response
        .validate_for(&request)
        .map_err(|error| format!("validate media-host capture-probe response: {error}"))?;
    Ok(response)
}

pub(super) fn project_capture_proof(
    binding: &RemoteAppTargetBinding,
    plan: &NativeTargetPlan,
    proof: CaptureProof,
) -> anyhow::Result<ResolvedCaptureTargetProof> {
    proof.validate_for(plan)?;
    let backend = match proof.backend {
        CaptureBackend::ScreenCaptureKit => "screencapturekit",
        CaptureBackend::XcapX11 => "xcap",
        CaptureBackend::WindowsGraphicsCapture => "windows_graphics_capture",
        CaptureBackend::Dxgi => "dxgi",
        CaptureBackend::PortalPipeWire => "portal_pipewire",
    };
    let mut projected = ResolvedCaptureTargetProof::new(backend, binding.target_kind())
        .with_native_identity(
            plan.display_id,
            plan.window_id,
            plan.pid,
            plan.app_identity.clone(),
            plan.bundle_id.clone(),
        )
        .with_process_instance_id(plan.process_instance_id.clone())
        .with_native_dimensions(Some((
            proof.native_width as usize,
            proof.native_height as usize,
        )));
    if let Some(window_set) = binding.committed_app_window_set() {
        projected = projected.with_app_window_set(window_set.clone());
    }
    if let Some(layout) = binding.committed_app_surface_layout() {
        projected = projected.with_app_surface_layout(layout.clone());
    }
    Ok(projected)
}

#[cfg(target_os = "macos")]
fn probe_transport_error(ability: &'static str, detail: String) -> RemoteAppTargetError {
    RemoteAppTargetError::new(
        ability,
        TargetResolutionError::CaptureBackendUnavailable,
        format!("canonical media-host capture probe failed: {detail}"),
    )
}

#[cfg(target_os = "macos")]
fn probe_failure(
    ability: &'static str,
    reason: FailureReason,
    detail: String,
) -> RemoteAppTargetError {
    let target_reason = match reason {
        FailureReason::PermissionDenied | FailureReason::PermissionRevoked => {
            TargetResolutionError::TargetPermissionMissing
        }
        FailureReason::TargetInvalidated => TargetResolutionError::TargetStale,
        FailureReason::DeviceLost => TargetResolutionError::TargetDisplayUnavailable,
        FailureReason::CaptureUnavailable
        | FailureReason::EncoderUnavailable
        | FailureReason::AudioUnavailable
        | FailureReason::ProtocolViolation
        | FailureReason::Internal => TargetResolutionError::CaptureBackendUnavailable,
    };
    RemoteAppTargetError::new(ability, target_reason, detail)
}

pub(super) fn target_plan(binding: &RemoteAppTargetBinding) -> anyhow::Result<NativeTargetPlan> {
    let locator = binding.native_locator();
    let kind = match binding.target_kind() {
        RemoteDesktopTargetKind::Display => TargetKind::Display,
        RemoteDesktopTargetKind::Window => TargetKind::Window,
        RemoteDesktopTargetKind::Application => TargetKind::Application,
    };
    let application = if kind == TargetKind::Application {
        let windows = binding.committed_app_window_set().ok_or_else(|| {
            anyhow::anyhow!("application media binding has no committed window-set proof")
        })?;
        let layout = binding.committed_app_surface_layout().ok_or_else(|| {
            anyhow::anyhow!("application media binding has no committed surface-layout proof")
        })?;
        let surfaces = layout
            .front_to_back_surfaces()
            .map(|(window_id, x, y, width, height)| {
                Ok(ApplicationSurface {
                    window_id,
                    x,
                    y,
                    width: u32::try_from(width)?,
                    height: u32::try_from(height)?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Some(ApplicationWindowSet {
            display_id: windows.display_id(),
            display_ids: windows.display_ids().to_vec(),
            primary_pid: windows.primary_pid().ok_or_else(|| {
                anyhow::anyhow!("application media binding has no primary process")
            })?,
            process_instance_id: windows.process_instance_id().map(str::to_string),
            app_identity: locator.app_identity().map(str::to_string),
            bundle_id: windows.bundle_id().map(str::to_string),
            window_ids: windows.resolved_window_ids().to_vec(),
            window_set_epoch: windows.window_set_epoch(),
            front_to_back_surfaces: surfaces,
            surface_layout_epoch: layout.layout_epoch(),
        })
    } else {
        None
    };
    let plan = NativeTargetPlan {
        kind,
        display_id: locator.display_id(),
        window_id: locator.window_id(),
        pid: locator.pid(),
        process_instance_id: locator.process_instance_id().map(str::to_string),
        app_identity: locator.app_identity().map(str::to_string),
        bundle_id: locator.bundle_id().map(str::to_string),
        application,
    };
    plan.validate().map_err(|error| {
        anyhow::anyhow!(
            "{} target binding cannot form a media-host contract: {error}",
            binding.target_kind().as_str()
        )
    })?;
    Ok(plan)
}

//! Cross-platform xcap/OpenH264 capture adapter for the private RemoteApp media host.
//!
//! This adapter re-resolves the exact committed native target on every frame,
//! verifies platform process-instance identity, captures only the selected pixels,
//! and emits bounded OpenH264 Annex-B access units. Linux Wayland/Portal
//! capture remains a separate adapter and is never silently widened to X11.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use easynet_remoteapp_native_platform::{
    CaptureEligibleSurface, PlatformWindowProcessIdentityProvider,
};
use easynet_remoteapp_native_protocol::media_session::{
    ApplicationSurface, CaptureBackend, CaptureProof, EventBody, FailureReason, MediaStats,
    NativeTargetPlan, StartContract, TargetKind, VideoConfig,
};
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, IntraFramePeriod,
    Level as OpenH264Level, Profile, RateControlMode, UsageType,
};
use openh264::formats::{RgbSliceU8, YUVBuffer};
use openh264::{OpenH264API, Timestamp};

use super::{now_ms, BackendEvent, BackendFailure, SessionBackend};

pub(super) struct XcapOpenH264SessionBackend {
    process_identity_provider: Option<PlatformWindowProcessIdentityProvider>,
    contract: Option<StartContract>,
    encoder: Option<Encoder>,
    active: bool,
    media_gate: u32,
    codec_generation: u32,
    frame_index: u64,
    next_frame_at: Option<Instant>,
    discontinuity: bool,
    stats_started_at: Instant,
    last_stats_at: Instant,
    capture_frames: u64,
    encoded_frames: u64,
    video_bytes: u64,
}

impl Default for XcapOpenH264SessionBackend {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            process_identity_provider: None,
            contract: None,
            encoder: None,
            active: false,
            media_gate: 0,
            codec_generation: 1,
            frame_index: 0,
            next_frame_at: None,
            discontinuity: true,
            stats_started_at: now,
            last_stats_at: now,
            capture_frames: 0,
            encoded_frames: 0,
            video_bytes: 0,
        }
    }
}

impl SessionBackend for XcapOpenH264SessionBackend {
    fn prepare(&mut self, contract: &StartContract) -> Result<CaptureProof, BackendFailure> {
        ensure_platform_session()?;
        if contract.audio.is_some() {
            return Err(BackendFailure::new(
                FailureReason::AudioUnavailable,
                format!(
                    "{} xcap media-host video adapter cannot satisfy negotiated host audio yet",
                    std::env::consts::OS
                ),
            ));
        }
        let process_identity_provider =
            PlatformWindowProcessIdentityProvider::connect().map_err(|error| {
                target_invalidated(format!("initialize process identity provider: {error}"))
            })?;
        let captured = capture_exact_target(&process_identity_provider, &contract.target)?;
        let encoder = build_encoder(&contract.video)?;
        self.process_identity_provider = Some(process_identity_provider);
        self.contract = Some(contract.clone());
        self.encoder = Some(encoder);
        self.stats_started_at = Instant::now();
        self.last_stats_at = self.stats_started_at;
        Ok(CaptureProof {
            backend: platform_capture_backend(),
            observed_target: contract.target.clone(),
            native_width: captured.width,
            native_height: captured.height,
            verified_at_ms: now_ms(),
        })
    }

    fn activate(&mut self) -> Result<(), BackendFailure> {
        if self.contract.is_none() || self.encoder.is_none() {
            return Err(internal("activate before successful xcap preparation"));
        }
        Ok(())
    }

    fn begin_media(&mut self, media_gate: u32) -> Result<(), BackendFailure> {
        if media_gate == 0 {
            return Err(internal("xcap media began without a generation gate"));
        }
        self.media_gate = media_gate;
        self.frame_index = 0;
        self.discontinuity = true;
        self.next_frame_at = Some(Instant::now());
        self.active = true;
        self.encoder
            .as_mut()
            .ok_or_else(|| internal("xcap encoder missing at activation"))?
            .force_intra_frame();
        Ok(())
    }

    fn reconfigure(
        &mut self,
        video: &VideoConfig,
        force_keyframe: bool,
    ) -> Result<(), BackendFailure> {
        self.active = false;
        self.encoder = Some(build_encoder(video)?);
        self.contract
            .as_mut()
            .ok_or_else(|| internal("xcap reconfigure before preparation"))?
            .video = video.clone();
        self.codec_generation = self
            .codec_generation
            .checked_add(1)
            .ok_or_else(|| internal("xcap codec generation overflow"))?;
        if force_keyframe {
            self.encoder
                .as_mut()
                .expect("encoder was just installed")
                .force_intra_frame();
        }
        self.discontinuity = true;
        Ok(())
    }

    fn resume_media(&mut self, media_gate: u32) -> Result<(), BackendFailure> {
        if media_gate == 0 {
            return Err(internal("xcap media resumed without a generation gate"));
        }
        self.media_gate = media_gate;
        self.frame_index = 0;
        self.next_frame_at = Some(Instant::now());
        self.discontinuity = true;
        self.active = true;
        self.encoder
            .as_mut()
            .ok_or_else(|| internal("xcap encoder missing at resume"))?
            .force_intra_frame();
        Ok(())
    }

    fn request_keyframe(&mut self) -> Result<(), BackendFailure> {
        self.encoder
            .as_mut()
            .ok_or_else(|| internal("xcap keyframe requested before preparation"))?
            .force_intra_frame();
        Ok(())
    }

    fn poll(&mut self, timeout: Duration) -> Result<Option<BackendEvent>, BackendFailure> {
        if !self.active {
            std::thread::park_timeout(timeout);
            return Ok(None);
        }
        let now = Instant::now();
        let due = self.next_frame_at.unwrap_or(now);
        if now < due {
            std::thread::park_timeout(timeout.min(due.saturating_duration_since(now)));
            return Ok(None);
        }
        if self.last_stats_at.elapsed() >= Duration::from_secs(1) {
            self.last_stats_at = Instant::now();
            return Ok(Some(BackendEvent::Stats(MediaStats {
                capture_frames: self.capture_frames,
                encoded_video_frames: self.encoded_frames,
                encoded_audio_packets: 0,
                raw_video_frames_dropped: 0,
                encoded_video_frames_dropped: 0,
                audio_packets_dropped: 0,
                video_queue_depth: 0,
                audio_queue_depth: 0,
                video_bytes: self.video_bytes,
                audio_bytes: 0,
            })));
        }
        let contract = self
            .contract
            .as_ref()
            .ok_or_else(|| internal("xcap poll has no media contract"))?;
        let submitted_at_ms = now_ms();
        let process_identity_provider = self
            .process_identity_provider
            .as_ref()
            .ok_or_else(|| internal("xcap poll has no process identity provider"))?;
        let captured = capture_exact_target(process_identity_provider, &contract.target)?;
        self.capture_frames = self.capture_frames.saturating_add(1);
        let rgb = resize_rgba_to_rgb(
            &captured.rgba,
            captured.width,
            captured.height,
            contract.video.width,
            contract.video.height,
        )?;
        let timestamp_ms =
            self.frame_index.saturating_mul(1_000) / u64::from(contract.video.fps.max(1));
        let encoder = self
            .encoder
            .as_mut()
            .ok_or_else(|| internal("xcap encoder disappeared"))?;
        let yuv = YUVBuffer::from_rgb8_source(RgbSliceU8::new(
            &rgb,
            (
                contract.video.width as usize,
                contract.video.height as usize,
            ),
        ));
        let payload = encoder
            .encode_at(&yuv, Timestamp::from_millis(timestamp_ms))
            .map_err(|error| {
                BackendFailure::new(
                    FailureReason::EncoderUnavailable,
                    format!("OpenH264 encode failed: {error}"),
                )
            })?
            .to_vec();
        let interval = Duration::from_secs_f64(1.0 / f64::from(contract.video.fps.max(1)));
        self.next_frame_at = Some(Instant::now() + interval);
        if payload.is_empty() {
            return Ok(None);
        }
        let (keyframe, sps_pps_present) = inspect_annex_b(&payload);
        let discontinuity = self.discontinuity;
        self.discontinuity = false;
        self.frame_index = self.frame_index.saturating_add(1);
        self.encoded_frames = self.encoded_frames.saturating_add(1);
        self.video_bytes = self.video_bytes.saturating_add(payload.len() as u64);
        let encoded_at_ms = now_ms().max(submitted_at_ms);
        Ok(Some(BackendEvent::Video {
            body: EventBody::VideoH264 {
                media_gate: self.media_gate,
                pts_90khz: timestamp_ms.saturating_mul(90),
                duration_90khz: (90_000 / contract.video.fps.max(1)).max(1),
                keyframe,
                sps_pps_present,
                discontinuity,
                codec_generation: self.codec_generation,
                width: contract.video.width,
                height: contract.video.height,
                encode_submitted_at_ms: submitted_at_ms,
                encoded_at_ms,
            },
            payload,
        }))
    }

    fn stop(&mut self) -> Result<(), BackendFailure> {
        self.active = false;
        self.next_frame_at = None;
        self.encoder.take();
        self.contract.take();
        self.process_identity_provider.take();
        Ok(())
    }
}

struct CapturedRgba {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

#[cfg(target_os = "linux")]
fn ensure_platform_session() -> Result<(), BackendFailure> {
    if std::env::var_os("DISPLAY").is_none() {
        return Err(BackendFailure::new(
            FailureReason::CaptureUnavailable,
            "Linux xcap capture requires DISPLAY",
        ));
    }
    if std::env::var("XDG_SESSION_TYPE")
        .ok()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("wayland"))
    {
        return Err(BackendFailure::new(
            FailureReason::CaptureUnavailable,
            "Wayland targets require the dedicated Portal/PipeWire media-host adapter",
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn ensure_platform_session() -> Result<(), BackendFailure> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn platform_capture_backend() -> CaptureBackend {
    CaptureBackend::XcapX11
}

#[cfg(target_os = "windows")]
fn platform_capture_backend() -> CaptureBackend {
    CaptureBackend::WindowsGraphicsCapture
}

fn capture_exact_target(
    process_identity_provider: &PlatformWindowProcessIdentityProvider,
    target: &NativeTargetPlan,
) -> Result<CapturedRgba, BackendFailure> {
    match target.kind {
        TargetKind::Display => capture_display(target),
        TargetKind::Window => capture_window(process_identity_provider, target),
        TargetKind::Application => capture_application(process_identity_provider, target),
    }
}

fn capture_display(target: &NativeTargetPlan) -> Result<CapturedRgba, BackendFailure> {
    let expected = target
        .display_id
        .ok_or_else(|| internal("display target lost its exact id"))?;
    let monitor = xcap::Monitor::all()
        .map_err(capture_unavailable)?
        .into_iter()
        .find(|monitor| monitor.id().ok().map(u64::from) == Some(expected))
        .ok_or_else(|| {
            target_invalidated(format!("xcap display {expected} is no longer present"))
        })?;
    let image = monitor.capture_image().map_err(capture_unavailable)?;
    rgba_image(image)
}

fn capture_window(
    process_identity_provider: &PlatformWindowProcessIdentityProvider,
    target: &NativeTargetPlan,
) -> Result<CapturedRgba, BackendFailure> {
    let expected_id = target
        .window_id
        .ok_or_else(|| internal("window target lost its exact id"))?;
    let expected_pid = target
        .pid
        .and_then(|pid| u32::try_from(pid).ok())
        .ok_or_else(|| internal("window target lost its owner pid"))?;
    verify_process_instance(
        process_identity_provider,
        expected_pid,
        target.process_instance_id.as_deref(),
    )?;
    let window = exact_window(
        process_identity_provider,
        expected_id,
        expected_pid,
        target.process_instance_id.as_deref(),
    )?;
    verify_window_identity(process_identity_provider, &window, target, None)?;
    let image = window.capture_image().map_err(capture_unavailable)?;
    verify_process_instance(
        process_identity_provider,
        expected_pid,
        target.process_instance_id.as_deref(),
    )?;
    rgba_image(image)
}

fn capture_application(
    process_identity_provider: &PlatformWindowProcessIdentityProvider,
    target: &NativeTargetPlan,
) -> Result<CapturedRgba, BackendFailure> {
    let application = target
        .application
        .as_ref()
        .ok_or_else(|| internal("application target lost its window-set proof"))?;
    let pid = u32::try_from(application.primary_pid)
        .map_err(|_| internal("application target has an invalid pid"))?;
    let expected_process_instance_id = application.process_instance_id.as_deref();
    verify_process_instance(process_identity_provider, pid, expected_process_instance_id)?;
    let windows = xcap::Window::all().map_err(capture_unavailable)?;
    let mut actual_front_to_back = Vec::new();
    for window in &windows {
        let Ok(window_id) = window.id().map(u64::from) else {
            continue;
        };
        if !xcap_surface_eligible(window)? {
            continue;
        }
        let Some(instance) = process_identity_provider
            .resolve_window(window_id)
            .map_err(|error| {
                target_invalidated(format!("resolve window {window_id} owner: {error}"))
            })?
        else {
            continue;
        };
        if instance.pid() != pid || expected_process_instance_id != Some(instance.stable_id()) {
            continue;
        }
        actual_front_to_back.push(window_id);
    }
    let mut actual_membership = actual_front_to_back.clone();
    actual_membership.sort_unstable();
    if actual_membership != application.window_ids {
        return Err(target_invalidated(format!(
            "xcap application window membership changed: expected {:?}, observed {:?}",
            application.window_ids, actual_membership
        )));
    }
    let expected_front_to_back = application
        .front_to_back_surfaces
        .iter()
        .map(|surface| surface.window_id)
        .collect::<Vec<_>>();
    if actual_front_to_back != expected_front_to_back {
        return Err(target_invalidated(format!(
            "xcap application stacking order changed: expected {:?}, observed {:?}",
            expected_front_to_back, actual_front_to_back
        )));
    }
    let mut by_id = BTreeMap::new();
    for window in windows {
        if let Ok(id) = window.id() {
            by_id.insert(u64::from(id), window);
        }
    }
    let mut captured = BTreeMap::new();
    for surface in &application.front_to_back_surfaces {
        let window = by_id.get(&surface.window_id).ok_or_else(|| {
            target_invalidated(format!(
                "xcap application window {} is no longer present",
                surface.window_id
            ))
        })?;
        verify_window_identity(process_identity_provider, window, target, Some(surface))?;
        let image = window.capture_image().map_err(capture_unavailable)?;
        if image.width() != surface.width || image.height() != surface.height {
            return Err(target_invalidated(format!(
                "xcap application window {} dimensions changed from {}x{} to {}x{}",
                surface.window_id,
                surface.width,
                surface.height,
                image.width(),
                image.height()
            )));
        }
        captured.insert(surface.window_id, image);
    }
    verify_process_instance(process_identity_provider, pid, expected_process_instance_id)?;
    compose_application(&application.front_to_back_surfaces, &captured)
}

fn exact_window(
    process_identity_provider: &PlatformWindowProcessIdentityProvider,
    id: u64,
    pid: u32,
    expected_process_instance_id: Option<&str>,
) -> Result<xcap::Window, BackendFailure> {
    let window = xcap::Window::all()
        .map_err(capture_unavailable)?
        .into_iter()
        .find(|window| window.id().ok().map(u64::from) == Some(id))
        .ok_or_else(|| target_invalidated(format!("xcap window {id} is gone")))?;
    verify_window_process_instance(
        process_identity_provider,
        id,
        pid,
        expected_process_instance_id,
    )?;
    if !xcap_surface_eligible(&window)? {
        return Err(target_invalidated(format!(
            "xcap window {id} is no longer capture eligible"
        )));
    }
    Ok(window)
}

fn verify_window_identity(
    process_identity_provider: &PlatformWindowProcessIdentityProvider,
    window: &xcap::Window,
    target: &NativeTargetPlan,
    surface: Option<&ApplicationSurface>,
) -> Result<(), BackendFailure> {
    let expected_pid = target
        .pid
        .and_then(|pid| u32::try_from(pid).ok())
        .ok_or_else(|| internal("target owner pid is invalid"))?;
    let window_id = window.id().map(u64::from).map_err(capture_unavailable)?;
    verify_window_process_instance(
        process_identity_provider,
        window_id,
        expected_pid,
        target.process_instance_id.as_deref(),
    )?;
    if !xcap_surface_eligible(window)? {
        return Err(target_invalidated(format!(
            "xcap window {window_id} is no longer capture eligible"
        )));
    }
    if let Some(surface) = surface {
        let actual = (
            i64::from(window.x().map_err(capture_unavailable)?),
            i64::from(window.y().map_err(capture_unavailable)?),
            window.width().map_err(capture_unavailable)?,
            window.height().map_err(capture_unavailable)?,
        );
        let expected = (surface.x, surface.y, surface.width, surface.height);
        if actual != expected {
            return Err(target_invalidated(format!(
                "xcap application surface {} geometry changed",
                surface.window_id
            )));
        }
    }
    Ok(())
}

fn verify_window_process_instance(
    process_identity_provider: &PlatformWindowProcessIdentityProvider,
    window_id: u64,
    expected_pid: u32,
    expected_process_instance_id: Option<&str>,
) -> Result<(), BackendFailure> {
    let expected_process_instance_id = expected_process_instance_id.ok_or_else(|| {
        target_invalidated("window/application target lacks process-instance identity")
    })?;
    let observed = process_identity_provider
        .resolve_window(window_id)
        .map_err(|error| target_invalidated(format!("resolve window {window_id} owner: {error}")))?
        .ok_or_else(|| target_invalidated(format!("window {window_id} has no native owner")))?;
    if observed.pid() != expected_pid || observed.stable_id() != expected_process_instance_id {
        return Err(target_invalidated(format!(
            "window {window_id} process instance changed: expected pid={expected_pid} instance={expected_process_instance_id:?}, observed pid={} instance={:?}",
            observed.pid(),
            observed.stable_id()
        )));
    }
    Ok(())
}

fn xcap_surface_eligible(window: &xcap::Window) -> Result<bool, BackendFailure> {
    let width = window.width().map_err(capture_unavailable)?;
    let height = window.height().map_err(capture_unavailable)?;
    let minimized = window.is_minimized().map_err(capture_unavailable)?;
    Ok(CaptureEligibleSurface::xcap(width, height, minimized).is_eligible())
}

fn compose_application(
    front_to_back: &[ApplicationSurface],
    captured: &BTreeMap<u64, xcap::image::RgbaImage>,
) -> Result<CapturedRgba, BackendFailure> {
    let min_x = front_to_back
        .iter()
        .map(|surface| surface.x)
        .min()
        .ok_or_else(|| internal("empty application surface"))?;
    let min_y = front_to_back
        .iter()
        .map(|surface| surface.y)
        .min()
        .ok_or_else(|| internal("empty application surface"))?;
    let max_x = front_to_back
        .iter()
        .map(|surface| surface.x.saturating_add(i64::from(surface.width)))
        .max()
        .ok_or_else(|| internal("empty application surface"))?;
    let max_y = front_to_back
        .iter()
        .map(|surface| surface.y.saturating_add(i64::from(surface.height)))
        .max()
        .ok_or_else(|| internal("empty application surface"))?;
    let width = u32::try_from(max_x.saturating_sub(min_x))
        .map_err(|_| internal("application canvas width overflow"))?;
    let height = u32::try_from(max_y.saturating_sub(min_y))
        .map_err(|_| internal("application canvas height overflow"))?;
    let length = usize::try_from(u64::from(width) * u64::from(height) * 4)
        .map_err(|_| internal("application canvas allocation overflow"))?;
    let mut rgba = vec![0_u8; length];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    // Contract order is front-to-back, therefore paint the back first.
    for surface in front_to_back.iter().rev() {
        let image = captured
            .get(&surface.window_id)
            .ok_or_else(|| internal("captured application surface missing"))?;
        let offset_x = usize::try_from(surface.x - min_x)
            .map_err(|_| internal("application surface x offset overflow"))?;
        let offset_y = usize::try_from(surface.y - min_y)
            .map_err(|_| internal("application surface y offset overflow"))?;
        for y in 0..surface.height as usize {
            for x in 0..surface.width as usize {
                let source_offset = (y * surface.width as usize + x) * 4;
                let target_offset = ((offset_y + y) * width as usize + offset_x + x) * 4;
                let source = &image.as_raw()[source_offset..source_offset + 4];
                let target = &mut rgba[target_offset..target_offset + 4];
                let alpha = u16::from(source[3]);
                let inverse = 255 - alpha;
                for channel in 0..3 {
                    target[channel] = ((u16::from(source[channel]) * alpha
                        + u16::from(target[channel]) * inverse)
                        / 255) as u8;
                }
                target[3] = 255;
            }
        }
    }
    Ok(CapturedRgba {
        rgba,
        width,
        height,
    })
}

fn rgba_image(image: xcap::image::RgbaImage) -> Result<CapturedRgba, BackendFailure> {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return Err(target_invalidated("native capture returned an empty frame"));
    }
    Ok(CapturedRgba {
        rgba: image.into_raw(),
        width,
        height,
    })
}

fn resize_rgba_to_rgb(
    rgba: &[u8],
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
) -> Result<Vec<u8>, BackendFailure> {
    let source_len = usize::try_from(u64::from(source_width) * u64::from(source_height) * 4)
        .map_err(|_| internal("source frame size overflow"))?;
    if rgba.len() != source_len {
        return Err(internal("source RGBA frame length is inconsistent"));
    }
    let target_len = usize::try_from(u64::from(target_width) * u64::from(target_height) * 3)
        .map_err(|_| internal("target RGB frame size overflow"))?;
    let mut rgb = vec![0_u8; target_len];
    for target_y in 0..target_height {
        let source_y = u64::from(target_y) * u64::from(source_height) / u64::from(target_height);
        for target_x in 0..target_width {
            let source_x = u64::from(target_x) * u64::from(source_width) / u64::from(target_width);
            let source = ((source_y * u64::from(source_width) + source_x) * 4) as usize;
            let target = ((u64::from(target_y) * u64::from(target_width) + u64::from(target_x)) * 3)
                as usize;
            rgb[target..target + 3].copy_from_slice(&rgba[source..source + 3]);
        }
    }
    Ok(rgb)
}

fn build_encoder(config: &VideoConfig) -> Result<Encoder, BackendFailure> {
    let level = openh264_level(config.h264_level_idc).ok_or_else(|| {
        BackendFailure::new(
            FailureReason::EncoderUnavailable,
            format!("unsupported OpenH264 level_idc {}", config.h264_level_idc),
        )
    })?;
    let encoder_config = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .rate_control_mode(RateControlMode::Bitrate)
        .bitrate(BitRate::from_bps(config.bitrate_kbps.saturating_mul(1_000)))
        .max_frame_rate(FrameRate::from_hz(config.fps as f32))
        .profile(Profile::Baseline)
        .level(level)
        .complexity(Complexity::Low)
        .max_slice_len(config.max_nal_unit_bytes)
        .intra_frame_period(IntraFramePeriod::from_num_frames(
            config.keyframe_interval_frames,
        ));
    Encoder::with_api_config(OpenH264API::from_source(), encoder_config).map_err(|error| {
        BackendFailure::new(
            FailureReason::EncoderUnavailable,
            format!("initialize OpenH264: {error}"),
        )
    })
}

fn openh264_level(level_idc: u8) -> Option<OpenH264Level> {
    Some(match level_idc {
        10 => OpenH264Level::Level_1_0,
        9 => OpenH264Level::Level_1_B,
        11 => OpenH264Level::Level_1_1,
        12 => OpenH264Level::Level_1_2,
        13 => OpenH264Level::Level_1_3,
        20 => OpenH264Level::Level_2_0,
        21 => OpenH264Level::Level_2_1,
        22 => OpenH264Level::Level_2_2,
        30 => OpenH264Level::Level_3_0,
        31 => OpenH264Level::Level_3_1,
        32 => OpenH264Level::Level_3_2,
        40 => OpenH264Level::Level_4_0,
        41 => OpenH264Level::Level_4_1,
        42 => OpenH264Level::Level_4_2,
        50 => OpenH264Level::Level_5_0,
        51 => OpenH264Level::Level_5_1,
        52 => OpenH264Level::Level_5_2,
        _ => return None,
    })
}

fn inspect_annex_b(payload: &[u8]) -> (bool, bool) {
    let mut has_idr = false;
    let mut has_sps = false;
    let mut has_pps = false;
    let mut index = 0;
    while index + 3 < payload.len() {
        let prefix = if payload[index..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if payload[index..].starts_with(&[0, 0, 1]) {
            3
        } else {
            index += 1;
            continue;
        };
        if let Some(header) = payload.get(index + prefix) {
            match header & 0x1f {
                5 => has_idr = true,
                7 => has_sps = true,
                8 => has_pps = true,
                _ => {}
            }
        }
        index += prefix;
    }
    (has_idr, has_sps && has_pps)
}

fn verify_process_instance(
    process_identity_provider: &PlatformWindowProcessIdentityProvider,
    pid: u32,
    expected: Option<&str>,
) -> Result<(), BackendFailure> {
    let expected = expected.ok_or_else(|| {
        target_invalidated("window/application target lacks process-instance identity")
    })?;
    let observed = process_identity_provider
        .resolve_process(pid)
        .map_err(|error| target_invalidated(format!("resolve process {pid} identity: {error}")))?;
    observed.verify(expected).map_err(target_invalidated)
}

fn capture_unavailable(error: impl std::fmt::Display) -> BackendFailure {
    BackendFailure::new(
        FailureReason::CaptureUnavailable,
        format!("{} xcap capture failed: {error}", std::env::consts::OS),
    )
}

fn target_invalidated(detail: impl Into<String>) -> BackendFailure {
    BackendFailure::new(FailureReason::TargetInvalidated, detail)
}

fn internal(detail: impl Into<String>) -> BackendFailure {
    BackendFailure::new(FailureReason::Internal, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_resize_preserves_rgba_rgb_channels() {
        let rgb = resize_rgba_to_rgb(&[255, 0, 0, 255, 0, 255, 0, 255], 2, 1, 4, 2).unwrap();
        assert_eq!(&rgb[0..6], &[255, 0, 0, 255, 0, 0]);
        assert_eq!(&rgb[6..12], &[0, 255, 0, 0, 255, 0]);
    }

    #[test]
    fn annex_b_inspection_distinguishes_idr_and_parameter_sets() {
        assert_eq!(
            inspect_annex_b(&[0, 0, 0, 1, 0x67, 1, 0, 0, 1, 0x68, 1, 0, 0, 1, 0x65, 1,]),
            (true, true)
        );
        assert_eq!(inspect_annex_b(&[0, 0, 1, 0x41, 1]), (false, false));
    }
}

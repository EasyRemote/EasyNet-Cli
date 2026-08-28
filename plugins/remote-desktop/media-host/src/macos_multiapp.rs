// EasyNet CLI — ScreenCaptureKit multi-application surface
// =========================================================
//
// File: plugins/remote-desktop/media-host/src/macos_multiapp.rs
// Description: Bounded multi-window ScreenCaptureKit target layout and BGRA
// compositor for one complete macOS application Resource.
//
// Protocol Responsibility:
// - Owns only the plugin-native representation of one AppSurface.
// - Does not create Invocation, authority, receipt, session, or WebRTC state.
//
// Implementation Approach:
// - Resolve one desktop-independent SCContentFilter per committed SCWindow.
// - Scale all surfaces into one negotiated output canvas.
// - Retain only one fresh BGRA frame per surface and alpha-compose at most the
//   negotiated FPS into one CoreVideo pixel buffer. The first frame waits one
//   bounded frame interval for all committed surfaces; a dormant window then
//   becomes opaque black instead of stalling the stream. A single deferred
//   flush preserves static surface updates suppressed by throttling.
//
// Architectural Position:
// - Plugin-private RemoteApp media-host capture adapter (macOS only).

// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

#![cfg(target_os = "macos")]

use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use dispatch2::{DispatchQueue, DispatchRetained, DispatchTime};
use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_core_foundation::{CFRetained, CGRect};
use objc2_core_media::CMTime;
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, kCVReturnSuccess, CVImageBuffer, CVPixelBuffer, CVPixelBufferCreate,
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_screen_capture_kit::{SCContentFilter, SCShareableContent, SCWindow};

use super::macos_sck::{CapturedFrame, FrameSink};

const MAX_MULTI_APP_WINDOWS: usize = 32;
const MAX_MULTI_APP_PIXELS: u64 = 33_177_600;

pub(super) struct MultiAppSurfaceTarget {
    surfaces: Vec<NativeAppWindowSurface>,
    origin_x: f64,
    origin_y: f64,
    logical_width: usize,
    logical_height: usize,
    native_width: usize,
    native_height: usize,
}

struct NativeAppWindowSurface {
    window_id: u64,
    filter: Retained<SCContentFilter>,
    frame: CGRect,
    /// SCShareableContent enumerates front-to-back. Lower values are nearer
    /// the user; composition therefore draws larger values first.
    front_to_back_index: usize,
}

pub(super) struct ScaledAppWindowSurface {
    pub window_id: u64,
    pub filter: Retained<SCContentFilter>,
    pub offset_x: usize,
    pub offset_y: usize,
    pub width: usize,
    pub height: usize,
    pub front_to_back_index: usize,
}

impl MultiAppSurfaceTarget {
    pub fn from_windows(windows: Vec<Retained<SCWindow>>) -> anyhow::Result<Self> {
        if windows.is_empty() || windows.len() > MAX_MULTI_APP_WINDOWS {
            anyhow::bail!(
                "macOS application surface requires 1..={MAX_MULTI_APP_WINDOWS} committed windows; observed {}",
                windows.len()
            );
        }
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut maximum_point_pixel_scale = 1.0_f64;
        let mut surfaces = Vec::with_capacity(windows.len());
        for (front_to_back_index, window) in windows.into_iter().enumerate() {
            let frame = unsafe { window.frame() };
            if !valid_frame(frame) {
                anyhow::bail!(
                    "committed ScreenCaptureKit window {} has invalid frame {:?}",
                    unsafe { window.windowID() as u64 },
                    frame
                );
            }
            min_x = min_x.min(frame.origin.x);
            min_y = min_y.min(frame.origin.y);
            max_x = max_x.max(frame.origin.x + frame.size.width);
            max_y = max_y.max(frame.origin.y + frame.size.height);
            let filter = unsafe {
                SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
            };
            let point_pixel_scale =
                f64::from(unsafe { SCShareableContent::infoForFilter(&filter).pointPixelScale() });
            if point_pixel_scale.is_finite() && point_pixel_scale > 0.0 {
                maximum_point_pixel_scale = maximum_point_pixel_scale.max(point_pixel_scale);
            }
            surfaces.push(NativeAppWindowSurface {
                window_id: unsafe { window.windowID() as u64 },
                filter,
                frame,
                front_to_back_index,
            });
        }
        let logical_width = finite_dimension(max_x - min_x)
            .ok_or_else(|| anyhow::anyhow!("macOS application surface union width is invalid"))?;
        let logical_height = finite_dimension(max_y - min_y)
            .ok_or_else(|| anyhow::anyhow!("macOS application surface union height is invalid"))?;
        let native_width = scaled_native_dimension(logical_width, maximum_point_pixel_scale)
            .ok_or_else(|| {
                anyhow::anyhow!("macOS application surface native pixel width is invalid")
            })?;
        let native_height = scaled_native_dimension(logical_height, maximum_point_pixel_scale)
            .ok_or_else(|| {
                anyhow::anyhow!("macOS application surface native pixel height is invalid")
            })?;
        ensure_canvas_bound(native_width, native_height)?;
        Ok(Self {
            surfaces,
            origin_x: min_x,
            origin_y: min_y,
            logical_width,
            logical_height,
            native_width,
            native_height,
        })
    }

    pub fn native_dimensions(&self) -> (usize, usize) {
        (self.native_width, self.native_height)
    }

    pub fn scale_to(
        &self,
        width: usize,
        height: usize,
    ) -> anyhow::Result<Vec<ScaledAppWindowSurface>> {
        ensure_canvas_bound(width, height)?;
        let scale_x = width as f64 / self.logical_width as f64;
        let scale_y = height as f64 / self.logical_height as f64;
        self.surfaces
            .iter()
            .map(|surface| {
                let offset_x = ((surface.frame.origin.x - self.origin_x) * scale_x).round();
                let offset_y = ((surface.frame.origin.y - self.origin_y) * scale_y).round();
                let surface_width = (surface.frame.size.width * scale_x).round().max(1.0);
                let surface_height = (surface.frame.size.height * scale_y).round().max(1.0);
                let offset_x = usize::try_from(offset_x as i128)
                    .map_err(|_| invalid_layout(surface.window_id, "negative scaled x offset"))?;
                let offset_y = usize::try_from(offset_y as i128)
                    .map_err(|_| invalid_layout(surface.window_id, "negative scaled y offset"))?;
                let surface_width = usize::try_from(surface_width as u128)
                    .map_err(|_| invalid_layout(surface.window_id, "scaled width out of range"))?;
                let surface_height = usize::try_from(surface_height as u128)
                    .map_err(|_| invalid_layout(surface.window_id, "scaled height out of range"))?;
                if offset_x.saturating_add(surface_width) > width
                    || offset_y.saturating_add(surface_height) > height
                {
                    return Err(invalid_layout(
                        surface.window_id,
                        "scaled surface exceeds output canvas",
                    ));
                }
                Ok(ScaledAppWindowSurface {
                    window_id: surface.window_id,
                    filter: surface.filter.clone(),
                    offset_x,
                    offset_y,
                    width: surface_width,
                    height: surface_height,
                    front_to_back_index: surface.front_to_back_index,
                })
            })
            .collect()
    }
}

pub(super) struct MultiAppSurfaceCompositor {
    state: Mutex<CompositorState>,
    sink: FrameSink,
    deferred_queue: DispatchRetained<DispatchQueue>,
}

struct CompositorState {
    width: usize,
    height: usize,
    surfaces: Vec<SurfaceLayout>,
    latest: Vec<Option<LatestSurfaceFrame>>,
    minimum_emit_interval: Duration,
    last_emitted_at: Option<Instant>,
    last_emitted_pts: Option<CMTime>,
    deferred_emit_scheduled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SurfaceLayout {
    offset_x: usize,
    offset_y: usize,
    width: usize,
    height: usize,
    front_to_back_index: usize,
}

struct LatestSurfaceFrame {
    bytes: Vec<u8>,
    stride: usize,
    width: usize,
    height: usize,
    pts: CMTime,
}

impl MultiAppSurfaceCompositor {
    pub fn new(
        width: usize,
        height: usize,
        surfaces: &[ScaledAppWindowSurface],
        fps: u32,
        sink: FrameSink,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(CompositorState {
                width,
                height,
                surfaces: surfaces
                    .iter()
                    .map(|surface| SurfaceLayout {
                        offset_x: surface.offset_x,
                        offset_y: surface.offset_y,
                        width: surface.width,
                        height: surface.height,
                        front_to_back_index: surface.front_to_back_index,
                    })
                    .collect(),
                latest: (0..surfaces.len()).map(|_| None).collect(),
                minimum_emit_interval: Duration::from_secs_f64(1.0 / fps.max(1) as f64),
                last_emitted_at: None,
                last_emitted_pts: None,
                deferred_emit_scheduled: false,
            }),
            sink,
            deferred_queue: DispatchQueue::new(
                "tech.easynet.remote-desktop.multi-app-compositor",
                None,
            ),
        })
    }

    pub fn accept(
        self: &Arc<Self>,
        surface_index: usize,
        frame: CapturedFrame,
    ) -> anyhow::Result<()> {
        let mut latest = copy_bgra_surface(&frame.image_buffer)?;
        latest.pts = frame.pts;
        let now = Instant::now();
        let (composed, deferred_delay) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(slot) = state.latest.get_mut(surface_index) else {
                anyhow::bail!("multi-app compositor surface index {surface_index} is out of range");
            };
            *slot = Some(latest);
            let all_surfaces_ready = state.latest.iter().all(Option::is_some);
            if state.last_emitted_at.is_none() && !all_surfaces_ready {
                let minimum_emit_interval = state.minimum_emit_interval;
                let delay = schedule_deferred_if_needed(&mut state, minimum_emit_interval);
                (None, delay)
            } else {
                compose_or_schedule(&mut state, now, false)?
            }
        };
        if let Some(frame) = composed {
            (self.sink)(frame);
        }
        if let Some(delay) = deferred_delay {
            self.schedule_deferred(delay);
        }
        Ok(())
    }

    fn schedule_deferred(self: &Arc<Self>, delay: Duration) {
        let weak = Arc::downgrade(self);
        let when = DispatchTime::try_from(delay).unwrap_or(DispatchTime::NOW);
        if self
            .deferred_queue
            .after(when, move || flush_deferred(weak))
            .is_err()
        {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .deferred_emit_scheduled = false;
        }
    }

    fn flush_deferred(self: &Arc<Self>) -> anyhow::Result<()> {
        let now = Instant::now();
        let (composed, deferred_delay) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.deferred_emit_scheduled = false;
            compose_or_schedule(&mut state, now, true)?
        };
        if let Some(frame) = composed {
            (self.sink)(frame);
        }
        if let Some(delay) = deferred_delay {
            self.schedule_deferred(delay);
        }
        Ok(())
    }
}

fn flush_deferred(compositor: Weak<MultiAppSurfaceCompositor>) {
    let Some(compositor) = compositor.upgrade() else {
        return;
    };
    if let Err(error) = compositor.flush_deferred() {
        eprintln!("[remoteapp-media-host] deferred multi-app composition failed: {error}");
    }
}

fn compose_or_schedule(
    state: &mut CompositorState,
    now: Instant,
    allow_initial_partial: bool,
) -> anyhow::Result<(Option<CapturedFrame>, Option<Duration>)> {
    let composite_pts = greatest_surface_pts(&state.latest)?;
    if state
        .last_emitted_pts
        .is_some_and(|last| unsafe { composite_pts.compare(last) } <= 0)
    {
        return Ok((None, None));
    }
    if !allow_initial_partial {
        if let Some(last) = state.last_emitted_at {
            let elapsed = now.saturating_duration_since(last);
            if elapsed < state.minimum_emit_interval {
                let remaining = state.minimum_emit_interval.saturating_sub(elapsed);
                let delay = schedule_deferred_if_needed(state, remaining);
                return Ok((None, delay));
            }
        }
    }
    let image_buffer = compose_locked_surfaces(state)?;
    state.last_emitted_at = Some(now);
    state.last_emitted_pts = Some(composite_pts);
    Ok((
        Some(CapturedFrame {
            image_buffer,
            pts: composite_pts,
        }),
        None,
    ))
}

fn schedule_deferred_if_needed(state: &mut CompositorState, delay: Duration) -> Option<Duration> {
    if state.deferred_emit_scheduled {
        None
    } else {
        state.deferred_emit_scheduled = true;
        Some(delay)
    }
}

fn copy_bgra_surface(image_buffer: &CVImageBuffer) -> anyhow::Result<LatestSurfaceFrame> {
    let format = CVPixelBufferGetPixelFormatType(image_buffer);
    if format != kCVPixelFormatType_32BGRA {
        anyhow::bail!("multi-app ScreenCaptureKit surface pixel format 0x{format:08x} is not BGRA");
    }
    let flags = CVPixelBufferLockFlags::ReadOnly;
    let lock = unsafe { CVPixelBufferLockBaseAddress(image_buffer, flags) };
    if lock != kCVReturnSuccess {
        anyhow::bail!("multi-app CVPixelBufferLockBaseAddress failed with {lock}");
    }
    let result = (|| {
        let width = CVPixelBufferGetWidth(image_buffer);
        let height = CVPixelBufferGetHeight(image_buffer);
        let stride = CVPixelBufferGetBytesPerRow(image_buffer);
        let base = CVPixelBufferGetBaseAddress(image_buffer);
        if width == 0 || height == 0 || stride < width.saturating_mul(4) || base.is_null() {
            anyhow::bail!("multi-app ScreenCaptureKit surface returned invalid BGRA buffer");
        }
        let bytes = unsafe { std::slice::from_raw_parts(base.cast::<u8>(), stride * height) };
        Ok(LatestSurfaceFrame {
            bytes: bytes.to_vec(),
            stride,
            width,
            height,
            pts: unsafe { objc2_core_media::kCMTimeInvalid },
        })
    })();
    let _ = unsafe { CVPixelBufferUnlockBaseAddress(image_buffer, flags) };
    result
}

fn greatest_surface_pts(latest: &[Option<LatestSurfaceFrame>]) -> anyhow::Result<CMTime> {
    let mut pts: Option<CMTime> = None;
    for surface in latest {
        let Some(surface) = surface.as_ref() else {
            continue;
        };
        if !surface
            .pts
            .flags
            .contains(objc2_core_media::CMTimeFlags::Valid)
        {
            anyhow::bail!("multi-app ScreenCaptureKit surface returned invalid presentation time");
        }
        pts = Some(match pts {
            Some(current) => unsafe { current.maximum(surface.pts) },
            None => surface.pts,
        });
    }
    pts.ok_or_else(|| anyhow::anyhow!("multi-app compositor has no surface presentation time"))
}

fn compose_locked_surfaces(state: &CompositorState) -> anyhow::Result<Retained<CVImageBuffer>> {
    let mut raw = ptr::null_mut::<CVPixelBuffer>();
    let result = unsafe {
        CVPixelBufferCreate(
            None,
            state.width,
            state.height,
            kCVPixelFormatType_32BGRA,
            None,
            NonNull::from(&mut raw),
        )
    };
    if result != kCVReturnSuccess {
        anyhow::bail!("multi-app CVPixelBufferCreate failed with {result}");
    }
    let raw = NonNull::new(raw)
        .ok_or_else(|| anyhow::anyhow!("multi-app CVPixelBufferCreate returned null"))?;
    let buffer = unsafe { CFRetained::from_raw(raw) };
    let flags = CVPixelBufferLockFlags::empty();
    let lock = unsafe { CVPixelBufferLockBaseAddress(&buffer, flags) };
    if lock != kCVReturnSuccess {
        anyhow::bail!("multi-app output CVPixelBufferLockBaseAddress failed with {lock}");
    }
    let compose_result = (|| {
        let stride = CVPixelBufferGetBytesPerRow(&buffer);
        let base = CVPixelBufferGetBaseAddress(&buffer);
        if stride < state.width.saturating_mul(4) || base.is_null() {
            anyhow::bail!("multi-app output CVPixelBuffer has invalid storage");
        }
        let destination = unsafe {
            std::slice::from_raw_parts_mut(base.cast::<u8>(), stride.saturating_mul(state.height))
        };
        for pixel in destination.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[0, 0, 0, 255]);
        }
        let mut order = (0..state.surfaces.len()).collect::<Vec<_>>();
        order.sort_by_key(|index| std::cmp::Reverse(state.surfaces[*index].front_to_back_index));
        for index in order {
            let layout = state.surfaces[index];
            if let Some(source) = state.latest[index].as_ref() {
                alpha_blend_bgra(destination, stride, source, layout)?;
            }
        }
        Ok(())
    })();
    let _ = unsafe { CVPixelBufferUnlockBaseAddress(&buffer, flags) };
    compose_result?;
    Ok(buffer.into())
}

fn alpha_blend_bgra(
    destination: &mut [u8],
    destination_stride: usize,
    source: &LatestSurfaceFrame,
    layout: SurfaceLayout,
) -> anyhow::Result<()> {
    let destination_height = destination.len() / destination_stride.max(1);
    if layout.offset_x.saturating_add(layout.width) > destination_stride / 4
        || layout.offset_y.saturating_add(layout.height) > destination_height
    {
        anyhow::bail!("multi-app compositor destination layout exceeds canvas");
    }
    if source.width != layout.width
        || source.height != layout.height
        || source.bytes.len() < source.stride.saturating_mul(layout.height)
        || source.stride < layout.width.saturating_mul(4)
    {
        anyhow::bail!("multi-app compositor source dimensions do not match layout");
    }
    for y in 0..layout.height {
        let source_row = &source.bytes[y * source.stride..y * source.stride + layout.width * 4];
        let destination_start = (layout.offset_y + y) * destination_stride + layout.offset_x * 4;
        let destination_row =
            &mut destination[destination_start..destination_start + layout.width * 4];
        for (source_pixel, destination_pixel) in source_row
            .chunks_exact(4)
            .zip(destination_row.chunks_exact_mut(4))
        {
            let alpha = u16::from(source_pixel[3]);
            let inverse = 255_u16 - alpha;
            for channel in 0..3 {
                destination_pixel[channel] = ((u16::from(source_pixel[channel]) * alpha
                    + u16::from(destination_pixel[channel]) * inverse)
                    / 255) as u8;
            }
            destination_pixel[3] = 255;
        }
    }
    Ok(())
}

fn valid_frame(frame: CGRect) -> bool {
    frame.origin.x.is_finite()
        && frame.origin.y.is_finite()
        && frame.size.width.is_finite()
        && frame.size.height.is_finite()
        && frame.size.width > 0.0
        && frame.size.height > 0.0
}

fn finite_dimension(value: f64) -> Option<usize> {
    (value.is_finite() && value > 0.0 && value <= usize::MAX as f64).then(|| value.ceil() as usize)
}

fn scaled_native_dimension(logical_dimension: usize, point_pixel_scale: f64) -> Option<usize> {
    finite_dimension(logical_dimension as f64 * point_pixel_scale)
}

fn ensure_canvas_bound(width: usize, height: usize) -> anyhow::Result<()> {
    let pixels = (width as u128).saturating_mul(height as u128);
    if width == 0 || height == 0 || pixels > u128::from(MAX_MULTI_APP_PIXELS) {
        anyhow::bail!(
            "macOS multi-app surface {width}x{height} exceeds bounded pixel limit {MAX_MULTI_APP_PIXELS}"
        );
    }
    Ok(())
}

fn invalid_layout(window_id: u64, detail: &str) -> anyhow::Error {
    anyhow::anyhow!("committed window {window_id} has invalid multi-app layout: {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_compositor_preserves_black_gaps_and_front_window_order() {
        let mut destination = vec![0_u8; 5 * 4];
        for pixel in destination.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        let red = LatestSurfaceFrame {
            bytes: vec![0, 0, 255, 255, 0, 0, 255, 255],
            stride: 8,
            width: 2,
            height: 1,
            pts: valid_pts(1),
        };
        alpha_blend_bgra(
            &mut destination,
            20,
            &red,
            SurfaceLayout {
                offset_x: 0,
                offset_y: 0,
                width: 2,
                height: 1,
                front_to_back_index: 1,
            },
        )
        .expect("back window blends");
        let green = LatestSurfaceFrame {
            bytes: vec![0, 255, 0, 255, 0, 255, 0, 255],
            stride: 8,
            width: 2,
            height: 1,
            pts: valid_pts(2),
        };
        alpha_blend_bgra(
            &mut destination,
            20,
            &green,
            SurfaceLayout {
                offset_x: 1,
                offset_y: 0,
                width: 2,
                height: 1,
                front_to_back_index: 0,
            },
        )
        .expect("front window blends");
        assert_eq!(&destination[0..4], &[0, 0, 255, 255]);
        assert_eq!(&destination[4..12], &[0, 255, 0, 255, 0, 255, 0, 255]);
        assert_eq!(&destination[12..20], &[0, 0, 0, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn compositor_rejects_source_layout_mismatch() {
        let mut destination = vec![0_u8; 8];
        let source = LatestSurfaceFrame {
            bytes: vec![0; 4],
            stride: 4,
            width: 1,
            height: 1,
            pts: valid_pts(1),
        };
        let err = alpha_blend_bgra(
            &mut destination,
            8,
            &source,
            SurfaceLayout {
                offset_x: 0,
                offset_y: 0,
                width: 2,
                height: 1,
                front_to_back_index: 0,
            },
        )
        .expect_err("short source must fail closed");
        assert!(err.to_string().contains("do not match layout"));
    }

    #[test]
    fn compositor_uses_greatest_surface_pts_for_monotonic_output() {
        let frames = vec![
            Some(frame_with_pts(20)),
            Some(frame_with_pts(10)),
            Some(frame_with_pts(30)),
        ];
        let greatest = greatest_surface_pts(&frames).expect("valid greatest pts");
        assert_eq!(unsafe { greatest.compare(valid_pts(30)) }, 0);
    }

    #[test]
    fn compositor_does_not_wait_for_a_dormant_committed_window() {
        let state = CompositorState {
            width: 2,
            height: 1,
            surfaces: vec![
                SurfaceLayout {
                    offset_x: 0,
                    offset_y: 0,
                    width: 1,
                    height: 1,
                    front_to_back_index: 0,
                },
                SurfaceLayout {
                    offset_x: 1,
                    offset_y: 0,
                    width: 1,
                    height: 1,
                    front_to_back_index: 1,
                },
            ],
            latest: vec![Some(frame_with_pts(1)), None],
            minimum_emit_interval: Duration::ZERO,
            last_emitted_at: None,
            last_emitted_pts: None,
            deferred_emit_scheduled: false,
        };

        compose_locked_surfaces(&state)
            .expect("a ready application surface must not wait for a dormant window");
    }

    #[test]
    fn initial_batch_prefers_all_surfaces_and_deferred_flush_preserves_static_updates() {
        let now = Instant::now();
        let mut state = CompositorState {
            width: 2,
            height: 1,
            surfaces: vec![
                SurfaceLayout {
                    offset_x: 0,
                    offset_y: 0,
                    width: 1,
                    height: 1,
                    front_to_back_index: 0,
                },
                SurfaceLayout {
                    offset_x: 1,
                    offset_y: 0,
                    width: 1,
                    height: 1,
                    front_to_back_index: 1,
                },
            ],
            latest: vec![Some(frame_with_pts(1)), None],
            minimum_emit_interval: Duration::from_millis(16),
            last_emitted_at: None,
            last_emitted_pts: None,
            deferred_emit_scheduled: false,
        };

        assert_eq!(
            schedule_deferred_if_needed(&mut state, Duration::from_millis(16)),
            Some(Duration::from_millis(16))
        );
        state.latest[1] = Some(frame_with_pts(2));
        let (complete, duplicate_schedule) =
            compose_or_schedule(&mut state, now, false).expect("complete initial batch");
        assert!(complete.is_some());
        assert!(duplicate_schedule.is_none());

        state.latest[1] = Some(frame_with_pts(3));
        let (throttled, duplicate_schedule) =
            compose_or_schedule(&mut state, now, false).expect("throttled static update");
        assert!(throttled.is_none());
        assert!(
            duplicate_schedule.is_none(),
            "one deferred flush already exists"
        );

        state.deferred_emit_scheduled = false;
        let (flushed, reschedule) =
            compose_or_schedule(&mut state, now, true).expect("deferred static update flush");
        assert!(flushed.is_some());
        assert!(reschedule.is_none());
        assert_eq!(
            unsafe { state.last_emitted_pts.unwrap().compare(valid_pts(3)) },
            0
        );
    }

    #[test]
    fn native_canvas_dimension_applies_retina_point_pixel_scale() {
        assert_eq!(scaled_native_dimension(1_440, 2.0), Some(2_880));
        assert_eq!(scaled_native_dimension(1_440, 1.0), Some(1_440));
        assert_eq!(scaled_native_dimension(1_440, f64::NAN), None);
    }

    fn frame_with_pts(value: i64) -> LatestSurfaceFrame {
        LatestSurfaceFrame {
            bytes: vec![0, 0, 0, 255],
            stride: 4,
            width: 1,
            height: 1,
            pts: valid_pts(value),
        }
    }

    fn valid_pts(value: i64) -> CMTime {
        CMTime {
            value,
            timescale: 60,
            flags: objc2_core_media::CMTimeFlags::Valid,
            epoch: 0,
        }
    }
}

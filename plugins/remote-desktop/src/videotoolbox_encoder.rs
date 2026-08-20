// EasyNet CLI — VideoToolbox H.264 hardware encoder
// ==================================================
//
// File: plugins/remote-desktop/src/videotoolbox_encoder.rs
// Description: macOS VideoToolbox VTCompressionSession wrapper that
// hardware-encodes CVPixelBuffers to H.264 Annex-B for the WebRTC
// RTP/SRTP carrier.
//
// Protocol Responsibility:
// - Phase 1 of plugin.macos.screencapturekit.videotoolbox.webrtc.v1.
// - Owns ONLY encode: pixel buffer in, Annex-B access units out. It
//   does not capture (ScreenCaptureKit, phase 2) nor packetize
//   (WebRTC track, phase 3).
//
// Implementation Approach:
// - VTCompressionSessionCreate with the H.264 codec, real-time
//   ScreenContent tuning, and an async output callback that funnels
//   compressed CMSampleBuffers into a channel.
// - VideoToolbox emits AVCC (4-byte length-prefixed) NAL units; WebRTC
//   needs Annex-B (00 00 00 01 start codes). We convert, and on
//   keyframes we prepend SPS/PPS pulled from the format description.
//
// Architectural Position:
// - EasyNet-Cli device adapter, native media plugin (macOS only).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

#![cfg(target_os = "macos")]

use std::collections::VecDeque;
use std::ffi::{c_int, c_void};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use objc2_core_foundation::{kCFBooleanTrue, CFRetained};
use objc2_core_media::{
    kCMVideoCodecType_H264, CMSampleBuffer, CMTime, CMTimeFlags,
    CMVideoFormatDescriptionGetH264ParameterSetAtIndex,
};
use objc2_core_video::CVImageBuffer;
use objc2_video_toolbox::{
    kVTCompressionPropertyKey_AllowFrameReordering, kVTCompressionPropertyKey_AllowOpenGOP,
    kVTCompressionPropertyKey_AverageBitRate, kVTCompressionPropertyKey_ExpectedFrameRate,
    kVTCompressionPropertyKey_MaxFrameDelayCount, kVTCompressionPropertyKey_MaxKeyFrameInterval,
    kVTCompressionPropertyKey_MaximumRealTimeFrameRate,
    kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality,
    kVTCompressionPropertyKey_ProfileLevel, kVTCompressionPropertyKey_RealTime,
    kVTProfileLevel_H264_Baseline_AutoLevel, VTCompressionSession, VTEncodeInfoFlags, VTSession,
    VTSessionSetProperty,
};

/// Maximum compressed access units waiting for the WebRTC RTP writer.
///
/// Invariant 1: the native desktop path is live media, not a reliable file
/// stream. If the writer falls behind, older frames are discarded so the
/// browser sees the freshest desktop state.
const ENCODED_OUTPUT_QUEUE_CAPACITY: usize = 4;

/// VideoToolbox compression window limit. A value of one allows inter-frame
/// compression while forcing the encoder to emit promptly.
const MAX_FRAME_DELAY_COUNT: i32 = 1;

/// How long a submitted frame may sit without an output callback before its
/// in-flight slot is reclaimed.
///
/// VideoToolbox normally calls back within a frame interval; a frame older than
/// this is treated as silently dropped by the encoder. The bound is generous
/// (orders of magnitude above a 144 Hz interval) so it only fires on genuine
/// loss, never on healthy jitter — without it, two lost callbacks would pin the
/// in-flight count at the ceiling and starve the stream permanently.
const ENCODER_FRAME_STALE_MS: u64 = 1_000;

/// One encoded H.264 access unit in Annex-B byte-stream format.
#[derive(Debug, Clone)]
pub struct EncodedAccessUnit {
    pub annexb: Vec<u8>,
    pub is_keyframe: bool,
    pub pts_ms: u64,
    pub encode_submitted_at_ms: u64,
    pub encoded_at_ms: u64,
    pub encode_latency_ms: u64,
}

/// Snapshot of the native encoder's live-media backpressure state.
///
/// This is not a protocol type. The plugin maps it into product session events so
/// operators can distinguish true high-refresh WebRTC from hidden queueing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoToolboxEncoderStats {
    pub submitted_frames: u64,
    pub input_dropped_frames: u64,
    pub output_dropped_units: u64,
    pub emitted_units: u64,
    pub queued_units: usize,
    pub in_flight_frames: usize,
    pub max_in_flight_frames: usize,
    pub configured_bitrate_kbps: u32,
}

/// A VideoToolbox compression session handle that can encode from any thread.
///
/// VideoToolbox documents VTCompressionSessionEncodeFrame as callable from
/// arbitrary threads, so this handle is `Send + Sync`; the SCK capture
/// callback (a different thread from the media drain loop) holds a clone to
/// submit frames.
#[derive(Clone)]
pub struct EncoderSession {
    session: CFRetained<VTCompressionSession>,
    in_flight: Arc<AtomicUsize>,
    max_in_flight_frames: usize,
    counters: Arc<EncoderCounters>,
    pending_timings: Arc<PendingFrameTimings>,
}

// SAFETY: VTCompressionSession is internally synchronized; encode_frame and
// complete_frames are safe to call concurrently per Apple's VideoToolbox
// threading contract.
unsafe impl Send for EncoderSession {}
unsafe impl Sync for EncoderSession {}

impl EncoderSession {
    /// Submit one captured frame for asynchronous encoding.
    ///
    /// Encoding is asynchronous; encoded units arrive later via
    /// [`VideoToolboxEncoder::poll`]. Frames are admitted only while fewer than
    /// the configured in-flight ceiling are outstanding; excess frames are
    /// counted as input drops and skipped to bound capture-side latency.
    pub fn encode(
        &self,
        image_buffer: &CVImageBuffer,
        pts: CMTime,
        duration: CMTime,
    ) -> anyhow::Result<()> {
        let submitted_at_ms = now_wall_ms();
        if !try_reserve_in_flight(&self.in_flight, self.max_in_flight_frames) {
            // The ceiling is full. If frames have been outstanding past the
            // stale deadline, the encoder silently dropped them; reclaim their
            // slots so a lost callback cannot starve the stream permanently.
            let stale = self
                .pending_timings
                .reclaim_stale(submitted_at_ms, ENCODER_FRAME_STALE_MS);
            for _ in 0..stale {
                release_in_flight(&self.in_flight);
                self.counters.input_dropped.fetch_add(1, Ordering::Relaxed);
            }
            if !try_reserve_in_flight(&self.in_flight, self.max_in_flight_frames) {
                self.counters.input_dropped.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        }
        let pts_ms = cmtime_to_ms(pts);
        // Eviction means VideoToolbox accepted earlier frames but never reported
        // them back; release their orphaned in-flight slots so the encoder
        // cannot deadlock at the ceiling.
        let evicted = self.pending_timings.push(pts_ms, submitted_at_ms);
        for _ in 0..evicted {
            release_in_flight(&self.in_flight);
            self.counters.input_dropped.fetch_add(1, Ordering::Relaxed);
        }
        self.counters.submitted.fetch_add(1, Ordering::Relaxed);
        let mut info_flags = VTEncodeInfoFlags::empty();
        let status = unsafe {
            self.session.encode_frame(
                image_buffer,
                pts,
                duration,
                None,
                std::ptr::null_mut(),
                &mut info_flags,
            )
        };
        if status != 0 {
            release_in_flight(&self.in_flight);
            self.pending_timings.remove(pts_ms);
            anyhow::bail!("VTCompressionSessionEncodeFrame failed: OSStatus={status}");
        }
        Ok(())
    }
}

/// Hardware H.264 encoder backed by a VideoToolbox compression session.
///
/// The session's output callback runs on a VideoToolbox-owned thread and
/// pushes [`EncodedAccessUnit`]s into a bounded latest-frame queue;
/// [`Self::poll`] drains it from the media loop's thread. [`Self::session`]
/// hands out a `Send + Sync` encode handle for the capture thread.
pub struct VideoToolboxEncoder {
    session: CFRetained<VTCompressionSession>,
    queue: Arc<EncodedAccessUnitQueue>,
    in_flight: Arc<AtomicUsize>,
    max_in_flight_frames: usize,
    counters: Arc<EncoderCounters>,
    pending_timings: Arc<PendingFrameTimings>,
    current_bitrate_kbps: AtomicUsize,
    // The callback context is heap-owned for the session lifetime; kept here
    // so it is dropped only after the session is invalidated.
    _ctx: Box<CallbackContext>,
}

pub type EncoderWakeup = Arc<dyn Fn() + Send + Sync>;

struct CallbackContext {
    queue: Arc<EncodedAccessUnitQueue>,
    in_flight: Arc<AtomicUsize>,
    pending_timings: Arc<PendingFrameTimings>,
    wakeup: Option<EncoderWakeup>,
}

struct EncoderCounters {
    submitted: AtomicU64,
    input_dropped: AtomicU64,
    emitted: AtomicU64,
}

impl EncoderCounters {
    fn new() -> Self {
        Self {
            submitted: AtomicU64::new(0),
            input_dropped: AtomicU64::new(0),
            emitted: AtomicU64::new(0),
        }
    }
}

struct EncodedAccessUnitQueue {
    inner: Mutex<VecDeque<EncodedAccessUnit>>,
    capacity: usize,
    dropped: AtomicU64,
}

#[derive(Debug, Clone, Copy)]
struct PendingFrameTiming {
    pts_ms: u64,
    submitted_at_ms: u64,
}

/// Submit timestamps for frames currently inside VideoToolbox, keyed by PTS.
///
/// The queue is bounded by the configured in-flight ceiling, so a healthy queue
/// never exceeds the session's latency budget.
/// If the encoder ever drops a submitted frame without an output callback, its
/// timing entry would otherwise leak; [`Self::push`] evicts the oldest stale
/// entry on overflow and reports it so the caller can rebalance the in-flight
/// slot it left behind.
#[derive(Debug)]
struct PendingFrameTimings {
    inner: Mutex<VecDeque<PendingFrameTiming>>,
    capacity: usize,
}

impl PendingFrameTimings {
    fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
        }
    }

    /// Record a submitted frame. Returns the number of stale entries evicted to
    /// stay within capacity — each evicted entry corresponds to a frame
    /// VideoToolbox accepted but never reported back, whose in-flight slot the
    /// caller must release.
    fn push(&self, pts_ms: u64, submitted_at_ms: u64) -> usize {
        let Ok(mut q) = self.inner.lock() else {
            return 0;
        };
        let mut evicted = 0;
        while q.len() >= self.capacity {
            if q.pop_front().is_none() {
                break;
            }
            evicted += 1;
        }
        q.push_back(PendingFrameTiming {
            pts_ms,
            submitted_at_ms,
        });
        evicted
    }

    /// Remove and return the timing for `pts_ms`, or `None` if absent. Unlike a
    /// fallback to the queue head, a miss yields `None` so the caller never
    /// attributes one frame's submit time to another frame's latency.
    fn take(&self, pts_ms: u64) -> Option<PendingFrameTiming> {
        let Ok(mut q) = self.inner.lock() else {
            return None;
        };
        let index = q.iter().position(|timing| timing.pts_ms == pts_ms)?;
        q.remove(index)
    }

    fn remove(&self, pts_ms: u64) {
        let Ok(mut q) = self.inner.lock() else {
            return;
        };
        if let Some(index) = q.iter().position(|timing| timing.pts_ms == pts_ms) {
            q.remove(index);
        }
    }

    /// Drop entries submitted more than `deadline_ms` before `now_ms` and return
    /// how many were dropped. Each corresponds to a frame the encoder accepted
    /// but never reported back, whose in-flight slot the caller must release.
    fn reclaim_stale(&self, now_ms: u64, deadline_ms: u64) -> usize {
        let Ok(mut q) = self.inner.lock() else {
            return 0;
        };
        let mut reclaimed = 0;
        while let Some(front) = q.front() {
            if now_ms.saturating_sub(front.submitted_at_ms) < deadline_ms {
                break;
            }
            q.pop_front();
            reclaimed += 1;
        }
        reclaimed
    }
}

impl EncodedAccessUnitQueue {
    fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity: capacity.max(1),
            dropped: AtomicU64::new(0),
        }
    }

    fn push_latest(&self, unit: EncodedAccessUnit) {
        let Ok(mut q) = self.inner.lock() else {
            return;
        };
        while q.len() >= self.capacity {
            let drop_index = q.iter().position(|queued| !queued.is_keyframe).unwrap_or(0);
            q.remove(drop_index);
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        q.push_back(unit);
    }

    fn drain(&self) -> Vec<EncodedAccessUnit> {
        let Ok(mut q) = self.inner.lock() else {
            return Vec::new();
        };
        q.drain(..).collect()
    }

    fn len(&self) -> usize {
        self.inner.lock().map(|q| q.len()).unwrap_or(0)
    }

    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl VideoToolboxEncoder {
    pub fn new_with_wakeup_and_in_flight(
        width: i32,
        height: i32,
        bitrate_kbps: u32,
        keyframe_interval: u32,
        fps_hint: u32,
        wakeup: Option<EncoderWakeup>,
        max_in_flight_frames: usize,
    ) -> anyhow::Result<Self> {
        let max_in_flight_frames = max_in_flight_frames.max(1);
        let queue = Arc::new(EncodedAccessUnitQueue::new(ENCODED_OUTPUT_QUEUE_CAPACITY));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let counters = Arc::new(EncoderCounters::new());
        let pending_timings = Arc::new(PendingFrameTimings::new(max_in_flight_frames));
        let ctx = Box::new(CallbackContext {
            queue: Arc::clone(&queue),
            in_flight: Arc::clone(&in_flight),
            pending_timings: Arc::clone(&pending_timings),
            wakeup,
        });
        let ctx_ptr = (&*ctx as *const CallbackContext) as *mut c_void;

        let mut session_out: *mut VTCompressionSession = std::ptr::null_mut();
        let status = unsafe {
            VTCompressionSession::create(
                None,
                width,
                height,
                kCMVideoCodecType_H264,
                None,
                None,
                None,
                Some(output_callback),
                ctx_ptr,
                NonNull::new(&mut session_out).expect("stack slot is non-null"),
            )
        };
        if status != 0 || session_out.is_null() {
            anyhow::bail!("VTCompressionSessionCreate failed: OSStatus={status}");
        }
        // Take ownership of the +1 retain returned by Create.
        let session = unsafe {
            CFRetained::from_raw(NonNull::new(session_out).expect("checked non-null above"))
        };

        configure_realtime_h264(&session, bitrate_kbps, keyframe_interval, fps_hint)?;
        let status = unsafe { session.prepare_to_encode_frames() };
        if status != 0 {
            anyhow::bail!("VTCompressionSessionPrepareToEncodeFrames failed: OSStatus={status}");
        }

        Ok(Self {
            session,
            queue,
            in_flight,
            max_in_flight_frames,
            counters,
            pending_timings,
            current_bitrate_kbps: AtomicUsize::new(bitrate_kbps as usize),
            _ctx: ctx,
        })
    }

    /// A `Send + Sync` handle for submitting frames from the capture thread.
    pub fn session(&self) -> EncoderSession {
        EncoderSession {
            session: self.session.clone(),
            in_flight: Arc::clone(&self.in_flight),
            max_in_flight_frames: self.max_in_flight_frames,
            counters: Arc::clone(&self.counters),
            pending_timings: Arc::clone(&self.pending_timings),
        }
    }

    /// Non-blocking drain of all currently-available encoded access units.
    pub fn poll(&self) -> Vec<EncodedAccessUnit> {
        let units = self.queue.drain();
        self.counters
            .emitted
            .fetch_add(units.len() as u64, Ordering::Relaxed);
        units
    }

    pub fn stats(&self) -> VideoToolboxEncoderStats {
        VideoToolboxEncoderStats {
            submitted_frames: self.counters.submitted.load(Ordering::Relaxed),
            input_dropped_frames: self.counters.input_dropped.load(Ordering::Relaxed),
            output_dropped_units: self.queue.dropped(),
            emitted_units: self.counters.emitted.load(Ordering::Relaxed),
            queued_units: self.queue.len(),
            in_flight_frames: self.in_flight.load(Ordering::Acquire),
            max_in_flight_frames: self.max_in_flight_frames,
            configured_bitrate_kbps: self.current_bitrate_kbps.load(Ordering::Relaxed) as u32,
        }
    }

    pub fn set_bitrate_kbps(&self, bitrate_kbps: u32) -> anyhow::Result<()> {
        set_average_bitrate(&self.session, bitrate_kbps)?;
        self.current_bitrate_kbps
            .store(bitrate_kbps as usize, Ordering::Relaxed);
        Ok(())
    }
}

impl Drop for VideoToolboxEncoder {
    fn drop(&mut self) {
        // Flush, then tear down deterministically before the callback context
        // (held in _ctx) is freed.
        unsafe {
            let _ = self.session.complete_frames(invalid_cmtime());
            self.session.invalidate();
        }
    }
}

fn configure_realtime_h264(
    session: &VTCompressionSession,
    bitrate_kbps: u32,
    keyframe_interval: u32,
    fps_hint: u32,
) -> anyhow::Result<()> {
    use objc2_core_foundation::{CFNumber, CFType};

    let real_time: &CFType = unsafe { kCFBooleanTrue }
        .map(|b| b.as_ref())
        .ok_or_else(|| anyhow::anyhow!("kCFBooleanTrue unavailable"))?;
    set_property(
        session,
        unsafe { kVTCompressionPropertyKey_RealTime },
        real_time,
    )?;

    // AllowFrameReordering=false keeps latency low (no B-frames buffered).
    set_property_bool(
        session,
        unsafe { kVTCompressionPropertyKey_AllowFrameReordering },
        false,
    )?;
    let _ = set_property_bool(
        session,
        unsafe { kVTCompressionPropertyKey_AllowOpenGOP },
        false,
    );
    let _ = set_property_bool(
        session,
        unsafe { kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality },
        true,
    );
    let _ = set_property_i32(
        session,
        unsafe { kVTCompressionPropertyKey_MaxFrameDelayCount },
        MAX_FRAME_DELAY_COUNT,
    );

    let profile: &CFType =
        unsafe { &*(kVTProfileLevel_H264_Baseline_AutoLevel as *const _ as *const CFType) };
    set_property(
        session,
        unsafe { kVTCompressionPropertyKey_ProfileLevel },
        profile,
    )?;

    set_average_bitrate(session, bitrate_kbps)?;
    if fps_hint > 0 {
        let fps = fps_hint.min(240) as i32;
        let _ = set_property_i32(
            session,
            unsafe { kVTCompressionPropertyKey_ExpectedFrameRate },
            fps,
        );
        let _ = set_property_i32(
            session,
            unsafe { kVTCompressionPropertyKey_MaximumRealTimeFrameRate },
            fps,
        );
    }

    let gop = keyframe_interval.max(1) as i32;
    let gop_num = CFNumber::new_i32(gop);
    set_property(
        session,
        unsafe { kVTCompressionPropertyKey_MaxKeyFrameInterval },
        gop_num.as_ref(),
    )?;

    Ok(())
}

fn set_average_bitrate(session: &VTCompressionSession, bitrate_kbps: u32) -> anyhow::Result<()> {
    use objc2_core_foundation::CFNumber;
    let bitrate_bps = (bitrate_kbps.saturating_mul(1000)) as i32;
    let bitrate = CFNumber::new_i32(bitrate_bps);
    set_property(
        session,
        unsafe { kVTCompressionPropertyKey_AverageBitRate },
        bitrate.as_ref(),
    )
}

fn try_reserve_in_flight(in_flight: &AtomicUsize, max_in_flight_frames: usize) -> bool {
    let max_in_flight_frames = max_in_flight_frames.max(1);
    let mut current = in_flight.load(Ordering::Acquire);
    while current < max_in_flight_frames {
        match in_flight.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(next) => current = next,
        }
    }
    false
}

fn release_in_flight(in_flight: &AtomicUsize) {
    let mut current = in_flight.load(Ordering::Acquire);
    while current > 0 {
        match in_flight.compare_exchange_weak(
            current,
            current - 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return,
            Err(next) => current = next,
        }
    }
}

fn set_property(
    session: &VTCompressionSession,
    key: &objc2_core_foundation::CFString,
    value: &objc2_core_foundation::CFType,
) -> anyhow::Result<()> {
    let vt_session = unsafe { &*(session as *const VTCompressionSession as *const VTSession) };
    let status = unsafe { VTSessionSetProperty(vt_session, key, Some(value)) };
    if status != 0 {
        anyhow::bail!("VTSessionSetProperty failed: OSStatus={status}");
    }
    Ok(())
}

fn set_property_bool(
    session: &VTCompressionSession,
    key: &objc2_core_foundation::CFString,
    value: bool,
) -> anyhow::Result<()> {
    use objc2_core_foundation::{CFBoolean, CFType};
    let b = if value {
        CFBoolean::new(true)
    } else {
        CFBoolean::new(false)
    };
    let v: &CFType = b.as_ref();
    set_property(session, key, v)
}

fn set_property_i32(
    session: &VTCompressionSession,
    key: &objc2_core_foundation::CFString,
    value: i32,
) -> anyhow::Result<()> {
    use objc2_core_foundation::CFNumber;
    let n = CFNumber::new_i32(value);
    set_property(session, key, n.as_ref())
}

fn now_wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// VideoToolbox output callback. Runs on a VideoToolbox thread.
unsafe extern "C-unwind" fn output_callback(
    output_ref_con: *mut c_void,
    _source_frame_ref_con: *mut c_void,
    status: i32,
    _info_flags: VTEncodeInfoFlags,
    sample_buffer: *mut CMSampleBuffer,
) {
    if output_ref_con.is_null() {
        return;
    }
    let ctx = unsafe { &*(output_ref_con as *const CallbackContext) };
    release_in_flight(&ctx.in_flight);
    if status != 0 || sample_buffer.is_null() {
        return;
    }
    let sample = unsafe { &*sample_buffer };

    let pts_ms = cmtime_to_ms(unsafe { sample.presentation_time_stamp() });
    let timing = ctx.pending_timings.take(pts_ms);
    let Some(unit) = (unsafe { access_unit_from_sample(sample, timing) }) else {
        return;
    };
    ctx.queue.push_latest(unit);
    if let Some(wakeup) = &ctx.wakeup {
        wakeup();
    }
}

/// Convert a compressed CMSampleBuffer (AVCC) into an Annex-B access unit,
/// prepending SPS/PPS on keyframes.
unsafe fn access_unit_from_sample(
    sample: &CMSampleBuffer,
    timing: Option<PendingFrameTiming>,
) -> Option<EncodedAccessUnit> {
    let is_keyframe = unsafe { sample_is_keyframe(sample) };
    let pts = unsafe { sample.presentation_time_stamp() };
    let pts_ms = cmtime_to_ms(pts);
    let encoded_at_ms = now_wall_ms();
    let encode_submitted_at_ms = timing
        .map(|timing| timing.submitted_at_ms)
        .unwrap_or(encoded_at_ms);

    let mut annexb = Vec::new();
    if is_keyframe {
        if let Some(param_sets) = unsafe { parameter_sets_annexb(sample) } {
            annexb.extend_from_slice(&param_sets);
        }
    }
    let payload = unsafe { avcc_block_to_annexb(sample)? };
    annexb.extend_from_slice(&payload);
    if annexb.is_empty() {
        return None;
    }
    Some(EncodedAccessUnit {
        annexb,
        is_keyframe,
        pts_ms,
        encode_submitted_at_ms,
        encoded_at_ms,
        encode_latency_ms: encoded_at_ms.saturating_sub(encode_submitted_at_ms),
    })
}

const ANNEXB_START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Rewrite a length-prefixed AVCC block buffer into Annex-B start codes.
/// Returns None if the data buffer cannot be read.
unsafe fn avcc_block_to_annexb(sample: &CMSampleBuffer) -> Option<Vec<u8>> {
    let data = unsafe { sample_block_bytes(sample)? };
    let mut out = Vec::with_capacity(data.len() + 16);
    let mut i = 0usize;
    while i + 4 <= data.len() {
        let nal_len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        i += 4;
        if nal_len == 0 || i + nal_len > data.len() {
            break;
        }
        out.extend_from_slice(&ANNEXB_START_CODE);
        out.extend_from_slice(&data[i..i + nal_len]);
        i += nal_len;
    }
    Some(out)
}

fn invalid_cmtime() -> CMTime {
    CMTime {
        value: 0,
        timescale: 0,
        flags: CMTimeFlags::empty(),
        epoch: 0,
    }
}

/// A compressed sample is a keyframe (IDR / sync sample) unless its
/// attachments dictionary marks it `NotSync = true`. VideoToolbox attaches
/// one dictionary per sample; we inspect index 0.
unsafe fn sample_is_keyframe(sample: &CMSampleBuffer) -> bool {
    use objc2_core_foundation::{CFBoolean, CFDictionary};
    use objc2_core_media::kCMSampleAttachmentKey_NotSync;

    let Some(attachments) = (unsafe { sample.sample_attachments_array(false) }) else {
        // No attachments array => treat as sync sample (conservative: emit
        // SPS/PPS rather than risk an undecodable stream).
        return true;
    };
    if attachments.count() == 0 {
        return true;
    }
    let dict_ptr = unsafe { attachments.value_at_index(0) };
    if dict_ptr.is_null() {
        return true;
    }
    let dict = unsafe { &*(dict_ptr as *const CFDictionary) };
    let key = unsafe { kCMSampleAttachmentKey_NotSync };
    let key_ptr = (key as *const objc2_core_foundation::CFString).cast();
    let value = unsafe { dict.value(key_ptr) };
    if value.is_null() {
        // NotSync key absent => sync sample.
        return true;
    }
    let not_sync = unsafe { &*(value as *const CFBoolean) };
    // Keyframe iff NOT marked NotSync.
    !not_sync.value()
}

unsafe fn parameter_sets_annexb(sample: &CMSampleBuffer) -> Option<Vec<u8>> {
    let format = unsafe { sample.format_description()? };
    let mut count = 0usize;
    let mut header_len = 0 as c_int;
    let status = unsafe {
        CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
            format.as_ref(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut count,
            &mut header_len,
        )
    };
    if status != 0 || count == 0 {
        return None;
    }

    let mut out = Vec::new();
    for index in 0..count {
        let mut ptr: *const u8 = std::ptr::null();
        let mut len = 0usize;
        let status = unsafe {
            CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                format.as_ref(),
                index,
                &mut ptr,
                &mut len,
                std::ptr::null_mut(),
                &mut header_len,
            )
        };
        if status != 0 || ptr.is_null() || len == 0 {
            continue;
        }
        out.extend_from_slice(&ANNEXB_START_CODE);
        out.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr, len) });
    }
    Some(out)
}

unsafe fn sample_block_bytes(sample: &CMSampleBuffer) -> Option<Vec<u8>> {
    let block = unsafe { sample.data_buffer()? };
    let len = unsafe { block.data_length() };
    if len == 0 {
        return None;
    }
    let mut out = vec![0u8; len];
    let dst = NonNull::new(out.as_mut_ptr() as *mut c_void)?;
    let status = unsafe { block.copy_data_bytes(0, len, dst) };
    if status != 0 {
        return None;
    }
    Some(out)
}

fn cmtime_to_ms(t: CMTime) -> u64 {
    if t.timescale <= 0 {
        return 0;
    }
    let secs = t.value as f64 / t.timescale as f64;
    (secs * 1000.0).max(0.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avcc_two_nals_become_annexb() {
        // Two AVCC NALs: lengths 3 and 2.
        let avcc = vec![
            0, 0, 0, 3, 0x67, 0x42, 0x00, // SPS-ish, len 3
            0, 0, 0, 2, 0x68, 0xce, // PPS-ish, len 2
        ];
        let out = rewrite_avcc_for_test(&avcc);
        assert_eq!(
            out,
            vec![
                0, 0, 0, 1, 0x67, 0x42, 0x00, //
                0, 0, 0, 1, 0x68, 0xce,
            ]
        );
    }

    #[test]
    fn avcc_truncated_nal_is_dropped() {
        // Declares len 9 but only 2 bytes follow.
        let avcc = vec![0, 0, 0, 9, 0x41, 0x9a];
        let out = rewrite_avcc_for_test(&avcc);
        assert!(out.is_empty());
    }

    #[test]
    fn cmtime_ms_conversion() {
        let t = CMTime {
            value: 90_000,
            timescale: 90_000,
            flags: objc2_core_media::CMTimeFlags::Valid,
            epoch: 0,
        };
        assert_eq!(cmtime_to_ms(t), 1000);
    }

    #[test]
    fn encoded_queue_drops_old_delta_frames_before_keyframes() {
        let q = EncodedAccessUnitQueue::new(2);
        q.push_latest(unit(1, true));
        q.push_latest(unit(2, false));
        q.push_latest(unit(3, false));

        let drained = q.drain();
        assert_eq!(q.dropped(), 1);
        assert_eq!(
            drained
                .iter()
                .map(|unit| (unit.pts_ms, unit.is_keyframe))
                .collect::<Vec<_>>(),
            vec![(1, true), (3, false)]
        );
    }

    #[test]
    fn pending_timing_take_returns_none_on_miss() {
        let timings = PendingFrameTimings::new(4);
        assert_eq!(timings.push(100, 1_000), 0);
        // A PTS that was never submitted must not borrow another frame's time.
        assert!(timings.take(999).is_none());
        // The real entry is still retrievable and exact.
        let got = timings.take(100).expect("submitted timing present");
        assert_eq!(got.pts_ms, 100);
        assert_eq!(got.submitted_at_ms, 1_000);
    }

    #[test]
    fn pending_timing_push_evicts_stale_entries_on_overflow() {
        let timings = PendingFrameTimings::new(2);
        assert_eq!(timings.push(1, 10), 0);
        assert_eq!(timings.push(2, 20), 0);
        // Third submit over capacity evicts the oldest orphaned entry (pts 1),
        // which never came back from the encoder.
        assert_eq!(timings.push(3, 30), 1);
        assert!(timings.take(1).is_none(), "evicted entry must be gone");
        assert!(timings.take(2).is_some());
        assert!(timings.take(3).is_some());
    }

    #[test]
    fn pending_timing_reclaim_drops_only_entries_past_deadline() {
        let timings = PendingFrameTimings::new(8);
        timings.push(1, 1_000); // stale relative to now=2_500, deadline=1_000
        timings.push(2, 1_400); // stale (2_500-1_400=1_100 >= 1_000)
        timings.push(3, 2_000); // fresh (2_500-2_000=500 < 1_000)
        let reclaimed = timings.reclaim_stale(2_500, 1_000);
        assert_eq!(reclaimed, 2);
        assert!(timings.take(1).is_none());
        assert!(timings.take(2).is_none());
        assert!(timings.take(3).is_some(), "fresh entry must survive");
    }

    #[test]
    fn in_flight_reservation_is_hard_bounded() {
        let in_flight = AtomicUsize::new(0);
        assert!(try_reserve_in_flight(&in_flight, 1));
        assert!(!try_reserve_in_flight(&in_flight, 1));
        release_in_flight(&in_flight);
        assert!(try_reserve_in_flight(&in_flight, 2));
        assert!(try_reserve_in_flight(&in_flight, 2));
        assert!(!try_reserve_in_flight(&in_flight, 2));
    }

    // Test-only mirror of the inner AVCC->Annex-B rewrite loop, so the
    // byte-stream conversion is verifiable without a live CMSampleBuffer.
    fn rewrite_avcc_for_test(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + 4 <= data.len() {
            let nal_len =
                u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
            i += 4;
            if nal_len == 0 || i + nal_len > data.len() {
                break;
            }
            out.extend_from_slice(&ANNEXB_START_CODE);
            out.extend_from_slice(&data[i..i + nal_len]);
            i += nal_len;
        }
        out
    }

    fn unit(pts_ms: u64, is_keyframe: bool) -> EncodedAccessUnit {
        EncodedAccessUnit {
            annexb: vec![pts_ms as u8],
            is_keyframe,
            pts_ms,
            encode_submitted_at_ms: pts_ms,
            encoded_at_ms: pts_ms,
            encode_latency_ms: 0,
        }
    }
}

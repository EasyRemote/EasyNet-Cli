//! Private binary contract for one immutable RemoteApp media-host generation.
//!
//! Protocol responsibility:
//! - bind a killable native media process to one build, transport and source;
//! - correlate every host event with an exact daemon command;
//! - validate raw H264/Opus before it reaches daemon-owned WebRTC writers.
//!
//! Runtime authority, URAs, consent, session/rebind state, WebRTC/RTP,
//! receipts and adaptation policy never cross this boundary.

use std::collections::BTreeSet;
use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{FrameError, ValidationError};

pub const SCHEMA_VERSION: u32 = 1;
pub const PROTOCOL: &str = "remoteapp_media_host_v1";
pub const VIDEO_LANE_FD_ENV: &str = "EASYNET_REMOTEAPP_MEDIA_VIDEO_FD";
pub const AUDIO_LANE_FD_ENV: &str = "EASYNET_REMOTEAPP_MEDIA_AUDIO_FD";
pub const MAX_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_OPUS_PACKET_BYTES: usize = 1_275;
pub const MAX_APPLICATION_WINDOWS: usize = 32;
pub const MAX_CAPTURE_PIXELS: u64 = 33_177_600; // 7680 x 4320
const MAX_STRING_BYTES: usize = 2_048;
const MAX_SURFACE_COORDINATE: i64 = 10_000_000;
const MAX_KEYFRAME_RECOVERY_SECONDS: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationFence {
    pub process_generation: u64,
    /// Exact release/build identity supplied by the daemon and compiled host.
    pub build_id: String,
    /// 128-bit daemon-generated nonce encoded as lowercase hexadecimal.
    pub session_nonce: String,
    pub transport_epoch: u64,
    pub media_source_epoch: u64,
    /// SHA-256 of the exact `StartContract` canonical JSON representation.
    pub contract_digest: String,
}

impl GenerationFence {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.process_generation == 0
            || self.transport_epoch == 0
            || self.media_source_epoch == 0
            || !valid_hex_exact(&self.build_id, 64)
            || !valid_hex_exact(&self.session_nonce, 32)
            || !valid_hex_exact(&self.contract_digest, 64)
        {
            return Err(ValidationError::new("invalid media generation fence"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Display,
    Window,
    Application,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationSurface {
    pub window_id: u64,
    pub x: i64,
    pub y: i64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationWindowSet {
    pub display_id: Option<u64>,
    /// Sorted, unique non-zero display membership.
    pub display_ids: Vec<u64>,
    pub primary_pid: i64,
    pub process_instance_id: Option<String>,
    pub app_identity: Option<String>,
    pub bundle_id: Option<String>,
    /// Sorted, unique non-zero window membership.
    pub window_ids: Vec<u64>,
    pub window_set_epoch: u64,
    /// Front-to-back native surface order. This is intentionally not sorted.
    pub front_to_back_surfaces: Vec<ApplicationSurface>,
    pub surface_layout_epoch: u64,
}

impl ApplicationWindowSet {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_positive_optional_id(self.display_id, "application display")?;
        validate_sorted_ids(
            &self.display_ids,
            MAX_APPLICATION_WINDOWS,
            "application displays",
        )?;
        validate_positive_pid(self.primary_pid, "application primary pid")?;
        validate_optional_string(&self.process_instance_id, "process instance")?;
        validate_optional_string(&self.app_identity, "application identity")?;
        validate_optional_string(&self.bundle_id, "bundle identity")?;
        if self.app_identity.is_none() && self.bundle_id.is_none() {
            return Err(ValidationError::new(
                "application target requires app_identity or bundle_id",
            ));
        }
        validate_sorted_ids(
            &self.window_ids,
            MAX_APPLICATION_WINDOWS,
            "application windows",
        )?;
        if self.window_ids.is_empty()
            || self.window_set_epoch == 0
            || self.surface_layout_epoch == 0
            || self.front_to_back_surfaces.is_empty()
            || self.front_to_back_surfaces.len() != self.window_ids.len()
        {
            return Err(ValidationError::new(
                "application target requires exact window-set and surface-layout proofs",
            ));
        }

        let mut surface_ids = BTreeSet::new();
        let mut min_x = i128::MAX;
        let mut min_y = i128::MAX;
        let mut max_x = i128::MIN;
        let mut max_y = i128::MIN;
        let mut total_pixels = 0_u64;
        for surface in &self.front_to_back_surfaces {
            if surface.window_id == 0
                || !surface_ids.insert(surface.window_id)
                || surface.width == 0
                || surface.height == 0
                || surface.x.unsigned_abs() > MAX_SURFACE_COORDINATE as u64
                || surface.y.unsigned_abs() > MAX_SURFACE_COORDINATE as u64
            {
                return Err(ValidationError::new("invalid application surface"));
            }
            let pixels = u64::from(surface.width)
                .checked_mul(u64::from(surface.height))
                .ok_or_else(|| ValidationError::new("application surface size overflow"))?;
            total_pixels = total_pixels
                .checked_add(pixels)
                .ok_or_else(|| ValidationError::new("application surface budget overflow"))?;
            let x = i128::from(surface.x);
            let y = i128::from(surface.y);
            let right = x
                .checked_add(i128::from(surface.width))
                .ok_or_else(|| ValidationError::new("application surface x overflow"))?;
            let bottom = y
                .checked_add(i128::from(surface.height))
                .ok_or_else(|| ValidationError::new("application surface y overflow"))?;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(right);
            max_y = max_y.max(bottom);
        }
        let expected_ids = self.window_ids.iter().copied().collect::<BTreeSet<_>>();
        if surface_ids != expected_ids {
            return Err(ValidationError::new(
                "application surface membership differs from committed window set",
            ));
        }
        let union_width = max_x
            .checked_sub(min_x)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| ValidationError::new("application union width overflow"))?;
        let union_height = max_y
            .checked_sub(min_y)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| ValidationError::new("application union height overflow"))?;
        let union_pixels = union_width
            .checked_mul(union_height)
            .ok_or_else(|| ValidationError::new("application union pixel overflow"))?;
        if total_pixels > MAX_CAPTURE_PIXELS || union_pixels > MAX_CAPTURE_PIXELS {
            return Err(ValidationError::new(
                "application surface layout exceeds media pixel budget",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTargetPlan {
    pub kind: TargetKind,
    pub display_id: Option<u64>,
    pub window_id: Option<u64>,
    pub pid: Option<i64>,
    pub process_instance_id: Option<String>,
    pub app_identity: Option<String>,
    pub bundle_id: Option<String>,
    pub application: Option<ApplicationWindowSet>,
}

impl NativeTargetPlan {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_positive_optional_id(self.display_id, "display")?;
        validate_positive_optional_id(self.window_id, "window")?;
        if let Some(pid) = self.pid {
            validate_positive_pid(pid, "target pid")?;
        }
        validate_optional_string(&self.process_instance_id, "process instance")?;
        validate_optional_string(&self.app_identity, "application identity")?;
        validate_optional_string(&self.bundle_id, "bundle identity")?;

        match self.kind {
            TargetKind::Display => {
                if self.display_id.is_none()
                    || self.window_id.is_some()
                    || self.pid.is_some()
                    || self.process_instance_id.is_some()
                    || self.app_identity.is_some()
                    || self.bundle_id.is_some()
                    || self.application.is_some()
                {
                    return Err(ValidationError::new(
                        "display target must contain only an exact display locator",
                    ));
                }
            }
            TargetKind::Window => {
                if self.window_id.is_none() || self.pid.is_none() || self.application.is_some() {
                    return Err(ValidationError::new(
                        "window target requires exact window and owner process locators",
                    ));
                }
            }
            TargetKind::Application => {
                let application = self.application.as_ref().ok_or_else(|| {
                    ValidationError::new("application target requires application proof")
                })?;
                application.validate()?;
                if self.window_id.is_some()
                    || self.pid != Some(application.primary_pid)
                    || self.process_instance_id != application.process_instance_id
                    || self.app_identity != application.app_identity
                    || self.bundle_id != application.bundle_id
                    || (self.display_id.is_some() && self.display_id != application.display_id)
                {
                    return Err(ValidationError::new(
                        "application locator and application proof disagree",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    H264AnnexB,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoConfig {
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub keyframe_interval_frames: u32,
    pub max_pending_frames: u32,
    pub max_access_unit_bytes: u32,
    /// Maximum encoded H.264 NAL size, excluding its Annex-B start code.
    ///
    /// The daemon derives this from its packet transport. The media host must
    /// configure the encoder to honor it so normal RTP packetization can retain
    /// mapped `Bytes` slices instead of copying large NALs into FU-A fragments.
    pub max_nal_unit_bytes: u32,
    pub h264_profile_idc: u8,
    pub h264_level_idc: u8,
}

impl VideoConfig {
    fn validate(&self) -> Result<(), ValidationError> {
        let pixels = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .ok_or_else(|| ValidationError::new("video dimensions overflow"))?;
        if self.width == 0
            || self.height == 0
            || self.width % 2 != 0
            || self.height % 2 != 0
            || pixels > MAX_CAPTURE_PIXELS
            || !(1..=240).contains(&self.fps)
            || self.bitrate_kbps == 0
            || self.bitrate_kbps > 200_000
            || self.keyframe_interval_frames == 0
            || self.keyframe_interval_frames
                > self.fps.saturating_mul(MAX_KEYFRAME_RECOVERY_SECONDS)
            || self.max_pending_frames == 0
            || self.max_pending_frames > 3
            || self.max_access_unit_bytes == 0
            || self.max_access_unit_bytes as usize > MAX_PAYLOAD_BYTES
            || self.max_nal_unit_bytes < 256
            || self.max_nal_unit_bytes > self.max_access_unit_bytes
            || self.h264_profile_idc != 66
            || self.h264_level_idc == 0
        {
            return Err(ValidationError::new("invalid media video configuration"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    Opus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioConfig {
    pub codec: AudioCodec,
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub frame_duration_ms: u32,
    pub max_pending_packets: u32,
}

impl AudioConfig {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.sample_rate_hz != 48_000
            || self.channels != 2
            || self.frame_duration_ms != 20
            || self.max_pending_packets == 0
            || self.max_pending_packets > 4
        {
            return Err(ValidationError::new("invalid media audio configuration"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartContract {
    pub target: NativeTargetPlan,
    pub video: VideoConfig,
    pub audio: Option<AudioConfig>,
}

impl StartContract {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.target.validate()?;
        self.video.validate()?;
        if let Some(audio) = &self.audio {
            audio.validate()?;
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, ValidationError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| ValidationError::new(format!("encode media contract: {error}")))?;
        Ok(lower_hex(&Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandBody {
    StartPrepared {
        contract: StartContract,
    },
    Activate,
    BeginMedia {
        activation_command_sequence: u64,
    },
    Reconfigure {
        video: VideoConfig,
        force_keyframe: bool,
    },
    ResumeMedia {
        reconfigure_command_sequence: u64,
    },
    RequestKeyframe,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub schema_version: u32,
    pub protocol: String,
    pub fence: GenerationFence,
    pub sequence: u64,
    pub body: CommandBody,
}

/// The legal first frames accepted by the canonical media host. One-shot
/// control and active-session modes share one executable and protocol
/// identity, but never share one process lifetime.
#[derive(Debug, Clone)]
pub enum InitialFrame {
    CaptureProbe(crate::capture_probe::Request),
    Capability(crate::media_capabilities::Request),
    ScreenCapturePermission(crate::screen_capture_permission::Request),
    Session(Command),
}

impl Command {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_envelope(
            self.schema_version,
            &self.protocol,
            &self.fence,
            self.sequence,
        )?;
        match &self.body {
            CommandBody::StartPrepared { contract } => {
                if contract.digest()? != self.fence.contract_digest {
                    return Err(ValidationError::new(
                        "media start contract digest does not match generation fence",
                    ));
                }
            }
            CommandBody::Reconfigure { video, .. } => video.validate()?,
            CommandBody::BeginMedia {
                activation_command_sequence,
            } if *activation_command_sequence == 0 => {
                return Err(ValidationError::new("invalid activation barrier"));
            }
            CommandBody::ResumeMedia {
                reconfigure_command_sequence,
            } if *reconfigure_command_sequence == 0 => {
                return Err(ValidationError::new("invalid reconfigure barrier"));
            }
            CommandBody::Activate
            | CommandBody::BeginMedia { .. }
            | CommandBody::ResumeMedia { .. }
            | CommandBody::RequestKeyframe
            | CommandBody::Stop => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBackend {
    ScreenCaptureKit,
    WindowsGraphicsCapture,
    Dxgi,
    XcapX11,
    PortalPipeWire,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureProof {
    pub backend: CaptureBackend,
    pub observed_target: NativeTargetPlan,
    pub native_width: u32,
    pub native_height: u32,
    pub verified_at_ms: u64,
}

impl CaptureProof {
    pub fn validate_for(&self, target: &NativeTargetPlan) -> Result<(), ValidationError> {
        self.observed_target.validate()?;
        let pixels = u64::from(self.native_width)
            .checked_mul(u64::from(self.native_height))
            .ok_or_else(|| ValidationError::new("capture proof dimensions overflow"))?;
        if &self.observed_target != target
            || self.native_width == 0
            || self.native_height == 0
            || pixels > MAX_CAPTURE_PIXELS
            || self.verified_at_ms == 0
        {
            return Err(ValidationError::new(
                "capture proof does not match exact media target contract",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    PermissionDenied,
    PermissionRevoked,
    TargetInvalidated,
    CaptureUnavailable,
    EncoderUnavailable,
    AudioUnavailable,
    DeviceLost,
    ProtocolViolation,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaStats {
    pub capture_frames: u64,
    pub encoded_video_frames: u64,
    pub encoded_audio_packets: u64,
    pub raw_video_frames_dropped: u64,
    pub encoded_video_frames_dropped: u64,
    pub audio_packets_dropped: u64,
    pub video_queue_depth: u32,
    pub audio_queue_depth: u32,
    pub video_bytes: u64,
    pub audio_bytes: u64,
}

impl MediaStats {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.video_queue_depth > 3 || self.audio_queue_depth > 4 {
            return Err(ValidationError::new("media stats exceed queue contract"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventBody {
    Prepared {
        command_sequence: u64,
        capture_proof: CaptureProof,
    },
    Activated {
        command_sequence: u64,
    },
    VideoH264 {
        media_gate: u32,
        pts_90khz: u64,
        duration_90khz: u32,
        keyframe: bool,
        sps_pps_present: bool,
        discontinuity: bool,
        codec_generation: u32,
        width: u32,
        height: u32,
        encode_submitted_at_ms: u64,
        encoded_at_ms: u64,
    },
    AudioOpus {
        media_gate: u32,
        pts_48khz: u64,
        duration_samples: u16,
        discontinuity: bool,
        sample_rate_hz: u32,
        channels: u8,
    },
    Reconfigured {
        command_sequence: u64,
        video: VideoConfig,
        codec_generation: u32,
    },
    KeyframeRequested {
        command_sequence: u64,
    },
    Stats {
        stats: MediaStats,
    },
    Failed {
        reason: FailureReason,
        detail: String,
    },
    Stopped {
        command_sequence: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaLane {
    Control,
    Video,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventMetadata {
    pub schema_version: u32,
    pub protocol: String,
    pub fence: GenerationFence,
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub body: EventBody,
}

impl EventMetadata {
    pub fn required_lane(&self) -> MediaLane {
        required_lane_for_body(&self.body)
    }

    pub(crate) fn validate_shape(
        &self,
        physical_lane: MediaLane,
        payload: &[u8],
    ) -> Result<(), ValidationError> {
        validate_envelope(
            self.schema_version,
            &self.protocol,
            &self.fence,
            self.sequence,
        )?;
        validate_event_shape(
            physical_lane,
            self.sequence,
            self.observed_at_ms,
            &self.body,
            payload,
        )
    }
}

/// Allocation-free metadata decoded from one fixed `RVID`/`RAUD` header.
///
/// The generation fence is validated against the header nonce before this
/// value is constructed, so immutable generation strings are not cloned for
/// every media frame.
#[derive(Debug, Clone, PartialEq)]
pub struct BinaryMediaEvent {
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub body: EventBody,
}

impl BinaryMediaEvent {
    pub fn required_lane(&self) -> MediaLane {
        required_lane_for_body(&self.body)
    }

    pub fn validate_shape(
        &self,
        physical_lane: MediaLane,
        payload: &[u8],
    ) -> Result<(), ValidationError> {
        validate_event_shape(
            physical_lane,
            self.sequence,
            self.observed_at_ms,
            &self.body,
            payload,
        )
    }
}

fn required_lane_for_body(body: &EventBody) -> MediaLane {
    match body {
        EventBody::VideoH264 { .. } => MediaLane::Video,
        EventBody::AudioOpus { .. } => MediaLane::Audio,
        _ => MediaLane::Control,
    }
}

pub(crate) fn validate_event_shape(
    physical_lane: MediaLane,
    sequence: u64,
    observed_at_ms: u64,
    body: &EventBody,
    payload: &[u8],
) -> Result<(), ValidationError> {
    if sequence == 0 || required_lane_for_body(body) != physical_lane || observed_at_ms == 0 {
        return Err(ValidationError::new(
            "media event arrived on the wrong physical lane",
        ));
    }
    let requires_payload = match body {
        EventBody::Prepared {
            command_sequence,
            capture_proof,
        } => {
            if *command_sequence == 0 {
                return Err(ValidationError::new(
                    "prepared event has no command sequence",
                ));
            }
            capture_proof.observed_target.validate()?;
            false
        }
        EventBody::Activated { command_sequence }
        | EventBody::KeyframeRequested { command_sequence }
        | EventBody::Stopped { command_sequence } => {
            if *command_sequence == 0 {
                return Err(ValidationError::new(
                    "control event has no command sequence",
                ));
            }
            false
        }
        EventBody::VideoH264 {
            media_gate,
            duration_90khz,
            codec_generation,
            width,
            height,
            encode_submitted_at_ms,
            encoded_at_ms,
            ..
        } => {
            if *media_gate == 0
                || *duration_90khz == 0
                || *codec_generation == 0
                || *width == 0
                || *height == 0
                || *encode_submitted_at_ms == 0
                || *encoded_at_ms < *encode_submitted_at_ms
                || *encoded_at_ms > observed_at_ms
            {
                return Err(ValidationError::new("invalid encoded video event"));
            }
            true
        }
        EventBody::AudioOpus {
            media_gate,
            duration_samples,
            sample_rate_hz,
            channels,
            ..
        } => {
            if *media_gate == 0
                || *duration_samples != 960
                || *sample_rate_hz != 48_000
                || *channels != 2
                || payload.len() > MAX_OPUS_PACKET_BYTES
            {
                return Err(ValidationError::new("invalid encoded audio event"));
            }
            true
        }
        EventBody::Reconfigured {
            command_sequence,
            video,
            codec_generation,
        } => {
            if *command_sequence == 0 || *codec_generation == 0 {
                return Err(ValidationError::new("invalid reconfiguration event"));
            }
            video.validate()?;
            false
        }
        EventBody::Stats { stats } => {
            stats.validate()?;
            false
        }
        EventBody::Failed { detail, .. } => {
            if !valid_string(detail) {
                return Err(ValidationError::new("invalid media failure detail"));
            }
            false
        }
    };
    if requires_payload != !payload.is_empty() || payload.len() > MAX_PAYLOAD_BYTES {
        return Err(ValidationError::new(
            "media event payload does not match event kind",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HostLifecycle {
    AwaitingStart,
    Preparing { command_sequence: u64 },
    Prepared,
    Activating { command_sequence: u64 },
    Activated { command_sequence: u64 },
    Active,
    Reconfiguring { command_sequence: u64 },
    Reconfigured { command_sequence: u64 },
    Stopping { command_sequence: u64 },
    Terminal,
    Poisoned,
}

/// Host-side command state machine. Native capture code must complete each
/// fallible transition through the corresponding `mark_*` method before it can
/// accept the next daemon command or emit its acknowledgement.
#[derive(Debug)]
pub struct MediaHostCommandValidator {
    fence: GenerationFence,
    lifecycle: HostLifecycle,
    command_sequence: u64,
    contract: Option<StartContract>,
    media_gate: u32,
}

impl MediaHostCommandValidator {
    pub fn new(fence: GenerationFence) -> Result<Self, ValidationError> {
        fence.validate()?;
        Ok(Self {
            fence,
            lifecycle: HostLifecycle::AwaitingStart,
            command_sequence: 0,
            contract: None,
            media_gate: 0,
        })
    }

    pub fn observe(&mut self, command: &Command) -> Result<(), ValidationError> {
        if self.lifecycle == HostLifecycle::Poisoned {
            return Err(ValidationError::new("media host command state is poisoned"));
        }
        let result = self.observe_inner(command);
        if result.is_err() {
            self.lifecycle = HostLifecycle::Poisoned;
        }
        result
    }

    fn observe_inner(&mut self, command: &Command) -> Result<(), ValidationError> {
        command.validate()?;
        if command.fence != self.fence || command.sequence != self.command_sequence + 1 {
            return Err(ValidationError::new(
                "stale or non-contiguous media host command",
            ));
        }
        match &command.body {
            CommandBody::StartPrepared { contract }
                if self.lifecycle == HostLifecycle::AwaitingStart =>
            {
                self.contract = Some(contract.clone());
                self.lifecycle = HostLifecycle::Preparing {
                    command_sequence: command.sequence,
                };
            }
            CommandBody::Activate if self.lifecycle == HostLifecycle::Prepared => {
                self.lifecycle = HostLifecycle::Activating {
                    command_sequence: command.sequence,
                };
            }
            CommandBody::BeginMedia {
                activation_command_sequence,
            } if self.lifecycle
                == (HostLifecycle::Activated {
                    command_sequence: *activation_command_sequence,
                }) =>
            {
                self.media_gate = self
                    .media_gate
                    .checked_add(1)
                    .ok_or_else(|| ValidationError::new("media host gate overflow"))?;
                self.lifecycle = HostLifecycle::Active;
            }
            CommandBody::Reconfigure { .. } if self.lifecycle == HostLifecycle::Active => {
                self.lifecycle = HostLifecycle::Reconfiguring {
                    command_sequence: command.sequence,
                };
            }
            CommandBody::ResumeMedia {
                reconfigure_command_sequence,
            } if self.lifecycle
                == (HostLifecycle::Reconfigured {
                    command_sequence: *reconfigure_command_sequence,
                }) =>
            {
                self.media_gate = self
                    .media_gate
                    .checked_add(1)
                    .ok_or_else(|| ValidationError::new("media host gate overflow"))?;
                self.lifecycle = HostLifecycle::Active;
            }
            CommandBody::RequestKeyframe if self.lifecycle == HostLifecycle::Active => {}
            CommandBody::Stop
                if !matches!(
                    self.lifecycle,
                    HostLifecycle::AwaitingStart
                        | HostLifecycle::Stopping { .. }
                        | HostLifecycle::Terminal
                ) =>
            {
                self.lifecycle = HostLifecycle::Stopping {
                    command_sequence: command.sequence,
                };
            }
            _ => {
                return Err(ValidationError::new(
                    "invalid media host command transition",
                ))
            }
        }
        self.command_sequence = command.sequence;
        Ok(())
    }

    pub fn mark_prepared(&mut self, command_sequence: u64) -> Result<(), ValidationError> {
        if self.lifecycle != (HostLifecycle::Preparing { command_sequence }) {
            self.lifecycle = HostLifecycle::Poisoned;
            return Err(ValidationError::new(
                "media host prepare completion mismatch",
            ));
        }
        self.lifecycle = HostLifecycle::Prepared;
        Ok(())
    }

    pub fn mark_activated(&mut self, command_sequence: u64) -> Result<(), ValidationError> {
        if self.lifecycle != (HostLifecycle::Activating { command_sequence }) {
            self.lifecycle = HostLifecycle::Poisoned;
            return Err(ValidationError::new(
                "media host activation completion mismatch",
            ));
        }
        self.lifecycle = HostLifecycle::Activated { command_sequence };
        Ok(())
    }

    pub fn mark_reconfigured(&mut self, command_sequence: u64) -> Result<(), ValidationError> {
        if self.lifecycle != (HostLifecycle::Reconfiguring { command_sequence }) {
            self.lifecycle = HostLifecycle::Poisoned;
            return Err(ValidationError::new(
                "media host reconfiguration completion mismatch",
            ));
        }
        self.lifecycle = HostLifecycle::Reconfigured { command_sequence };
        Ok(())
    }

    pub fn mark_stopped(&mut self, command_sequence: u64) -> Result<(), ValidationError> {
        if self.lifecycle != (HostLifecycle::Stopping { command_sequence }) {
            self.lifecycle = HostLifecycle::Poisoned;
            return Err(ValidationError::new("media host stop completion mismatch"));
        }
        self.lifecycle = HostLifecycle::Terminal;
        Ok(())
    }

    /// Terminate a generation after a native operation fails. This is legal
    /// only after the start command has established the immutable fence.
    pub fn mark_failed(&mut self) -> Result<(), ValidationError> {
        if matches!(
            self.lifecycle,
            HostLifecycle::AwaitingStart | HostLifecycle::Terminal | HostLifecycle::Poisoned
        ) {
            self.lifecycle = HostLifecycle::Poisoned;
            return Err(ValidationError::new(
                "media host failure occurred outside an established generation",
            ));
        }
        self.lifecycle = HostLifecycle::Terminal;
        Ok(())
    }

    pub fn contract(&self) -> Option<&StartContract> {
        self.contract.as_ref()
    }

    pub fn media_gate(&self) -> u32 {
        self.media_gate
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConversationLifecycle {
    AwaitingStart,
    Preparing { command_sequence: u64 },
    Prepared,
    Activating { command_sequence: u64 },
    Activated,
    Active,
    Stopping { command_sequence: u64 },
    Terminal,
    Poisoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingReconfigure {
    command_sequence: u64,
    video: VideoConfig,
}

/// Daemon-side validator for the complete command/event conversation.
///
/// Any protocol error poisons the generation. Callers must then close all
/// lanes and retire the process; a malformed byte stream is never resynced.
#[derive(Debug)]
pub struct MediaConversationValidator {
    fence: GenerationFence,
    lifecycle: ConversationLifecycle,
    contract: Option<StartContract>,
    command_sequence: u64,
    control_sequence: u64,
    video_sequence: u64,
    audio_sequence: u64,
    control_observed_at_ms: u64,
    video_observed_at_ms: u64,
    audio_observed_at_ms: u64,
    video_pts: Option<u64>,
    audio_pts: Option<u64>,
    codec_generation: u32,
    media_gate: u32,
    media_suspended: bool,
    awaiting_idr: bool,
    dropping_until_requested_idr: bool,
    activation_command_sequence: Option<u64>,
    reconfigure_command_sequence: Option<u64>,
    pending_reconfigure: Option<PendingReconfigure>,
    pending_keyframe_command: Option<u64>,
}

impl MediaConversationValidator {
    pub fn new(fence: GenerationFence) -> Result<Self, ValidationError> {
        fence.validate()?;
        Ok(Self {
            fence,
            lifecycle: ConversationLifecycle::AwaitingStart,
            contract: None,
            command_sequence: 0,
            control_sequence: 0,
            video_sequence: 0,
            audio_sequence: 0,
            control_observed_at_ms: 0,
            video_observed_at_ms: 0,
            audio_observed_at_ms: 0,
            video_pts: None,
            audio_pts: None,
            codec_generation: 1,
            media_gate: 0,
            media_suspended: true,
            awaiting_idr: true,
            dropping_until_requested_idr: false,
            activation_command_sequence: None,
            reconfigure_command_sequence: None,
            pending_reconfigure: None,
            pending_keyframe_command: None,
        })
    }

    pub fn register_command(&mut self, command: &Command) -> Result<(), ValidationError> {
        if self.lifecycle == ConversationLifecycle::Poisoned {
            return Err(ValidationError::new("media generation is poisoned"));
        }
        let result = self.register_command_inner(command);
        if result.is_err() {
            self.lifecycle = ConversationLifecycle::Poisoned;
        }
        result
    }

    fn register_command_inner(&mut self, command: &Command) -> Result<(), ValidationError> {
        command.validate()?;
        if command.fence != self.fence || command.sequence != self.command_sequence + 1 {
            return Err(ValidationError::new(
                "stale or non-contiguous media command",
            ));
        }
        match &command.body {
            CommandBody::StartPrepared { contract }
                if self.lifecycle == ConversationLifecycle::AwaitingStart =>
            {
                self.contract = Some(contract.clone());
                self.lifecycle = ConversationLifecycle::Preparing {
                    command_sequence: command.sequence,
                };
            }
            CommandBody::Activate if self.lifecycle == ConversationLifecycle::Prepared => {
                self.lifecycle = ConversationLifecycle::Activating {
                    command_sequence: command.sequence,
                };
            }
            CommandBody::BeginMedia {
                activation_command_sequence,
            } if self.lifecycle == ConversationLifecycle::Activated
                && self.activation_command_sequence == Some(*activation_command_sequence) =>
            {
                self.media_gate = self
                    .media_gate
                    .checked_add(1)
                    .ok_or_else(|| ValidationError::new("media gate overflow"))?;
                self.media_suspended = false;
                self.awaiting_idr = true;
                self.video_pts = None;
                self.audio_pts = None;
                self.lifecycle = ConversationLifecycle::Active;
            }
            CommandBody::Reconfigure { video, .. }
                if self.lifecycle == ConversationLifecycle::Active
                    && self.pending_reconfigure.is_none() =>
            {
                self.media_suspended = true;
                self.pending_reconfigure = Some(PendingReconfigure {
                    command_sequence: command.sequence,
                    video: video.clone(),
                });
            }
            CommandBody::ResumeMedia {
                reconfigure_command_sequence,
            } if self.lifecycle == ConversationLifecycle::Active
                && self.pending_reconfigure.is_none()
                && self.reconfigure_command_sequence == Some(*reconfigure_command_sequence) =>
            {
                self.media_gate = self
                    .media_gate
                    .checked_add(1)
                    .ok_or_else(|| ValidationError::new("media gate overflow"))?;
                self.media_suspended = false;
                self.awaiting_idr = true;
                self.video_pts = None;
                self.audio_pts = None;
            }
            CommandBody::RequestKeyframe
                if self.lifecycle == ConversationLifecycle::Active
                    && self.pending_keyframe_command.is_none() =>
            {
                self.pending_keyframe_command = Some(command.sequence);
                self.dropping_until_requested_idr = true;
            }
            CommandBody::Stop
                if !matches!(
                    self.lifecycle,
                    ConversationLifecycle::AwaitingStart
                        | ConversationLifecycle::Stopping { .. }
                        | ConversationLifecycle::Terminal
                ) =>
            {
                self.media_suspended = true;
                self.lifecycle = ConversationLifecycle::Stopping {
                    command_sequence: command.sequence,
                };
            }
            _ => return Err(ValidationError::new("invalid media command transition")),
        }
        self.command_sequence = command.sequence;
        Ok(())
    }

    pub fn observe(
        &mut self,
        physical_lane: MediaLane,
        metadata: &EventMetadata,
        payload: &[u8],
    ) -> Result<MediaObservation, ValidationError> {
        if self.lifecycle == ConversationLifecycle::Poisoned {
            return Err(ValidationError::new("media generation is poisoned"));
        }
        let result = (|| {
            metadata.validate_shape(physical_lane, payload)?;
            if metadata.fence != self.fence {
                return Err(ValidationError::new("stale media generation fence"));
            }
            self.observe_validated(
                physical_lane,
                metadata.sequence,
                metadata.observed_at_ms,
                &metadata.body,
                payload,
            )
        })();
        if result.is_err() {
            self.lifecycle = ConversationLifecycle::Poisoned;
        }
        result
    }

    /// Observe a media event whose fixed header was already validated against
    /// this generation's nonce. This keeps immutable envelope strings out of
    /// the video/audio hot path without weakening conversation sequencing.
    pub fn observe_binary_media(
        &mut self,
        physical_lane: MediaLane,
        metadata: &BinaryMediaEvent,
        payload: &[u8],
    ) -> Result<MediaObservation, ValidationError> {
        if self.lifecycle == ConversationLifecycle::Poisoned {
            return Err(ValidationError::new("media generation is poisoned"));
        }
        let result = (|| {
            metadata.validate_shape(physical_lane, payload)?;
            self.observe_validated(
                physical_lane,
                metadata.sequence,
                metadata.observed_at_ms,
                &metadata.body,
                payload,
            )
        })();
        if result.is_err() {
            self.lifecycle = ConversationLifecycle::Poisoned;
        }
        result
    }

    /// Advance one media-lane sequence for a frame that the bounded carrier
    /// intentionally discarded before payload validation could occur.
    ///
    /// Shared-memory notifications retain the generation-bound sequence,
    /// observation time and media gate even when no slot can be leased. This
    /// method prevents an intentional bounded drop from being misclassified as
    /// a protocol sequence gap. It never accepts control-lane drops and never
    /// treats a dropped payload as codec evidence.
    pub fn observe_backpressure_drop(
        &mut self,
        physical_lane: MediaLane,
        sequence: u64,
        observed_at_ms: u64,
        media_gate: u32,
    ) -> Result<MediaObservation, ValidationError> {
        if self.lifecycle == ConversationLifecycle::Poisoned {
            return Err(ValidationError::new("media generation is poisoned"));
        }
        let result = self.observe_backpressure_drop_inner(
            physical_lane,
            sequence,
            observed_at_ms,
            media_gate,
        );
        if result.is_err() {
            self.lifecycle = ConversationLifecycle::Poisoned;
        }
        result
    }

    fn observe_backpressure_drop_inner(
        &mut self,
        physical_lane: MediaLane,
        sequence: u64,
        observed_at_ms: u64,
        media_gate: u32,
    ) -> Result<MediaObservation, ValidationError> {
        let (lane_sequence, lane_observed_at_ms) = match physical_lane {
            MediaLane::Control => {
                return Err(ValidationError::new(
                    "control events cannot be discarded as media backpressure",
                ))
            }
            MediaLane::Video => (&mut self.video_sequence, &mut self.video_observed_at_ms),
            MediaLane::Audio => (&mut self.audio_sequence, &mut self.audio_observed_at_ms),
        };
        if sequence != *lane_sequence + 1
            || observed_at_ms == 0
            || observed_at_ms < *lane_observed_at_ms
        {
            return Err(ValidationError::new(
                "non-contiguous dropped media sequence or regressing observation time",
            ));
        }
        if media_gate < self.media_gate
            || (self.media_suspended && media_gate == self.media_gate && self.media_gate > 0)
        {
            *lane_sequence = sequence;
            *lane_observed_at_ms = observed_at_ms;
            return Ok(MediaObservation::StaleDiscarded);
        }
        if self.lifecycle != ConversationLifecycle::Active
            || self.media_suspended
            || media_gate != self.media_gate
        {
            return Err(ValidationError::new(
                "dropped media arrived outside active media gate",
            ));
        }
        *lane_sequence = sequence;
        *lane_observed_at_ms = observed_at_ms;
        if physical_lane == MediaLane::Video {
            self.awaiting_idr = true;
            self.dropping_until_requested_idr = true;
        }
        Ok(MediaObservation::BackpressureDiscarded)
    }

    fn observe_validated(
        &mut self,
        physical_lane: MediaLane,
        event_sequence: u64,
        event_observed_at_ms: u64,
        body: &EventBody,
        payload: &[u8],
    ) -> Result<MediaObservation, ValidationError> {
        let (sequence, observed_at_ms) = match physical_lane {
            MediaLane::Control => (&mut self.control_sequence, &mut self.control_observed_at_ms),
            MediaLane::Video => (&mut self.video_sequence, &mut self.video_observed_at_ms),
            MediaLane::Audio => (&mut self.audio_sequence, &mut self.audio_observed_at_ms),
        };
        if event_sequence != *sequence + 1 || event_observed_at_ms < *observed_at_ms {
            return Err(ValidationError::new(
                "non-contiguous media sequence or regressing observation time",
            ));
        }

        match body {
            EventBody::Prepared {
                command_sequence,
                capture_proof,
            } => {
                let expected = match self.lifecycle {
                    ConversationLifecycle::Preparing { command_sequence } => command_sequence,
                    _ => return Err(ValidationError::new("unsolicited prepared event")),
                };
                if *command_sequence != expected {
                    return Err(ValidationError::new(
                        "prepared command correlation mismatch",
                    ));
                }
                let contract = self
                    .contract
                    .as_ref()
                    .ok_or_else(|| ValidationError::new("media start contract missing"))?;
                capture_proof.validate_for(&contract.target)?;
                self.lifecycle = ConversationLifecycle::Prepared;
            }
            EventBody::Activated { command_sequence } => {
                let expected = match self.lifecycle {
                    ConversationLifecycle::Activating { command_sequence } => command_sequence,
                    _ => return Err(ValidationError::new("unsolicited activated event")),
                };
                if *command_sequence != expected {
                    return Err(ValidationError::new(
                        "activate command correlation mismatch",
                    ));
                }
                self.activation_command_sequence = Some(*command_sequence);
                self.lifecycle = ConversationLifecycle::Activated;
            }
            EventBody::VideoH264 {
                media_gate,
                pts_90khz,
                keyframe,
                sps_pps_present,
                discontinuity,
                codec_generation,
                width,
                height,
                ..
            } => {
                if *media_gate < self.media_gate
                    || (self.media_suspended
                        && *media_gate == self.media_gate
                        && self.media_gate > 0)
                {
                    *sequence = event_sequence;
                    *observed_at_ms = event_observed_at_ms;
                    return Ok(MediaObservation::StaleDiscarded);
                }
                if self.lifecycle != ConversationLifecycle::Active
                    || self.media_suspended
                    || *media_gate != self.media_gate
                {
                    return Err(ValidationError::new(
                        "video arrived outside active media gate",
                    ));
                }
                let config = &self
                    .contract
                    .as_ref()
                    .ok_or_else(|| ValidationError::new("media start contract missing"))?
                    .video;
                if *width != config.width
                    || *height != config.height
                    || payload.len() > config.max_access_unit_bytes as usize
                    || *codec_generation != self.codec_generation
                    || self
                        .video_pts
                        .is_some_and(|previous| *pts_90khz <= previous)
                {
                    return Err(ValidationError::new(
                        "video does not match negotiated media contract",
                    ));
                }
                let annex_b = inspect_h264_annex_b(payload)?;
                if annex_b.max_nal_unit_bytes > config.max_nal_unit_bytes as usize {
                    return Err(ValidationError::new(
                        "H264 NAL exceeds negotiated packetization bound",
                    ));
                }
                if *keyframe != annex_b.has_idr
                    || *sps_pps_present != (annex_b.has_sps && annex_b.has_pps)
                {
                    return Err(ValidationError::new(
                        "H264 metadata does not match Annex-B access unit",
                    ));
                }
                if let Some(sps) = annex_b.sps {
                    let parsed = parse_baseline_sps(sps)?;
                    if parsed.profile_idc != config.h264_profile_idc
                        || parsed.level_idc != config.h264_level_idc
                        || parsed.width != config.width
                        || parsed.height != config.height
                    {
                        return Err(ValidationError::new(
                            "H264 SPS does not match negotiated profile, level or dimensions",
                        ));
                    }
                }
                let recovery_idr = annex_b.has_idr && annex_b.has_sps && annex_b.has_pps;
                if self.dropping_until_requested_idr && !recovery_idr {
                    self.video_pts = Some(*pts_90khz);
                    *sequence = event_sequence;
                    *observed_at_ms = event_observed_at_ms;
                    return Ok(MediaObservation::StaleDiscarded);
                }
                if *discontinuity {
                    self.awaiting_idr = true;
                }
                if self.awaiting_idr && !recovery_idr {
                    return Err(ValidationError::new(
                        "media generation must resume at an H264 IDR with SPS/PPS",
                    ));
                }
                if recovery_idr {
                    self.awaiting_idr = false;
                    self.dropping_until_requested_idr = false;
                }
                self.video_pts = Some(*pts_90khz);
            }
            EventBody::AudioOpus {
                media_gate,
                pts_48khz,
                discontinuity,
                ..
            } => {
                if *media_gate < self.media_gate
                    || (self.media_suspended
                        && *media_gate == self.media_gate
                        && self.media_gate > 0)
                {
                    *sequence = event_sequence;
                    *observed_at_ms = event_observed_at_ms;
                    return Ok(MediaObservation::StaleDiscarded);
                }
                if self.lifecycle != ConversationLifecycle::Active
                    || self.media_suspended
                    || *media_gate != self.media_gate
                {
                    return Err(ValidationError::new(
                        "audio arrived outside active media gate",
                    ));
                }
                if self
                    .contract
                    .as_ref()
                    .and_then(|contract| contract.audio.as_ref())
                    .is_none()
                    || self
                        .audio_pts
                        .is_some_and(|previous| *pts_48khz <= previous)
                    || opus_packet_samples_48khz(payload)? != 960
                    || (self.audio_pts.is_none() && !*discontinuity)
                {
                    return Err(ValidationError::new(
                        "Opus packet does not match negotiated audio contract",
                    ));
                }
                self.audio_pts = Some(*pts_48khz);
            }
            EventBody::Reconfigured {
                command_sequence,
                video,
                codec_generation,
            } => {
                let pending = self
                    .pending_reconfigure
                    .take()
                    .ok_or_else(|| ValidationError::new("unsolicited reconfigured event"))?;
                if pending.command_sequence != *command_sequence
                    || pending.video != *video
                    || *codec_generation != self.codec_generation + 1
                {
                    return Err(ValidationError::new(
                        "reconfigure command correlation mismatch",
                    ));
                }
                self.codec_generation = *codec_generation;
                self.reconfigure_command_sequence = Some(*command_sequence);
                self.contract
                    .as_mut()
                    .expect("active media contract exists")
                    .video = video.clone();
                self.awaiting_idr = true;
                self.video_pts = None;
            }
            EventBody::KeyframeRequested { command_sequence } => {
                if self.pending_keyframe_command != Some(*command_sequence) {
                    return Err(ValidationError::new(
                        "keyframe command correlation mismatch",
                    ));
                }
                self.pending_keyframe_command = None;
            }
            EventBody::Stats { .. } => {
                if self.lifecycle != ConversationLifecycle::Active {
                    return Err(ValidationError::new(
                        "stats arrived outside active lifecycle",
                    ));
                }
            }
            EventBody::Failed { .. } => {
                if matches!(
                    self.lifecycle,
                    ConversationLifecycle::AwaitingStart | ConversationLifecycle::Terminal
                ) {
                    return Err(ValidationError::new("unsolicited failure event"));
                }
                self.lifecycle = ConversationLifecycle::Terminal;
            }
            EventBody::Stopped { command_sequence } => {
                let expected = match self.lifecycle {
                    ConversationLifecycle::Stopping { command_sequence } => command_sequence,
                    _ => return Err(ValidationError::new("unsolicited stopped event")),
                };
                if *command_sequence != expected {
                    return Err(ValidationError::new("stop command correlation mismatch"));
                }
                self.lifecycle = ConversationLifecycle::Terminal;
            }
        }

        *sequence = event_sequence;
        *observed_at_ms = event_observed_at_ms;
        Ok(MediaObservation::Accepted)
    }

    pub fn finish_control_eof(&mut self) -> Result<(), ValidationError> {
        if self.lifecycle == ConversationLifecycle::Terminal {
            return Ok(());
        }
        self.lifecycle = ConversationLifecycle::Poisoned;
        Err(ValidationError::new(
            "media control lane reached EOF before a terminal event",
        ))
    }

    pub fn is_terminal(&self) -> bool {
        self.lifecycle == ConversationLifecycle::Terminal
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaObservation {
    Accepted,
    /// A valid generation-bound frame from the preceding media gate that was
    /// already in flight when control suspended or replaced that gate.
    StaleDiscarded,
    /// A generation-bound media sequence was intentionally omitted by a
    /// bounded carrier. No codec payload was accepted as evidence.
    BackpressureDiscarded,
}

fn validate_envelope(
    schema_version: u32,
    protocol: &str,
    fence: &GenerationFence,
    sequence: u64,
) -> Result<(), ValidationError> {
    if schema_version != SCHEMA_VERSION || protocol != PROTOCOL || sequence == 0 {
        return Err(ValidationError::new("unsupported media-host envelope"));
    }
    fence.validate()
}

fn valid_string(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_STRING_BYTES && !value.contains('\0')
}

fn validate_optional_string(value: &Option<String>, field: &str) -> Result<(), ValidationError> {
    if value.as_deref().is_some_and(|value| !valid_string(value)) {
        return Err(ValidationError::new(format!("invalid {field}")));
    }
    Ok(())
}

fn validate_positive_optional_id(value: Option<u64>, field: &str) -> Result<(), ValidationError> {
    if value == Some(0) {
        return Err(ValidationError::new(format!("{field} id must be positive")));
    }
    Ok(())
}

fn validate_positive_pid(value: i64, field: &str) -> Result<(), ValidationError> {
    if value <= 0 {
        return Err(ValidationError::new(format!("{field} must be positive")));
    }
    Ok(())
}

fn validate_sorted_ids(values: &[u64], maximum: usize, field: &str) -> Result<(), ValidationError> {
    if values.len() > maximum
        || values.contains(&0)
        || values.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(ValidationError::new(format!(
            "{field} must be sorted, unique and bounded",
        )));
    }
    Ok(())
}

fn valid_hex_exact(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[derive(Debug)]
struct H264Inspection<'a> {
    has_idr: bool,
    has_sps: bool,
    has_pps: bool,
    sps: Option<&'a [u8]>,
    max_nal_unit_bytes: usize,
}

fn inspect_h264_annex_b(payload: &[u8]) -> Result<H264Inspection<'_>, ValidationError> {
    let Some((first_offset, first_prefix_len)) = find_annex_b_start_code(payload, 0) else {
        return Err(ValidationError::new(
            "H264 access unit is not canonical Annex-B",
        ));
    };
    if first_offset != 0 {
        return Err(ValidationError::new(
            "H264 access unit is not canonical Annex-B",
        ));
    }
    let mut inspection = H264Inspection {
        has_idr: false,
        has_sps: false,
        has_pps: false,
        sps: None,
        max_nal_unit_bytes: 0,
    };
    let mut nal_start = first_offset + first_prefix_len;
    loop {
        let next = find_annex_b_start_code(payload, nal_start);
        let nal_end = next.map(|(offset, _)| offset).unwrap_or(payload.len());
        if nal_start >= nal_end {
            return Err(ValidationError::new("H264 Annex-B contains an empty NAL"));
        }
        let nal = &payload[nal_start..nal_end];
        inspection.max_nal_unit_bytes = inspection.max_nal_unit_bytes.max(nal.len());
        match nal[0] & 0x1f {
            5 => inspection.has_idr = true,
            7 => {
                inspection.has_sps = true;
                inspection.sps = Some(nal);
            }
            8 => inspection.has_pps = true,
            _ => {}
        }
        let Some((next_offset, next_prefix_len)) = next else {
            break;
        };
        nal_start = next_offset + next_prefix_len;
    }
    Ok(inspection)
}

/// Find the next Annex-B start code without building a per-access-unit index.
///
/// This validator runs for every admitted video frame. Returning one scalar
/// cursor at a time keeps the shared-slot-to-WebRTC hot path allocation-free.
fn find_annex_b_start_code(payload: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut index = from;
    while index + 3 <= payload.len() {
        if payload[index..].starts_with(&[0, 0, 0, 1]) {
            return Some((index, 4));
        }
        if payload[index..].starts_with(&[0, 0, 1]) {
            return Some((index, 3));
        }
        index += 1;
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct ParsedSps {
    profile_idc: u8,
    level_idc: u8,
    width: u32,
    height: u32,
}

fn parse_baseline_sps(nal: &[u8]) -> Result<ParsedSps, ValidationError> {
    if nal.len() < 5 || nal[0] & 0x1f != 7 {
        return Err(ValidationError::new("invalid H264 SPS NAL"));
    }
    let rbsp = remove_emulation_prevention(&nal[1..]);
    if rbsp.len() < 4 || rbsp[0] != 66 {
        return Err(ValidationError::new(
            "RemoteApp media-host v1 requires H264 Baseline SPS",
        ));
    }
    let profile_idc = rbsp[0];
    let level_idc = rbsp[2];
    let mut bits = BitReader::new(&rbsp[3..]);
    bits.read_ue()?; // seq_parameter_set_id
    bits.read_ue()?; // log2_max_frame_num_minus4
    let pic_order_cnt_type = bits.read_ue()?;
    match pic_order_cnt_type {
        0 => {
            bits.read_ue()?;
        }
        1 => {
            bits.read_bit()?;
            bits.read_se()?;
            bits.read_se()?;
            let cycle = bits.read_ue()?;
            if cycle > 256 {
                return Err(ValidationError::new("H264 SPS POC cycle is oversized"));
            }
            for _ in 0..cycle {
                bits.read_se()?;
            }
        }
        2 => {}
        _ => return Err(ValidationError::new("unsupported H264 SPS POC type")),
    }
    bits.read_ue()?; // max_num_ref_frames
    bits.read_bit()?; // gaps_in_frame_num_value_allowed_flag
    let width_in_mbs = bits
        .read_ue()?
        .checked_add(1)
        .ok_or_else(|| ValidationError::new("H264 SPS width overflow"))?;
    let height_in_map_units = bits
        .read_ue()?
        .checked_add(1)
        .ok_or_else(|| ValidationError::new("H264 SPS height overflow"))?;
    let frame_mbs_only = bits.read_bit()?;
    if frame_mbs_only == 0 {
        bits.read_bit()?;
    }
    bits.read_bit()?; // direct_8x8_inference_flag
    let frame_cropping = bits.read_bit()?;
    let (crop_left, crop_right, crop_top, crop_bottom) = if frame_cropping != 0 {
        (
            bits.read_ue()?,
            bits.read_ue()?,
            bits.read_ue()?,
            bits.read_ue()?,
        )
    } else {
        (0, 0, 0, 0)
    };
    let width = width_in_mbs
        .checked_mul(16)
        .and_then(|value| value.checked_sub((crop_left + crop_right).checked_mul(2)?))
        .ok_or_else(|| ValidationError::new("H264 SPS cropped width overflow"))?;
    let frame_factor = 2_u32
        .checked_sub(frame_mbs_only)
        .ok_or_else(|| ValidationError::new("H264 SPS frame factor overflow"))?;
    let height = height_in_map_units
        .checked_mul(16)
        .and_then(|value| value.checked_mul(frame_factor))
        .and_then(|value| {
            value.checked_sub((crop_top + crop_bottom).checked_mul(2 * frame_factor)?)
        })
        .ok_or_else(|| ValidationError::new("H264 SPS cropped height overflow"))?;
    Ok(ParsedSps {
        profile_idc,
        level_idc,
        width,
        height,
    })
}

fn remove_emulation_prevention(bytes: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(bytes.len());
    let mut zeroes = 0_u8;
    for byte in bytes.iter().copied() {
        if zeroes >= 2 && byte == 3 {
            zeroes = 0;
            continue;
        }
        rbsp.push(byte);
        zeroes = if byte == 0 {
            zeroes.saturating_add(1)
        } else {
            0
        };
    }
    rbsp
}

struct BitReader<'a> {
    bytes: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit: 0 }
    }

    fn read_bit(&mut self) -> Result<u32, ValidationError> {
        let byte = self
            .bytes
            .get(self.bit / 8)
            .ok_or_else(|| ValidationError::new("truncated H264 SPS"))?;
        let value = u32::from((byte >> (7 - (self.bit % 8))) & 1);
        self.bit += 1;
        Ok(value)
    }

    fn read_ue(&mut self) -> Result<u32, ValidationError> {
        let mut leading_zeroes = 0_u32;
        while self.read_bit()? == 0 {
            leading_zeroes += 1;
            if leading_zeroes > 31 {
                return Err(ValidationError::new("oversized H264 Exp-Golomb value"));
            }
        }
        let mut suffix = 0_u32;
        for _ in 0..leading_zeroes {
            suffix = (suffix << 1) | self.read_bit()?;
        }
        (1_u32 << leading_zeroes)
            .checked_sub(1)
            .and_then(|base| base.checked_add(suffix))
            .ok_or_else(|| ValidationError::new("H264 Exp-Golomb overflow"))
    }

    fn read_se(&mut self) -> Result<i32, ValidationError> {
        let value = self.read_ue()?;
        if value & 1 == 0 {
            Ok(-i32::try_from(value / 2)
                .map_err(|_| ValidationError::new("H264 signed Exp-Golomb overflow"))?)
        } else {
            i32::try_from(value.div_ceil(2))
                .map_err(|_| ValidationError::new("H264 signed Exp-Golomb overflow"))
        }
    }
}

fn opus_packet_samples_48khz(payload: &[u8]) -> Result<u32, ValidationError> {
    let toc = *payload
        .first()
        .ok_or_else(|| ValidationError::new("empty Opus packet"))?;
    let samples_per_frame = if toc & 0x80 != 0 {
        (48_000_u32 << ((toc >> 3) & 0x03)) / 400
    } else if toc & 0x60 == 0x60 {
        if toc & 0x08 != 0 {
            48_000 / 50
        } else {
            48_000 / 100
        }
    } else {
        let size = (toc >> 3) & 0x03;
        if size == 3 {
            48_000 * 60 / 1_000
        } else {
            (48_000_u32 << size) / 100
        }
    };
    let frames = match toc & 0x03 {
        0 => 1,
        1 | 2 => 2,
        3 => u32::from(
            payload
                .get(1)
                .ok_or_else(|| ValidationError::new("truncated Opus frame-count byte"))?
                & 0x3f,
        ),
        _ => unreachable!(),
    };
    let total = samples_per_frame
        .checked_mul(frames)
        .ok_or_else(|| ValidationError::new("Opus packet duration overflow"))?;
    if frames == 0 || total > 5_760 {
        return Err(ValidationError::new("invalid Opus packet duration"));
    }
    Ok(total)
}

pub fn write_command_frame(writer: &mut impl Write, command: &Command) -> Result<(), FrameError> {
    command
        .validate()
        .map_err(|error| FrameError::Encode(error.to_string()))?;
    write_json_frame(writer, command)
}

pub fn read_command_frame(reader: &mut impl Read) -> Result<Option<Command>, FrameError> {
    let command: Option<Command> = read_json_frame(reader)?;
    if let Some(command) = &command {
        command
            .validate()
            .map_err(|error| FrameError::Decode(error.to_string()))?;
    }
    Ok(command)
}

pub fn read_initial_frame(reader: &mut impl Read) -> Result<Option<InitialFrame>, FrameError> {
    let Some(value) = read_json_frame::<serde_json::Value>(reader)? else {
        return Ok(None);
    };
    let object = value.as_object().ok_or_else(|| {
        FrameError::Decode("media-host initial frame must be a JSON object".into())
    })?;
    let has_capability_kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == crate::media_capabilities::REQUEST_KIND);
    let has_capture_probe_kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == crate::capture_probe::REQUEST_KIND);
    let has_screen_capture_permission_kind = object
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == crate::screen_capture_permission::REQUEST_KIND);
    let has_session_command = object
        .get("body")
        .and_then(serde_json::Value::as_object)
        .and_then(|body| body.get("command"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|command| command == "start_prepared");
    if usize::from(has_capture_probe_kind)
        + usize::from(has_capability_kind)
        + usize::from(has_screen_capture_permission_kind)
        + usize::from(has_session_command)
        != 1
    {
        return Err(FrameError::Decode(
            "media-host initial frame must select exactly one capture probe, capability, permission, or session mode"
                .into(),
        ));
    }
    if has_capture_probe_kind {
        let request: crate::capture_probe::Request =
            serde_json::from_value(value).map_err(|error| FrameError::Decode(error.to_string()))?;
        request
            .validate()
            .map_err(|error| FrameError::Decode(error.to_string()))?;
        return Ok(Some(InitialFrame::CaptureProbe(request)));
    }
    if has_capability_kind {
        let request: crate::media_capabilities::Request =
            serde_json::from_value(value).map_err(|error| FrameError::Decode(error.to_string()))?;
        request
            .validate()
            .map_err(|error| FrameError::Decode(error.to_string()))?;
        return Ok(Some(InitialFrame::Capability(request)));
    }
    if has_screen_capture_permission_kind {
        let request: crate::screen_capture_permission::Request =
            serde_json::from_value(value).map_err(|error| FrameError::Decode(error.to_string()))?;
        request
            .validate()
            .map_err(|error| FrameError::Decode(error.to_string()))?;
        return Ok(Some(InitialFrame::ScreenCapturePermission(request)));
    }
    let command: Command =
        serde_json::from_value(value).map_err(|error| FrameError::Decode(error.to_string()))?;
    command
        .validate()
        .map_err(|error| FrameError::Decode(error.to_string()))?;
    if !matches!(command.body, CommandBody::StartPrepared { .. }) {
        return Err(FrameError::Decode(
            "media-host session must begin with start_prepared".into(),
        ));
    }
    Ok(Some(InitialFrame::Session(command)))
}

fn write_json_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), FrameError> {
    let bytes = serde_json::to_vec(value).map_err(|error| FrameError::Encode(error.to_string()))?;
    if bytes.is_empty() || bytes.len() > MAX_METADATA_BYTES {
        return Err(FrameError::Oversized);
    }
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|()| writer.write_all(&bytes))
        .and_then(|()| writer.flush())
        .map_err(|error| FrameError::Io(error.to_string()))
}

fn read_json_frame<T: for<'de> Deserialize<'de>>(
    reader: &mut impl Read,
) -> Result<Option<T>, FrameError> {
    let Some(first) = read_first_byte(reader)? else {
        return Ok(None);
    };
    let mut header = [0_u8; 4];
    header[0] = first;
    reader
        .read_exact(&mut header[1..])
        .map_err(|_| FrameError::UnexpectedEof)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_METADATA_BYTES {
        return Err(FrameError::Oversized);
    }
    let mut bytes = vec![0_u8; length];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| FrameError::UnexpectedEof)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| FrameError::Decode(error.to_string()))
}

pub fn write_event_frame(
    writer: &mut impl Write,
    physical_lane: MediaLane,
    metadata: &EventMetadata,
    payload: &[u8],
) -> Result<(), FrameError> {
    metadata
        .validate_shape(physical_lane, payload)
        .map_err(|error| FrameError::Encode(error.to_string()))?;
    match physical_lane {
        MediaLane::Control => write_control_event_frame(writer, metadata, payload),
        MediaLane::Video => write_video_event_frame(writer, metadata, payload),
        MediaLane::Audio => write_audio_event_frame(writer, metadata, payload),
    }
}

pub fn read_event_frame(
    reader: &mut impl Read,
    physical_lane: MediaLane,
    expected_fence: Option<&GenerationFence>,
) -> Result<Option<(EventMetadata, Vec<u8>)>, FrameError> {
    match physical_lane {
        MediaLane::Control => read_control_event_frame(reader, physical_lane),
        MediaLane::Video | MediaLane::Audio => {
            let fence = expected_fence.ok_or_else(|| {
                FrameError::Decode("binary media frame requires its generation fence".into())
            })?;
            read_binary_media_event_frame(reader, physical_lane, fence)
        }
    }
}

const VIDEO_FRAME_MAGIC: [u8; 4] = *b"RVID";
const AUDIO_FRAME_MAGIC: [u8; 4] = *b"RAUD";
const BINARY_MEDIA_FRAME_VERSION: u8 = 1;
const VIDEO_FRAME_HEADER_BYTES: usize = 88;
const AUDIO_FRAME_HEADER_BYTES: usize = 64;
const VIDEO_FLAG_KEYFRAME: u8 = 1 << 0;
const VIDEO_FLAG_SPS_PPS: u8 = 1 << 1;
const VIDEO_FLAG_DISCONTINUITY: u8 = 1 << 2;
const AUDIO_FLAG_DISCONTINUITY: u8 = 1 << 0;

fn write_control_event_frame(
    writer: &mut impl Write,
    metadata: &EventMetadata,
    payload: &[u8],
) -> Result<(), FrameError> {
    let metadata_bytes =
        serde_json::to_vec(metadata).map_err(|error| FrameError::Encode(error.to_string()))?;
    if metadata_bytes.is_empty()
        || metadata_bytes.len() > MAX_METADATA_BYTES
        || payload.len() > MAX_PAYLOAD_BYTES
    {
        return Err(FrameError::Oversized);
    }
    writer
        .write_all(&(metadata_bytes.len() as u32).to_be_bytes())
        .and_then(|()| writer.write_all(&(payload.len() as u32).to_be_bytes()))
        .and_then(|()| writer.write_all(&metadata_bytes))
        .and_then(|()| writer.write_all(payload))
        .and_then(|()| writer.flush())
        .map_err(|error| FrameError::Io(error.to_string()))
}

fn write_video_event_frame(
    writer: &mut impl Write,
    metadata: &EventMetadata,
    payload: &[u8],
) -> Result<(), FrameError> {
    let mut header = [0_u8; VIDEO_FRAME_HEADER_BYTES];
    encode_binary_media_header(&mut header, MediaLane::Video, metadata, payload.len())?;
    writer
        .write_all(&header)
        .and_then(|()| writer.write_all(payload))
        .and_then(|()| writer.flush())
        .map_err(|error| FrameError::Io(error.to_string()))
}

fn write_audio_event_frame(
    writer: &mut impl Write,
    metadata: &EventMetadata,
    payload: &[u8],
) -> Result<(), FrameError> {
    let mut header = [0_u8; AUDIO_FRAME_HEADER_BYTES];
    encode_binary_media_header(&mut header, MediaLane::Audio, metadata, payload.len())?;
    writer
        .write_all(&header)
        .and_then(|()| writer.write_all(payload))
        .and_then(|()| writer.flush())
        .map_err(|error| FrameError::Io(error.to_string()))
}

pub(crate) fn encode_binary_media_header(
    header: &mut [u8],
    physical_lane: MediaLane,
    metadata: &EventMetadata,
    payload_len: usize,
) -> Result<(), FrameError> {
    encode_binary_media_header_compact(
        header,
        physical_lane,
        generation_nonce_bytes(&metadata.fence)?,
        metadata.sequence,
        metadata.observed_at_ms,
        &metadata.body,
        payload_len,
    )
}

pub(crate) fn encode_binary_media_header_compact(
    header: &mut [u8],
    physical_lane: MediaLane,
    generation_nonce: [u8; 16],
    sequence: u64,
    observed_at_ms: u64,
    body: &EventBody,
    payload_len: usize,
) -> Result<(), FrameError> {
    let expected_header_len = binary_media_header_len(physical_lane)?;
    if header.len() != expected_header_len || payload_len == 0 || payload_len > MAX_PAYLOAD_BYTES {
        return Err(FrameError::Oversized);
    }
    header.fill(0);
    match physical_lane {
        MediaLane::Video => encode_video_media_header(
            header,
            generation_nonce,
            sequence,
            observed_at_ms,
            body,
            payload_len,
        ),
        MediaLane::Audio => encode_audio_media_header(
            header,
            generation_nonce,
            sequence,
            observed_at_ms,
            body,
            payload_len,
        ),
        MediaLane::Control => unreachable!(),
    }
}

fn encode_video_media_header(
    header: &mut [u8],
    generation_nonce: [u8; 16],
    sequence: u64,
    observed_at_ms: u64,
    body: &EventBody,
    payload_len: usize,
) -> Result<(), FrameError> {
    let EventBody::VideoH264 {
        media_gate,
        pts_90khz,
        duration_90khz,
        keyframe,
        sps_pps_present,
        discontinuity,
        codec_generation,
        width,
        height,
        encode_submitted_at_ms,
        encoded_at_ms,
    } = body
    else {
        return Err(FrameError::Encode(
            "video lane requires VideoH264 metadata".into(),
        ));
    };
    let payload_len = u32::try_from(payload_len).map_err(|_| FrameError::Oversized)?;
    header[..4].copy_from_slice(&VIDEO_FRAME_MAGIC);
    header[4] = BINARY_MEDIA_FRAME_VERSION;
    header[5] = u8::from(*keyframe) * VIDEO_FLAG_KEYFRAME
        | u8::from(*sps_pps_present) * VIDEO_FLAG_SPS_PPS
        | u8::from(*discontinuity) * VIDEO_FLAG_DISCONTINUITY;
    header[6..8].copy_from_slice(&(VIDEO_FRAME_HEADER_BYTES as u16).to_be_bytes());
    header[8..16].copy_from_slice(&sequence.to_be_bytes());
    header[16..24].copy_from_slice(&observed_at_ms.to_be_bytes());
    header[24..40].copy_from_slice(&generation_nonce);
    header[40..44].copy_from_slice(&media_gate.to_be_bytes());
    header[44..48].copy_from_slice(&codec_generation.to_be_bytes());
    header[48..56].copy_from_slice(&pts_90khz.to_be_bytes());
    header[56..60].copy_from_slice(&duration_90khz.to_be_bytes());
    header[60..64].copy_from_slice(&width.to_be_bytes());
    header[64..68].copy_from_slice(&height.to_be_bytes());
    header[68..76].copy_from_slice(&encode_submitted_at_ms.to_be_bytes());
    header[76..84].copy_from_slice(&encoded_at_ms.to_be_bytes());
    header[84..88].copy_from_slice(&payload_len.to_be_bytes());
    Ok(())
}

fn encode_audio_media_header(
    header: &mut [u8],
    generation_nonce: [u8; 16],
    sequence: u64,
    observed_at_ms: u64,
    body: &EventBody,
    payload_len: usize,
) -> Result<(), FrameError> {
    let EventBody::AudioOpus {
        media_gate,
        pts_48khz,
        duration_samples,
        discontinuity,
        sample_rate_hz,
        channels,
    } = body
    else {
        return Err(FrameError::Encode(
            "audio lane requires AudioOpus metadata".into(),
        ));
    };
    let payload_len = u32::try_from(payload_len).map_err(|_| FrameError::Oversized)?;
    header[..4].copy_from_slice(&AUDIO_FRAME_MAGIC);
    header[4] = BINARY_MEDIA_FRAME_VERSION;
    header[5] = u8::from(*discontinuity) * AUDIO_FLAG_DISCONTINUITY;
    header[6..8].copy_from_slice(&(AUDIO_FRAME_HEADER_BYTES as u16).to_be_bytes());
    header[8..16].copy_from_slice(&sequence.to_be_bytes());
    header[16..24].copy_from_slice(&observed_at_ms.to_be_bytes());
    header[24..40].copy_from_slice(&generation_nonce);
    header[40..44].copy_from_slice(&media_gate.to_be_bytes());
    header[44..52].copy_from_slice(&pts_48khz.to_be_bytes());
    header[52..54].copy_from_slice(&duration_samples.to_be_bytes());
    header[54] = *channels;
    header[56..60].copy_from_slice(&sample_rate_hz.to_be_bytes());
    header[60..64].copy_from_slice(&payload_len.to_be_bytes());
    Ok(())
}

fn read_control_event_frame(
    reader: &mut impl Read,
    physical_lane: MediaLane,
) -> Result<Option<(EventMetadata, Vec<u8>)>, FrameError> {
    let Some(first) = read_first_byte(reader)? else {
        return Ok(None);
    };
    let mut header = [0_u8; 8];
    header[0] = first;
    reader
        .read_exact(&mut header[1..])
        .map_err(|_| FrameError::UnexpectedEof)?;
    let metadata_len = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
    let payload_len = u32::from_be_bytes(header[4..].try_into().unwrap()) as usize;
    if metadata_len == 0 || metadata_len > MAX_METADATA_BYTES || payload_len > MAX_PAYLOAD_BYTES {
        return Err(FrameError::Oversized);
    }
    let mut metadata_bytes = vec![0_u8; metadata_len];
    reader
        .read_exact(&mut metadata_bytes)
        .map_err(|_| FrameError::UnexpectedEof)?;
    let metadata: EventMetadata = serde_json::from_slice(&metadata_bytes)
        .map_err(|error| FrameError::Decode(error.to_string()))?;
    if metadata.required_lane() != physical_lane {
        return Err(FrameError::Decode(
            "media event arrived on the wrong physical lane".into(),
        ));
    }
    let mut payload = vec![0_u8; payload_len];
    reader
        .read_exact(&mut payload)
        .map_err(|_| FrameError::UnexpectedEof)?;
    metadata
        .validate_shape(physical_lane, &payload)
        .map_err(|error| FrameError::Decode(error.to_string()))?;
    Ok(Some((metadata, payload)))
}

fn read_binary_media_event_frame(
    reader: &mut impl Read,
    physical_lane: MediaLane,
    fence: &GenerationFence,
) -> Result<Option<(EventMetadata, Vec<u8>)>, FrameError> {
    let Some(first) = read_first_byte(reader)? else {
        return Ok(None);
    };
    let header_len = match physical_lane {
        MediaLane::Video => VIDEO_FRAME_HEADER_BYTES,
        MediaLane::Audio => AUDIO_FRAME_HEADER_BYTES,
        MediaLane::Control => unreachable!(),
    };
    let mut header = vec![0_u8; header_len];
    header[0] = first;
    reader
        .read_exact(&mut header[1..])
        .map_err(|_| FrameError::UnexpectedEof)?;
    let (metadata, payload_len) = decode_binary_media_header(&header, physical_lane, fence)?;
    let mut payload = vec![0_u8; payload_len];
    reader
        .read_exact(&mut payload)
        .map_err(|_| FrameError::UnexpectedEof)?;
    metadata
        .validate_shape(physical_lane, &payload)
        .map_err(|error| FrameError::Decode(error.to_string()))?;
    Ok(Some((metadata, payload)))
}

/// Decode one complete fixed `RVID`/`RAUD` frame in place.
///
/// Shared-media consumers use this function to validate mapped bytes without
/// allocating or copying the codec payload. The returned slice is borrowed
/// from `frame` and remains protected by the caller's slot lease.
pub fn decode_binary_media_event_frame<'a>(
    frame: &'a [u8],
    physical_lane: MediaLane,
    fence: &GenerationFence,
) -> Result<(EventMetadata, &'a [u8]), FrameError> {
    let (binary, payload) = decode_binary_media_event_frame_compact(
        frame,
        physical_lane,
        generation_nonce_bytes(fence)?,
    )?;
    Ok((
        EventMetadata {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            fence: fence.clone(),
            sequence: binary.sequence,
            observed_at_ms: binary.observed_at_ms,
            body: binary.body,
        },
        payload,
    ))
}

/// Decode a complete fixed media frame without allocating generation or
/// protocol strings. The caller supplies the generation nonce cached when the
/// shared lane was opened.
pub fn decode_binary_media_event_frame_compact(
    frame: &[u8],
    physical_lane: MediaLane,
    expected_generation_nonce: [u8; 16],
) -> Result<(BinaryMediaEvent, &[u8]), FrameError> {
    let header_len = binary_media_header_len(physical_lane)?;
    if frame.len() < header_len {
        return Err(FrameError::UnexpectedEof);
    }
    let (metadata, payload_len) = decode_binary_media_header_compact(
        &frame[..header_len],
        physical_lane,
        expected_generation_nonce,
    )?;
    let expected_len = header_len
        .checked_add(payload_len)
        .ok_or(FrameError::Oversized)?;
    if frame.len() != expected_len {
        return Err(FrameError::Decode(
            "binary media slot length differs from its fixed header".into(),
        ));
    }
    let payload = &frame[header_len..];
    metadata
        .validate_shape(physical_lane, payload)
        .map_err(|error| FrameError::Decode(error.to_string()))?;
    Ok((metadata, payload))
}

pub fn binary_media_frame_capacity(
    physical_lane: MediaLane,
    payload_capacity: usize,
) -> Result<usize, FrameError> {
    if payload_capacity == 0 || payload_capacity > MAX_PAYLOAD_BYTES {
        return Err(FrameError::Oversized);
    }
    binary_media_header_len(physical_lane)?
        .checked_add(payload_capacity)
        .ok_or(FrameError::Oversized)
}

pub(crate) fn binary_media_header_len(physical_lane: MediaLane) -> Result<usize, FrameError> {
    match physical_lane {
        MediaLane::Video => Ok(VIDEO_FRAME_HEADER_BYTES),
        MediaLane::Audio => Ok(AUDIO_FRAME_HEADER_BYTES),
        MediaLane::Control => Err(FrameError::Decode(
            "control events do not use fixed binary media headers".into(),
        )),
    }
}

fn decode_binary_media_header(
    header: &[u8],
    physical_lane: MediaLane,
    fence: &GenerationFence,
) -> Result<(EventMetadata, usize), FrameError> {
    let (binary, payload_len) =
        decode_binary_media_header_compact(header, physical_lane, generation_nonce_bytes(fence)?)?;
    Ok((
        EventMetadata {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            fence: fence.clone(),
            sequence: binary.sequence,
            observed_at_ms: binary.observed_at_ms,
            body: binary.body,
        },
        payload_len,
    ))
}

fn decode_binary_media_header_compact(
    header: &[u8],
    physical_lane: MediaLane,
    expected_generation_nonce: [u8; 16],
) -> Result<(BinaryMediaEvent, usize), FrameError> {
    let header_len = binary_media_header_len(physical_lane)?;
    if header.len() != header_len {
        return Err(FrameError::UnexpectedEof);
    }
    let expected_magic = match physical_lane {
        MediaLane::Video => VIDEO_FRAME_MAGIC,
        MediaLane::Audio => AUDIO_FRAME_MAGIC,
        MediaLane::Control => unreachable!(),
    };
    if header[..4] != expected_magic
        || header[4] != BINARY_MEDIA_FRAME_VERSION
        || usize::from(u16::from_be_bytes(header[6..8].try_into().unwrap())) != header_len
        || header[24..40] != expected_generation_nonce
    {
        return Err(FrameError::Decode(
            "binary media frame header or generation fence mismatch".into(),
        ));
    }
    let sequence = u64::from_be_bytes(header[8..16].try_into().unwrap());
    let observed_at_ms = u64::from_be_bytes(header[16..24].try_into().unwrap());
    let (body, payload_len) = match physical_lane {
        MediaLane::Video => {
            if header[5] & !(VIDEO_FLAG_KEYFRAME | VIDEO_FLAG_SPS_PPS | VIDEO_FLAG_DISCONTINUITY)
                != 0
            {
                return Err(FrameError::Decode("unknown binary video flags".into()));
            }
            (
                EventBody::VideoH264 {
                    media_gate: u32::from_be_bytes(header[40..44].try_into().unwrap()),
                    codec_generation: u32::from_be_bytes(header[44..48].try_into().unwrap()),
                    pts_90khz: u64::from_be_bytes(header[48..56].try_into().unwrap()),
                    duration_90khz: u32::from_be_bytes(header[56..60].try_into().unwrap()),
                    width: u32::from_be_bytes(header[60..64].try_into().unwrap()),
                    height: u32::from_be_bytes(header[64..68].try_into().unwrap()),
                    encode_submitted_at_ms: u64::from_be_bytes(header[68..76].try_into().unwrap()),
                    encoded_at_ms: u64::from_be_bytes(header[76..84].try_into().unwrap()),
                    keyframe: header[5] & VIDEO_FLAG_KEYFRAME != 0,
                    sps_pps_present: header[5] & VIDEO_FLAG_SPS_PPS != 0,
                    discontinuity: header[5] & VIDEO_FLAG_DISCONTINUITY != 0,
                },
                u32::from_be_bytes(header[84..88].try_into().unwrap()) as usize,
            )
        }
        MediaLane::Audio => {
            if header[5] & !AUDIO_FLAG_DISCONTINUITY != 0 || header[55] != 0 {
                return Err(FrameError::Decode("invalid binary audio flags".into()));
            }
            (
                EventBody::AudioOpus {
                    media_gate: u32::from_be_bytes(header[40..44].try_into().unwrap()),
                    pts_48khz: u64::from_be_bytes(header[44..52].try_into().unwrap()),
                    duration_samples: u16::from_be_bytes(header[52..54].try_into().unwrap()),
                    channels: header[54],
                    discontinuity: header[5] & AUDIO_FLAG_DISCONTINUITY != 0,
                    sample_rate_hz: u32::from_be_bytes(header[56..60].try_into().unwrap()),
                },
                u32::from_be_bytes(header[60..64].try_into().unwrap()) as usize,
            )
        }
        MediaLane::Control => unreachable!(),
    };
    if payload_len == 0 || payload_len > MAX_PAYLOAD_BYTES {
        return Err(FrameError::Oversized);
    }
    let metadata = BinaryMediaEvent {
        sequence,
        observed_at_ms,
        body,
    };
    Ok((metadata, payload_len))
}

pub fn generation_nonce_bytes(fence: &GenerationFence) -> Result<[u8; 16], FrameError> {
    if fence.session_nonce.len() != 32 {
        return Err(FrameError::Decode("invalid media generation nonce".into()));
    }
    let mut bytes = [0_u8; 16];
    for (index, output) in bytes.iter_mut().enumerate() {
        let offset = index * 2;
        let high = hex_nibble(fence.session_nonce.as_bytes()[offset])?;
        let low = hex_nibble(fence.session_nonce.as_bytes()[offset + 1])?;
        *output = (high << 4) | low;
    }
    Ok(bytes)
}

fn hex_nibble(value: u8) -> Result<u8, FrameError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(FrameError::Decode(
            "media generation nonce is not lowercase hexadecimal".into(),
        )),
    }
}

fn read_first_byte(reader: &mut impl Read) -> Result<Option<u8>, FrameError> {
    let mut first = [0_u8; 1];
    loop {
        match reader.read(&mut first) {
            Ok(0) => return Ok(None),
            Ok(1) => return Ok(Some(first[0])),
            Ok(_) => unreachable!("one-byte read returned more than one byte"),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(FrameError::Io(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use openh264::encoder::{
        BitRate, Complexity, Encoder, EncoderConfig, FrameRate, IntraFramePeriod,
        Level as OpenH264Level, Profile, RateControlMode, UsageType,
    };
    use openh264::formats::{RgbSliceU8, YUVBuffer};
    use openh264::{OpenH264API, Timestamp};

    use super::*;

    fn application_target() -> NativeTargetPlan {
        NativeTargetPlan {
            kind: TargetKind::Application,
            display_id: Some(1),
            window_id: None,
            pid: Some(42),
            process_instance_id: Some("42:boot:9".into()),
            app_identity: Some("editor".into()),
            bundle_id: Some("com.example.Editor".into()),
            application: Some(ApplicationWindowSet {
                display_id: Some(1),
                display_ids: vec![1],
                primary_pid: 42,
                process_instance_id: Some("42:boot:9".into()),
                app_identity: Some("editor".into()),
                bundle_id: Some("com.example.Editor".into()),
                window_ids: vec![7, 9],
                window_set_epoch: 12,
                front_to_back_surfaces: vec![
                    ApplicationSurface {
                        window_id: 9,
                        x: 100,
                        y: 20,
                        width: 320,
                        height: 180,
                    },
                    ApplicationSurface {
                        window_id: 7,
                        x: 0,
                        y: 0,
                        width: 640,
                        height: 360,
                    },
                ],
                surface_layout_epoch: 13,
            }),
        }
    }

    fn video_config() -> VideoConfig {
        VideoConfig {
            codec: VideoCodec::H264AnnexB,
            width: 640,
            height: 360,
            fps: 30,
            bitrate_kbps: 2_500,
            keyframe_interval_frames: 30,
            max_pending_frames: 1,
            max_access_unit_bytes: 2 * 1024 * 1024,
            max_nal_unit_bytes: 2 * 1024 * 1024,
            h264_profile_idc: 66,
            h264_level_idc: 31,
        }
    }

    fn contract(audio: bool) -> StartContract {
        StartContract {
            target: application_target(),
            video: video_config(),
            audio: audio.then_some(AudioConfig {
                codec: AudioCodec::Opus,
                sample_rate_hz: 48_000,
                channels: 2,
                frame_duration_ms: 20,
                max_pending_packets: 4,
            }),
        }
    }

    fn fence(contract: &StartContract) -> GenerationFence {
        GenerationFence {
            process_generation: 7,
            build_id: "33".repeat(32),
            session_nonce: "11".repeat(16),
            transport_epoch: 3,
            media_source_epoch: 5,
            contract_digest: contract.digest().unwrap(),
        }
    }

    fn command(sequence: u64, fence: &GenerationFence, body: CommandBody) -> Command {
        Command {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.into(),
            fence: fence.clone(),
            sequence,
            body,
        }
    }

    fn event(sequence: u64, fence: &GenerationFence, body: EventBody) -> EventMetadata {
        EventMetadata {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.into(),
            fence: fence.clone(),
            sequence,
            observed_at_ms: 100 + sequence,
            body,
        }
    }

    fn proof(target: &NativeTargetPlan) -> CaptureProof {
        CaptureProof {
            backend: CaptureBackend::ScreenCaptureKit,
            observed_target: target.clone(),
            native_width: 640,
            native_height: 360,
            verified_at_ms: 100,
        }
    }

    fn real_openh264_idr() -> Vec<u8> {
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(2_500_000))
            .max_frame_rate(FrameRate::from_hz(30.0))
            .profile(Profile::Baseline)
            .level(OpenH264Level::Level_3_1)
            .complexity(Complexity::Low)
            .max_slice_len(1_160)
            .intra_frame_period(IntraFramePeriod::from_num_frames(30));
        let mut encoder = Encoder::with_api_config(OpenH264API::from_source(), config).unwrap();
        let rgb = vec![0_u8; 640 * 360 * 3];
        let yuv = YUVBuffer::from_rgb8_source(RgbSliceU8::new(&rgb, (640, 360)));
        encoder
            .encode_at(&yuv, Timestamp::from_millis(0))
            .unwrap()
            .to_vec()
    }

    #[test]
    fn application_membership_is_sorted_but_surface_order_is_front_to_back() {
        application_target().validate().unwrap();
        let mut invalid = application_target();
        invalid.application.as_mut().unwrap().window_ids = vec![9, 7];
        assert!(invalid.validate().is_err());
        let mut missing_layout = application_target();
        missing_layout
            .application
            .as_mut()
            .unwrap()
            .front_to_back_surfaces
            .clear();
        assert!(missing_layout.validate().is_err());
    }

    #[test]
    fn contract_digest_binds_exact_target_video_and_audio() {
        let contract = contract(true);
        let fence = fence(&contract);
        command(
            1,
            &fence,
            CommandBody::StartPrepared {
                contract: contract.clone(),
            },
        )
        .validate()
        .unwrap();
        let mut changed = contract;
        changed.video.bitrate_kbps += 1;
        assert!(
            command(1, &fence, CommandBody::StartPrepared { contract: changed })
                .validate()
                .is_err()
        );
    }

    #[test]
    fn initial_frame_selects_exactly_one_process_mode() {
        let capability = crate::media_capabilities::Request::probe_capabilities(7, 9);
        let mut capability_bytes = Vec::new();
        crate::write_frame(&mut capability_bytes, &capability).unwrap();
        assert!(matches!(
            read_initial_frame(&mut Cursor::new(capability_bytes)).unwrap(),
            Some(InitialFrame::Capability(_))
        ));

        let permission = crate::screen_capture_permission::Request::new(
            7,
            10,
            crate::screen_capture_permission::Operation::Status,
        );
        let mut permission_bytes = Vec::new();
        crate::write_frame(&mut permission_bytes, &permission).unwrap();
        assert!(matches!(
            read_initial_frame(&mut Cursor::new(permission_bytes)).unwrap(),
            Some(InitialFrame::ScreenCapturePermission(_))
        ));

        let contract = contract(false);
        let fence = fence(&contract);
        let start = command(1, &fence, CommandBody::StartPrepared { contract });
        let mut session_bytes = Vec::new();
        write_command_frame(&mut session_bytes, &start).unwrap();
        assert!(matches!(
            read_initial_frame(&mut Cursor::new(session_bytes)).unwrap(),
            Some(InitialFrame::Session(_))
        ));

        let mut ambiguous = Vec::new();
        write_json_frame(
            &mut ambiguous,
            &serde_json::json!({
                "schema_version": SCHEMA_VERSION,
                "protocol": PROTOCOL,
                "kind": crate::media_capabilities::REQUEST_KIND,
                "body": { "command": "start_prepared" }
            }),
        )
        .unwrap();
        assert!(read_initial_frame(&mut Cursor::new(ambiguous)).is_err());
    }

    #[test]
    fn binary_framing_preserves_embedded_nuls_and_physical_lane() {
        let contract = contract(false);
        let fence = fence(&contract);
        let metadata = event(
            1,
            &fence,
            EventBody::VideoH264 {
                media_gate: 1,
                pts_90khz: 0,
                duration_90khz: 3_000,
                keyframe: true,
                sps_pps_present: true,
                discontinuity: false,
                codec_generation: 1,
                width: 640,
                height: 360,
                encode_submitted_at_ms: 90,
                encoded_at_ms: 99,
            },
        );
        // Shape-valid framing bytes; semantic SPS validation belongs to the
        // conversation validator.
        let payload = b"\0\0\0\x01\x67\x42\0\x1f\x80\0\0\0\x01\x68\0\0\0\x01\x65\0";
        let mut bytes = Vec::new();
        write_event_frame(&mut bytes, MediaLane::Video, &metadata, payload).unwrap();
        assert_eq!(&bytes[..4], b"RVID");
        assert_eq!(bytes.len(), VIDEO_FRAME_HEADER_BYTES + payload.len());
        assert!(!bytes
            .windows(PROTOCOL.len())
            .any(|window| window == PROTOCOL.as_bytes()));
        let (decoded, decoded_payload) = read_event_frame(
            &mut Cursor::new(bytes.clone()),
            MediaLane::Video,
            Some(&fence),
        )
        .unwrap()
        .unwrap();
        assert_eq!(decoded, metadata);
        assert_eq!(decoded_payload, payload);
        let (borrowed_metadata, borrowed_payload) =
            decode_binary_media_event_frame(&bytes, MediaLane::Video, &fence).unwrap();
        assert_eq!(borrowed_metadata, metadata);
        assert_eq!(borrowed_payload, payload);
        assert_eq!(
            borrowed_payload.as_ptr(),
            bytes[VIDEO_FRAME_HEADER_BYTES..].as_ptr()
        );
        assert!(read_event_frame(&mut Cursor::new(bytes), MediaLane::Audio, Some(&fence)).is_err());
    }

    #[test]
    fn audio_hot_lane_uses_fixed_header_without_json_metadata() {
        let contract = contract(true);
        let fence = fence(&contract);
        let metadata = event(
            1,
            &fence,
            EventBody::AudioOpus {
                media_gate: 1,
                pts_48khz: 960,
                duration_samples: 960,
                discontinuity: true,
                sample_rate_hz: 48_000,
                channels: 2,
            },
        );
        let payload = [0x98, 0x00];
        let mut bytes = Vec::new();
        write_event_frame(&mut bytes, MediaLane::Audio, &metadata, &payload).unwrap();
        assert_eq!(&bytes[..4], b"RAUD");
        assert_eq!(bytes.len(), AUDIO_FRAME_HEADER_BYTES + payload.len());
        assert!(!bytes
            .windows(PROTOCOL.len())
            .any(|window| window == PROTOCOL.as_bytes()));
        let (decoded, decoded_payload) =
            read_event_frame(&mut Cursor::new(bytes), MediaLane::Audio, Some(&fence))
                .unwrap()
                .unwrap();
        assert_eq!(decoded, metadata);
        assert_eq!(decoded_payload, payload);
    }

    #[test]
    fn framing_rejects_oversize_before_allocation() {
        let contract = contract(false);
        let fence = fence(&contract);
        let metadata = event(
            1,
            &fence,
            EventBody::VideoH264 {
                media_gate: 1,
                pts_90khz: 0,
                duration_90khz: 3_000,
                keyframe: true,
                sps_pps_present: true,
                discontinuity: false,
                codec_generation: 1,
                width: 640,
                height: 360,
                encode_submitted_at_ms: 90,
                encoded_at_ms: 99,
            },
        );
        let mut bytes = Vec::new();
        write_event_frame(&mut bytes, MediaLane::Video, &metadata, b"\0\0\0\x01\x65").unwrap();
        bytes[84..88].copy_from_slice(&((MAX_PAYLOAD_BYTES + 1) as u32).to_be_bytes());
        assert!(matches!(
            read_event_frame(&mut Cursor::new(bytes), MediaLane::Video, Some(&fence)),
            Err(FrameError::Oversized)
        ));
    }

    #[test]
    fn conversation_rejects_unsolicited_lifecycle_and_poison_is_sticky() {
        let contract = contract(false);
        let fence = fence(&contract);
        let mut validator = MediaConversationValidator::new(fence.clone()).unwrap();
        let unsolicited = event(
            1,
            &fence,
            EventBody::Activated {
                command_sequence: 1,
            },
        );
        assert!(validator
            .observe(MediaLane::Control, &unsolicited, &[])
            .is_err());
        assert!(validator
            .register_command(&command(1, &fence, CommandBody::StartPrepared { contract }))
            .is_err());
    }

    #[test]
    fn conversation_correlates_prepare_activate_and_stop() {
        let contract = contract(false);
        let fence = fence(&contract);
        let mut validator = MediaConversationValidator::new(fence.clone()).unwrap();
        validator
            .register_command(&command(
                1,
                &fence,
                CommandBody::StartPrepared {
                    contract: contract.clone(),
                },
            ))
            .unwrap();
        validator
            .observe(
                MediaLane::Control,
                &event(
                    1,
                    &fence,
                    EventBody::Prepared {
                        command_sequence: 1,
                        capture_proof: proof(&contract.target),
                    },
                ),
                &[],
            )
            .unwrap();
        validator
            .register_command(&command(2, &fence, CommandBody::Activate))
            .unwrap();
        validator
            .observe(
                MediaLane::Control,
                &event(
                    2,
                    &fence,
                    EventBody::Activated {
                        command_sequence: 2,
                    },
                ),
                &[],
            )
            .unwrap();
        validator
            .register_command(&command(
                3,
                &fence,
                CommandBody::BeginMedia {
                    activation_command_sequence: 2,
                },
            ))
            .unwrap();
        validator
            .register_command(&command(4, &fence, CommandBody::Stop))
            .unwrap();
        validator
            .observe(
                MediaLane::Control,
                &event(
                    3,
                    &fence,
                    EventBody::Stopped {
                        command_sequence: 4,
                    },
                ),
                &[],
            )
            .unwrap();
        assert!(validator.is_terminal());
        validator.finish_control_eof().unwrap();
    }

    #[test]
    fn host_command_state_requires_completed_prepare_and_cross_lane_barriers() {
        let contract = contract(false);
        let fence = fence(&contract);
        let mut host = MediaHostCommandValidator::new(fence.clone()).unwrap();
        host.observe(&command(
            1,
            &fence,
            CommandBody::StartPrepared {
                contract: contract.clone(),
            },
        ))
        .unwrap();
        assert_eq!(host.contract(), Some(&contract));
        host.mark_prepared(1).unwrap();
        host.observe(&command(2, &fence, CommandBody::Activate))
            .unwrap();
        host.mark_activated(2).unwrap();
        host.observe(&command(
            3,
            &fence,
            CommandBody::BeginMedia {
                activation_command_sequence: 2,
            },
        ))
        .unwrap();
        assert_eq!(host.media_gate(), 1);
        let mut next_video = video_config();
        next_video.bitrate_kbps = 1_500;
        host.observe(&command(
            4,
            &fence,
            CommandBody::Reconfigure {
                video: next_video,
                force_keyframe: true,
            },
        ))
        .unwrap();
        host.mark_reconfigured(4).unwrap();
        host.observe(&command(
            5,
            &fence,
            CommandBody::ResumeMedia {
                reconfigure_command_sequence: 4,
            },
        ))
        .unwrap();
        assert_eq!(host.media_gate(), 2);
        host.observe(&command(6, &fence, CommandBody::Stop))
            .unwrap();
        host.mark_stopped(6).unwrap();
    }

    #[test]
    fn media_gate_discards_in_flight_old_frames_during_reconfigure() {
        let contract = contract(false);
        let fence = fence(&contract);
        let mut validator = activated_validator(contract, fence.clone());
        let mut next_video = video_config();
        next_video.bitrate_kbps = 1_500;
        validator
            .register_command(&command(
                4,
                &fence,
                CommandBody::Reconfigure {
                    video: next_video,
                    force_keyframe: true,
                },
            ))
            .unwrap();
        let stale = event(
            1,
            &fence,
            EventBody::VideoH264 {
                media_gate: 1,
                pts_90khz: 3_000,
                duration_90khz: 3_000,
                keyframe: false,
                sps_pps_present: false,
                discontinuity: false,
                codec_generation: 1,
                width: 640,
                height: 360,
                encode_submitted_at_ms: 90,
                encoded_at_ms: 99,
            },
        );
        assert_eq!(
            validator
                .observe(MediaLane::Video, &stale, b"\0\0\0\x01\x41delta")
                .unwrap(),
            MediaObservation::StaleDiscarded
        );
    }

    #[test]
    fn explicit_backpressure_drop_advances_sequence_and_requires_recovery_idr() {
        let contract = contract(false);
        let fence = fence(&contract);
        let mut validator = activated_validator(contract, fence.clone());
        assert_eq!(
            validator
                .observe_backpressure_drop(MediaLane::Video, 1, 100, 1)
                .unwrap(),
            MediaObservation::BackpressureDiscarded
        );
        validator
            .register_command(&command(4, &fence, CommandBody::RequestKeyframe))
            .unwrap();
        validator
            .observe(
                MediaLane::Control,
                &event(
                    3,
                    &fence,
                    EventBody::KeyframeRequested {
                        command_sequence: 4,
                    },
                ),
                &[],
            )
            .unwrap();
        let recovery = event(
            2,
            &fence,
            EventBody::VideoH264 {
                media_gate: 1,
                pts_90khz: 3_000,
                duration_90khz: 3_000,
                keyframe: true,
                sps_pps_present: true,
                discontinuity: true,
                codec_generation: 1,
                width: 640,
                height: 360,
                encode_submitted_at_ms: 101,
                encoded_at_ms: 102,
            },
        );
        assert_eq!(
            validator
                .observe(MediaLane::Video, &recovery, &real_openh264_idr())
                .unwrap(),
            MediaObservation::Accepted
        );
    }

    #[test]
    fn audio_is_rejected_when_not_negotiated() {
        let contract = contract(false);
        let fence = fence(&contract);
        let mut validator = activated_validator(contract, fence.clone());
        let audio = event(
            1,
            &fence,
            EventBody::AudioOpus {
                media_gate: 1,
                pts_48khz: 0,
                duration_samples: 960,
                discontinuity: true,
                sample_rate_hz: 48_000,
                channels: 2,
            },
        );
        assert!(validator
            .observe(MediaLane::Audio, &audio, &[0x98, 0])
            .is_err());
    }

    #[test]
    fn h264_metadata_cannot_lie_about_idr_or_parameter_sets() {
        let contract = contract(false);
        let fence = fence(&contract);
        let mut validator = activated_validator(contract, fence.clone());
        let metadata = event(
            1,
            &fence,
            EventBody::VideoH264 {
                media_gate: 1,
                pts_90khz: 0,
                duration_90khz: 3_000,
                keyframe: true,
                sps_pps_present: true,
                discontinuity: false,
                codec_generation: 1,
                width: 640,
                height: 360,
                encode_submitted_at_ms: 90,
                encoded_at_ms: 99,
            },
        );
        assert!(validator
            .observe(MediaLane::Video, &metadata, b"\0\0\0\x01\x41delta")
            .is_err());
    }

    #[test]
    fn real_openh264_idr_matches_sps_and_is_accepted_after_activation_barrier() {
        let contract = contract(false);
        let fence = fence(&contract);
        let mut validator = activated_validator(contract, fence.clone());
        let payload = real_openh264_idr();
        let inspection = inspect_h264_annex_b(&payload).unwrap();
        assert!(inspection.has_idr && inspection.has_sps && inspection.has_pps);
        let parsed = parse_baseline_sps(inspection.sps.unwrap()).unwrap();
        assert_eq!((parsed.width, parsed.height), (640, 360));
        let metadata = event(
            1,
            &fence,
            EventBody::VideoH264 {
                media_gate: 1,
                pts_90khz: 0,
                duration_90khz: 3_000,
                keyframe: true,
                sps_pps_present: true,
                discontinuity: true,
                codec_generation: 1,
                width: 640,
                height: 360,
                encode_submitted_at_ms: 90,
                encoded_at_ms: 99,
            },
        );
        assert_eq!(
            validator
                .observe(MediaLane::Video, &metadata, &payload)
                .unwrap(),
            MediaObservation::Accepted
        );
    }

    #[test]
    fn compact_binary_metadata_preserves_the_canonical_conversation_state_machine() {
        let contract = contract(false);
        let fence = fence(&contract);
        let mut validator = activated_validator(contract, fence);
        let payload = real_openh264_idr();
        let metadata = BinaryMediaEvent {
            sequence: 1,
            observed_at_ms: 100,
            body: EventBody::VideoH264 {
                media_gate: 1,
                pts_90khz: 0,
                duration_90khz: 3_000,
                keyframe: true,
                sps_pps_present: true,
                discontinuity: true,
                codec_generation: 1,
                width: 640,
                height: 360,
                encode_submitted_at_ms: 90,
                encoded_at_ms: 99,
            },
        };
        assert_eq!(
            validator
                .observe_binary_media(MediaLane::Video, &metadata, &payload)
                .unwrap(),
            MediaObservation::Accepted
        );
    }

    #[test]
    fn negotiated_nal_bound_prevents_rtp_fragment_copy_from_becoming_normal_path() {
        let mut contract = contract(false);
        contract.video.max_nal_unit_bytes = 1_160;
        let fence = fence(&contract);
        let mut validator = activated_validator(contract, fence.clone());
        let idr = real_openh264_idr();
        assert!(inspect_h264_annex_b(&idr).unwrap().max_nal_unit_bytes <= 1_160);
        validator
            .observe_binary_media(
                MediaLane::Video,
                &BinaryMediaEvent {
                    sequence: 1,
                    observed_at_ms: 100,
                    body: EventBody::VideoH264 {
                        media_gate: 1,
                        pts_90khz: 0,
                        duration_90khz: 3_000,
                        keyframe: true,
                        sps_pps_present: true,
                        discontinuity: true,
                        codec_generation: 1,
                        width: 640,
                        height: 360,
                        encode_submitted_at_ms: 90,
                        encoded_at_ms: 99,
                    },
                },
                &idr,
            )
            .unwrap();
        let mut oversized_delta = vec![0_u8; 4 + 1_161];
        oversized_delta[..5].copy_from_slice(&[0, 0, 0, 1, 0x41]);
        assert!(validator
            .observe_binary_media(
                MediaLane::Video,
                &BinaryMediaEvent {
                    sequence: 2,
                    observed_at_ms: 101,
                    body: EventBody::VideoH264 {
                        media_gate: 1,
                        pts_90khz: 3_000,
                        duration_90khz: 3_000,
                        keyframe: false,
                        sps_pps_present: false,
                        discontinuity: false,
                        codec_generation: 1,
                        width: 640,
                        height: 360,
                        encode_submitted_at_ms: 100,
                        encoded_at_ms: 100,
                    },
                },
                &oversized_delta,
            )
            .is_err());
    }

    #[test]
    fn requested_keyframe_discards_in_flight_delta_until_recovery_idr() {
        let contract = contract(false);
        let fence = fence(&contract);
        let mut validator = activated_validator(contract, fence.clone());
        let idr = real_openh264_idr();
        let first = event(
            1,
            &fence,
            EventBody::VideoH264 {
                media_gate: 1,
                pts_90khz: 0,
                duration_90khz: 3_000,
                keyframe: true,
                sps_pps_present: true,
                discontinuity: true,
                codec_generation: 1,
                width: 640,
                height: 360,
                encode_submitted_at_ms: 90,
                encoded_at_ms: 99,
            },
        );
        validator.observe(MediaLane::Video, &first, &idr).unwrap();
        validator
            .register_command(&command(4, &fence, CommandBody::RequestKeyframe))
            .unwrap();

        let in_flight = event(
            2,
            &fence,
            EventBody::VideoH264 {
                media_gate: 1,
                pts_90khz: 3_000,
                duration_90khz: 3_000,
                keyframe: false,
                sps_pps_present: false,
                discontinuity: false,
                codec_generation: 1,
                width: 640,
                height: 360,
                encode_submitted_at_ms: 90,
                encoded_at_ms: 99,
            },
        );
        assert_eq!(
            validator
                .observe(MediaLane::Video, &in_flight, b"\0\0\0\x01\x41delta")
                .unwrap(),
            MediaObservation::StaleDiscarded
        );
        validator
            .observe(
                MediaLane::Control,
                &event(
                    3,
                    &fence,
                    EventBody::KeyframeRequested {
                        command_sequence: 4,
                    },
                ),
                &[],
            )
            .unwrap();

        let recovered = event(
            3,
            &fence,
            EventBody::VideoH264 {
                media_gate: 1,
                pts_90khz: 6_000,
                duration_90khz: 3_000,
                keyframe: true,
                sps_pps_present: true,
                discontinuity: false,
                codec_generation: 1,
                width: 640,
                height: 360,
                encode_submitted_at_ms: 90,
                encoded_at_ms: 99,
            },
        );
        assert_eq!(
            validator
                .observe(MediaLane::Video, &recovered, &idr)
                .unwrap(),
            MediaObservation::Accepted
        );
    }

    #[test]
    fn opus_packet_duration_is_validated_from_payload() {
        assert_eq!(opus_packet_samples_48khz(&[0x98, 0]).unwrap(), 960);
        assert_ne!(opus_packet_samples_48khz(&[0x80, 0]).unwrap(), 960);
    }

    #[test]
    fn command_framing_round_trip_is_bounded() {
        let contract = contract(false);
        let fence = fence(&contract);
        let command = command(1, &fence, CommandBody::StartPrepared { contract });
        let mut bytes = Vec::new();
        write_command_frame(&mut bytes, &command).unwrap();
        let decoded = read_command_frame(&mut Cursor::new(bytes))
            .unwrap()
            .unwrap();
        assert_eq!(decoded, command);
    }

    fn activated_validator(
        contract: StartContract,
        fence: GenerationFence,
    ) -> MediaConversationValidator {
        let mut validator = MediaConversationValidator::new(fence.clone()).unwrap();
        validator
            .register_command(&command(
                1,
                &fence,
                CommandBody::StartPrepared {
                    contract: contract.clone(),
                },
            ))
            .unwrap();
        validator
            .observe(
                MediaLane::Control,
                &event(
                    1,
                    &fence,
                    EventBody::Prepared {
                        command_sequence: 1,
                        capture_proof: proof(&contract.target),
                    },
                ),
                &[],
            )
            .unwrap();
        validator
            .register_command(&command(2, &fence, CommandBody::Activate))
            .unwrap();
        validator
            .observe(
                MediaLane::Control,
                &event(
                    2,
                    &fence,
                    EventBody::Activated {
                        command_sequence: 2,
                    },
                ),
                &[],
            )
            .unwrap();
        validator
            .register_command(&command(
                3,
                &fence,
                CommandBody::BeginMedia {
                    activation_command_sequence: 2,
                },
            ))
            .unwrap();
        validator
    }
}

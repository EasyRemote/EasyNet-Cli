//! EasyNet RemoteApp native media host
//! ===================================
//!
//! File: plugins/remote-desktop/media-host/src/lib.rs
//! Description: Canonical plugin-private process for capability inspection and
//! one immutable native media generation.
//!
//! Protocol Responsibility:
//! - Enforce `remoteapp_media_host_v1` fencing and lifecycle locally.
//! - Emit bounded control, Annex-B video, and Opus audio frames on independent
//!   physical lanes.
//!
//! Implementation Approach:
//! - Select capability or session mode from one strictly validated first frame.
//! - Run a host-side command validator and a mirrored conversation validator;
//!   any mismatch retires the process generation.
//! - Keep platform capture/encode behind one `SessionBackend` lifecycle.
//!
//! Usage Contract:
//! - The daemon supplies the exact executable digest and generation fence.
//! - The host receives no URA, authority, consent, receipt, WebRTC, or relay
//!   state.
//!
//! Architectural Position:
//! - RemoteDesktop PluginAbilityImpl native execution below daemon-owned
//!   session/WebRTC policy and above platform capture/codec APIs.

use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(all(feature = "native-media", target_os = "macos"))]
use base64::Engine;
#[cfg(all(feature = "native-media", target_os = "macos"))]
use easynet_remoteapp_native_protocol::capture_probe::Operation as CaptureProbeOperation;
use easynet_remoteapp_native_protocol::capture_probe::{
    Outcome as CaptureProbeOutcome, Request as CaptureProbeRequest,
    Response as CaptureProbeResponse,
};
use easynet_remoteapp_native_protocol::media_capabilities::{
    HostAudioCapability, Request, Response, SourceReadiness,
};
use easynet_remoteapp_native_protocol::media_session::{
    generation_nonce_bytes, read_command_frame, read_initial_frame, write_event_frame,
    BinaryMediaEvent, CaptureProof, Command, CommandBody, EventBody, EventMetadata, FailureReason,
    GenerationFence, InitialFrame, MediaConversationValidator, MediaHostCommandValidator,
    MediaLane, MediaStats, StartContract, VideoConfig, PROTOCOL as MEDIA_PROTOCOL,
    SCHEMA_VERSION as MEDIA_SCHEMA_VERSION,
};
#[cfg(unix)]
use easynet_remoteapp_native_protocol::media_session::{AUDIO_LANE_FD_ENV, VIDEO_LANE_FD_ENV};
use easynet_remoteapp_native_protocol::screen_capture_permission::{
    Operation as ScreenCapturePermissionOperation, Request as ScreenCapturePermissionRequest,
    Response as ScreenCapturePermissionResponse,
};
#[cfg(windows)]
use easynet_remoteapp_native_protocol::shared_media_lane::{
    open_windows_notification_writer, AUDIO_NOTIFICATION_PIPE_NAME_ENV, AUDIO_SHARED_LANE_NAME_ENV,
    VIDEO_NOTIFICATION_PIPE_NAME_ENV, VIDEO_SHARED_LANE_NAME_ENV,
};
#[cfg(any(unix, windows))]
use easynet_remoteapp_native_protocol::shared_media_lane::{
    SharedMediaLaneProducer, SharedSlotNotification,
};
#[cfg(unix)]
use easynet_remoteapp_native_protocol::shared_media_lane::{
    AUDIO_SHARED_LANE_FD_ENV, VIDEO_SHARED_LANE_FD_ENV,
};
#[cfg(unix)]
use easynet_remoteapp_native_protocol::PARENT_LIVENESS_FD_ENV;
use easynet_remoteapp_native_protocol::{write_frame, FrameError};
use sha2::{Digest, Sha256};

#[cfg(target_os = "macos")]
#[doc(hidden)]
pub mod macos_videotoolbox;

#[cfg(all(feature = "native-media", target_os = "macos"))]
mod macos_audio;
#[cfg(all(feature = "native-media", target_os = "macos"))]
mod macos_multiapp;
#[cfg(all(feature = "native-media", target_os = "macos"))]
mod macos_sck;
#[cfg(all(feature = "native-media", target_os = "macos"))]
use macos_sck::MacOsScreenCaptureKitSessionBackend as PlatformSessionBackend;

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows")
))]
#[path = "linux_x11.rs"]
mod xcap_openh264;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows")
))]
use xcap_openh264::XcapOpenH264SessionBackend as PlatformSessionBackend;

#[cfg(feature = "remoteapp-e2e-fault-injection")]
const TEST_FAULT_ENV: &str = "EASYNET_REMOTEAPP_MEDIA_HOST_TEST_FAULT";
const COMMAND_QUEUE_DEPTH: usize = 8;
const BACKEND_POLL_INTERVAL: Duration = Duration::from_millis(5);

#[cfg(not(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows", target_os = "macos")
)))]
const REASON_NATIVE_MEDIA_DISABLED: &str = "native_media_disabled";
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows")
))]
const REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE: &str =
    "active_media_session_audio_unavailable";
#[cfg(all(feature = "native-media", target_os = "macos"))]
const REASON_PROCESS_TREE_AUDIO_FILTER_UNVERIFIED: &str = "process_tree_audio_filter_unverified";

pub fn run() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    return run_macos_application();
    #[cfg(not(target_os = "macos"))]
    run_protocol()
}

fn run_protocol() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let parent_liveness_fd = start_parent_liveness_watchdog()?;
        let mut reader = duplicate_stdin()?;
        let initial = read_initial_frame(&mut reader)
            .map_err(|error| anyhow::anyhow!("read media-host initial frame: {error}"))?
            .ok_or_else(|| anyhow::anyhow!("media-host initial frame missing"))?;
        #[cfg(feature = "remoteapp-e2e-fault-injection")]
        apply_fault_injection()?;
        let stdout = io::stdout();
        return match initial {
            InitialFrame::CaptureProbe(request) => {
                run_capture_probe_request(request, stdout.lock())
            }
            InitialFrame::Capability(request) => run_capability_request(request, stdout.lock()),
            InitialFrame::ScreenCapturePermission(request) => {
                run_screen_capture_permission_request(request, stdout.lock())
            }
            InitialFrame::Session(command) => {
                let video = take_inherited_lane(VIDEO_LANE_FD_ENV, &[parent_liveness_fd])?;
                let audio =
                    take_inherited_lane(AUDIO_LANE_FD_ENV, &[parent_liveness_fd, video.raw_fd])?;
                let video_shared = take_inherited_lane(
                    VIDEO_SHARED_LANE_FD_ENV,
                    &[parent_liveness_fd, video.raw_fd, audio.raw_fd],
                )?;
                let audio_shared = take_inherited_lane(
                    AUDIO_SHARED_LANE_FD_ENV,
                    &[
                        parent_liveness_fd,
                        video.raw_fd,
                        audio.raw_fd,
                        video_shared.raw_fd,
                    ],
                )?;
                let generation_nonce = generation_nonce_bytes(&command.fence)?;
                let video_output = SharedMediaOutput {
                    lane: MediaLane::Video,
                    notification: video.file,
                    producer: SharedMediaLaneProducer::open(
                        &video_shared.file,
                        MediaLane::Video,
                        generation_nonce,
                    )?,
                };
                let audio_output = SharedMediaOutput {
                    lane: MediaLane::Audio,
                    notification: audio.file,
                    producer: SharedMediaLaneProducer::open(
                        &audio_shared.file,
                        MediaLane::Audio,
                        generation_nonce,
                    )?,
                };
                let build_id = executable_build_id()?;
                run_session(
                    command,
                    reader,
                    stdout.lock(),
                    video_output,
                    audio_output,
                    PlatformSessionBackend::default(),
                    &build_id,
                )
            }
        };
    }
    #[cfg(windows)]
    {
        let mut reader = duplicate_stdin()?;
        let stdout = io::stdout();
        let initial = read_initial_frame(&mut reader)
            .map_err(|error| anyhow::anyhow!("read media-host initial frame: {error}"))?
            .ok_or_else(|| anyhow::anyhow!("media-host initial frame missing"))?;
        #[cfg(feature = "remoteapp-e2e-fault-injection")]
        apply_fault_injection()?;
        match initial {
            InitialFrame::CaptureProbe(request) => {
                run_capture_probe_request(request, stdout.lock())
            }
            InitialFrame::Capability(request) => run_capability_request(request, stdout.lock()),
            InitialFrame::ScreenCapturePermission(request) => {
                run_screen_capture_permission_request(request, stdout.lock())
            }
            InitialFrame::Session(command) => {
                let generation_nonce = generation_nonce_bytes(&command.fence)?;
                let video_mapping = required_environment(VIDEO_SHARED_LANE_NAME_ENV)?;
                let audio_mapping = required_environment(AUDIO_SHARED_LANE_NAME_ENV)?;
                let video_notification = open_windows_notification_writer(&required_environment(
                    VIDEO_NOTIFICATION_PIPE_NAME_ENV,
                )?)?;
                let audio_notification = open_windows_notification_writer(&required_environment(
                    AUDIO_NOTIFICATION_PIPE_NAME_ENV,
                )?)?;
                let video_output = SharedMediaOutput {
                    lane: MediaLane::Video,
                    notification: video_notification,
                    producer: SharedMediaLaneProducer::open_named(
                        &video_mapping,
                        MediaLane::Video,
                        generation_nonce,
                    )?,
                };
                let audio_output = SharedMediaOutput {
                    lane: MediaLane::Audio,
                    notification: audio_notification,
                    producer: SharedMediaLaneProducer::open_named(
                        &audio_mapping,
                        MediaLane::Audio,
                        generation_nonce,
                    )?,
                };
                let build_id = executable_build_id()?;
                run_session(
                    command,
                    reader,
                    stdout.lock(),
                    video_output,
                    audio_output,
                    PlatformSessionBackend::default(),
                    &build_id,
                )
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("RemoteApp media host is unavailable on this platform")
}

#[cfg(target_os = "macos")]
fn run_macos_application() -> anyhow::Result<()> {
    let _application = initialize_macos_application()?;

    let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("easynet-rd-media-protocol".into())
        .spawn(move || {
            let _ = result_tx.send(run_protocol());
        })?;
    let result = loop {
        match result_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(result) => break result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                pump_macos_application_events(0.01);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break Err(anyhow::anyhow!(
                    "RemoteApp media host protocol worker disconnected"
                ));
            }
        }
    };
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("RemoteApp media host protocol worker panicked"))?;
    result
}

#[cfg(target_os = "macos")]
fn initialize_macos_application(
) -> anyhow::Result<objc2::rc::Retained<objc2_app_kit::NSApplication>> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let main_thread = MainThreadMarker::new().ok_or_else(|| {
        anyhow::anyhow!("RemoteApp media host must initialize AppKit on the main thread")
    })?;
    let application = NSApplication::sharedApplication(main_thread);
    application.finishLaunching();
    Ok(application)
}

#[cfg(target_os = "macos")]
fn pump_macos_application_events(seconds: f64) {
    use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop};

    let mode = unsafe { kCFRunLoopDefaultMode };
    let _ = CFRunLoop::run_in_mode(mode, seconds, true);
}

#[cfg(target_os = "macos")]
pub fn run_screen_capture_permission_application() -> anyhow::Result<()> {
    let _application = initialize_macos_application()?;
    if platform_screen_capture_permission_granted() {
        return Ok(());
    }

    let _ = platform_request_screen_capture_permission();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline && !platform_screen_capture_permission_granted() {
        pump_macos_application_events(0.05);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn run_launch_services_bootstrap(socket: std::ffi::OsString) -> anyhow::Result<()> {
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;

    use easynet_remoteapp_native_protocol::macos_launch_services::{
        receive_file_descriptors, FILE_DESCRIPTOR_COUNT,
    };

    let socket = std::path::PathBuf::from(socket);
    anyhow::ensure!(
        socket.is_absolute(),
        "LaunchServices bootstrap socket must be absolute"
    );
    let stream = UnixStream::connect(&socket)
        .map_err(|error| anyhow::anyhow!("connect LaunchServices bootstrap: {error}"))?;
    let descriptors = receive_file_descriptors(&stream)
        .map_err(|error| anyhow::anyhow!("receive LaunchServices bootstrap: {error}"))?;
    anyhow::ensure!(
        descriptors.len() == FILE_DESCRIPTOR_COUNT,
        "LaunchServices bootstrap descriptor count mismatch"
    );
    for (descriptor, target) in descriptors.iter().take(3).zip([0, 1, 2]) {
        if unsafe { libc::dup2(descriptor.as_raw_fd(), target) } == -1 {
            return Err(anyhow::anyhow!(
                "install LaunchServices stdio descriptor: {}",
                io::Error::last_os_error()
            ));
        }
    }
    for (name, descriptor) in [
        (PARENT_LIVENESS_FD_ENV, &descriptors[3]),
        (VIDEO_LANE_FD_ENV, &descriptors[4]),
        (AUDIO_LANE_FD_ENV, &descriptors[5]),
        (VIDEO_SHARED_LANE_FD_ENV, &descriptors[6]),
        (AUDIO_SHARED_LANE_FD_ENV, &descriptors[7]),
    ] {
        std::env::set_var(name, descriptor.as_raw_fd().to_string());
    }
    run()
}

fn run_capture_probe_request(
    request: CaptureProbeRequest,
    mut writer: impl Write,
) -> anyhow::Result<()> {
    request.validate()?;
    let started_at_ms = now_ms();
    #[cfg(all(feature = "native-media", target_os = "macos"))]
    let outcome = match &request.operation {
        CaptureProbeOperation::VerifyTarget => match macos_sck::probe_target(&request.target) {
            Ok(capture_proof) => CaptureProbeOutcome::Verified { capture_proof },
            Err(failure) => CaptureProbeOutcome::Failed {
                reason: failure.reason,
                detail: failure.detail,
            },
        },
        CaptureProbeOperation::DiagnosticJpeg { width, height } => {
            match macos_sck::capture_diagnostic_jpeg(&request.target, *width, *height) {
                Ok(frame) => CaptureProbeOutcome::DiagnosticJpeg {
                    capture_proof: frame.capture_proof,
                    width: frame.width,
                    height: frame.height,
                    jpeg_base64: base64::engine::general_purpose::STANDARD.encode(frame.jpeg),
                },
                Err(failure) => CaptureProbeOutcome::Failed {
                    reason: failure.reason,
                    detail: failure.detail,
                },
            }
        }
    };
    #[cfg(not(all(feature = "native-media", target_os = "macos")))]
    let outcome = CaptureProbeOutcome::Failed {
        reason: FailureReason::CaptureUnavailable,
        detail: "capture-probe mode is not linked for this platform".into(),
    };
    let response = CaptureProbeResponse::new(
        &request,
        started_at_ms,
        now_ms().max(started_at_ms),
        outcome,
    );
    response.validate_for(&request)?;
    write_frame(&mut writer, &response)
        .map_err(|error| anyhow::anyhow!("write media-host capture-probe response: {error}"))?;
    Ok(())
}

fn run_capability_request(request: Request, mut writer: impl Write) -> anyhow::Result<()> {
    request.validate()?;
    let started_at_ms = now_ms();
    let capability = platform_host_audio_capability();
    capability.validate()?;
    let response = Response::capabilities(
        &request,
        started_at_ms,
        now_ms().max(started_at_ms),
        capability,
    );
    write_frame(&mut writer, &response)
        .map_err(|error| anyhow::anyhow!("write media-host capability response: {error}"))?;
    Ok(())
}

fn run_screen_capture_permission_request(
    request: ScreenCapturePermissionRequest,
    mut writer: impl Write,
) -> anyhow::Result<()> {
    request.validate()?;
    let started_at_ms = now_ms();
    let requestable = cfg!(target_os = "macos");
    let previously_granted = platform_screen_capture_permission_granted();
    let requested = request.operation == ScreenCapturePermissionOperation::Request
        && requestable
        && !previously_granted;
    let granted = previously_granted || (requested && platform_request_screen_capture_permission());
    let response = ScreenCapturePermissionResponse::permission_result(
        &request,
        started_at_ms,
        now_ms().max(started_at_ms),
        platform_screen_capture_backend(),
        requestable,
        previously_granted,
        requested,
        granted,
        std::env::current_exe()
            .ok()
            .map(|path| path.display().to_string()),
    );
    write_frame(&mut writer, &response)
        .map_err(|error| anyhow::anyhow!("write media-host permission response: {error}"))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn platform_screen_capture_permission_granted() -> bool {
    unsafe { macos_screen_capture_tcc::preflight_screen_capture_access() }
}

#[cfg(target_os = "macos")]
fn platform_request_screen_capture_permission() -> bool {
    unsafe { macos_screen_capture_tcc::request_screen_capture_access() }
}

#[cfg(not(target_os = "macos"))]
fn platform_screen_capture_permission_granted() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
fn platform_request_screen_capture_permission() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn platform_screen_capture_backend() -> &'static str {
    "screencapturekit"
}

#[cfg(target_os = "windows")]
fn platform_screen_capture_backend() -> &'static str {
    "windows_graphics_capture"
}

#[cfg(target_os = "linux")]
fn platform_screen_capture_backend() -> &'static str {
    "x11_or_portal"
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_screen_capture_backend() -> &'static str {
    "unavailable"
}

#[cfg(target_os = "macos")]
mod macos_screen_capture_tcc {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    pub(super) unsafe fn preflight_screen_capture_access() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub(super) unsafe fn request_screen_capture_access() -> bool {
        unsafe { CGRequestScreenCaptureAccess() }
    }
}

#[derive(Debug)]
struct BackendFailure {
    reason: FailureReason,
    detail: String,
}

impl BackendFailure {
    fn new(reason: FailureReason, detail: impl Into<String>) -> Self {
        let mut detail = detail.into().replace('\0', "");
        if detail.len() > 2_048 {
            let mut boundary = 2_048;
            while !detail.is_char_boundary(boundary) {
                boundary -= 1;
            }
            detail.truncate(boundary);
        }
        if detail.is_empty() {
            detail = "native media operation failed without diagnostic detail".into();
        }
        Self { reason, detail }
    }
}

trait SessionBackend {
    fn prepare(&mut self, contract: &StartContract) -> Result<CaptureProof, BackendFailure>;
    fn activate(&mut self) -> Result<(), BackendFailure>;
    fn begin_media(&mut self, media_gate: u32) -> Result<(), BackendFailure>;
    fn reconfigure(
        &mut self,
        video: &VideoConfig,
        force_keyframe: bool,
    ) -> Result<(), BackendFailure>;
    fn resume_media(&mut self, media_gate: u32) -> Result<(), BackendFailure>;
    fn request_keyframe(&mut self) -> Result<(), BackendFailure>;
    fn poll(&mut self, timeout: Duration) -> Result<Option<BackendEvent>, BackendFailure>;
    fn stop(&mut self) -> Result<(), BackendFailure>;
}

/// Deliberately fail closed until a real platform adapter is linked. The daemon
/// does not select session mode until the corresponding adapter migration is
/// complete and proven; capability mode remains fully functional meanwhile.
#[cfg(not(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
)))]
#[derive(Default)]
struct PlatformSessionBackend;

#[cfg(not(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
)))]
impl SessionBackend for PlatformSessionBackend {
    fn prepare(&mut self, _: &StartContract) -> Result<CaptureProof, BackendFailure> {
        Err(BackendFailure::new(
            FailureReason::CaptureUnavailable,
            "active platform media adapter is not linked into the canonical media host",
        ))
    }

    fn activate(&mut self) -> Result<(), BackendFailure> {
        unreachable!("an unavailable backend cannot activate")
    }

    fn begin_media(&mut self, _: u32) -> Result<(), BackendFailure> {
        unreachable!("an unavailable backend cannot begin media")
    }

    fn reconfigure(&mut self, _: &VideoConfig, _: bool) -> Result<(), BackendFailure> {
        unreachable!("an unavailable backend cannot reconfigure")
    }

    fn resume_media(&mut self, _: u32) -> Result<(), BackendFailure> {
        unreachable!("an unavailable backend cannot resume media")
    }

    fn request_keyframe(&mut self) -> Result<(), BackendFailure> {
        unreachable!("an unavailable backend cannot request keyframes")
    }

    fn poll(&mut self, _: Duration) -> Result<Option<BackendEvent>, BackendFailure> {
        unreachable!("an unavailable backend cannot poll")
    }

    fn stop(&mut self) -> Result<(), BackendFailure> {
        unreachable!("an unavailable backend cannot stop")
    }
}

#[allow(dead_code)] // Platform variants are compiled only as each real adapter lands.
enum BackendEvent {
    Video { body: EventBody, payload: Vec<u8> },
    Audio { body: EventBody, payload: Vec<u8> },
    Stats(MediaStats),
}

enum CommandInput {
    Command(Command),
    Eof,
    Error(FrameError),
}

struct SessionCoordinator<Control, Video, Audio> {
    fence: GenerationFence,
    host: MediaHostCommandValidator,
    conversation: MediaConversationValidator,
    control: Control,
    video: Video,
    audio: Audio,
    control_sequence: u64,
    video_sequence: u64,
    audio_sequence: u64,
    codec_generation: u32,
}

trait MediaEventOutput {
    fn emit_event(
        &mut self,
        lane: MediaLane,
        fence: &GenerationFence,
        sequence: u64,
        observed_at_ms: u64,
        body: &EventBody,
        payload: &[u8],
    ) -> anyhow::Result<()>;
}

impl<Writer: Write> MediaEventOutput for Writer {
    fn emit_event(
        &mut self,
        lane: MediaLane,
        fence: &GenerationFence,
        sequence: u64,
        observed_at_ms: u64,
        body: &EventBody,
        payload: &[u8],
    ) -> anyhow::Result<()> {
        let metadata = EventMetadata {
            schema_version: MEDIA_SCHEMA_VERSION,
            protocol: MEDIA_PROTOCOL.to_string(),
            fence: fence.clone(),
            sequence,
            observed_at_ms,
            body: body.clone(),
        };
        write_event_frame(self, lane, &metadata, payload)?;
        Ok(())
    }
}

#[cfg(any(unix, windows))]
struct SharedMediaOutput {
    lane: MediaLane,
    notification: std::fs::File,
    producer: SharedMediaLaneProducer,
}

#[cfg(any(unix, windows))]
impl MediaEventOutput for SharedMediaOutput {
    fn emit_event(
        &mut self,
        lane: MediaLane,
        _fence: &GenerationFence,
        sequence: u64,
        observed_at_ms: u64,
        body: &EventBody,
        payload: &[u8],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(lane == self.lane, "shared media output lane mismatch");
        let outcome = self
            .producer
            .publish_media_event(sequence, observed_at_ms, body, payload)?;
        SharedSlotNotification::from(outcome).write_to(&mut self.notification, lane)?;
        Ok(())
    }
}

impl<Control: Write, Video: MediaEventOutput, Audio: MediaEventOutput>
    SessionCoordinator<Control, Video, Audio>
{
    fn new(
        fence: GenerationFence,
        control: Control,
        video: Video,
        audio: Audio,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            host: MediaHostCommandValidator::new(fence.clone())?,
            conversation: MediaConversationValidator::new(fence.clone())?,
            fence,
            control,
            video,
            audio,
            control_sequence: 0,
            video_sequence: 0,
            audio_sequence: 0,
            codec_generation: 1,
        })
    }

    fn register(&mut self, command: &Command) -> anyhow::Result<()> {
        self.conversation.register_command(command)?;
        self.host.observe(command)?;
        Ok(())
    }

    fn emit(&mut self, lane: MediaLane, body: EventBody, payload: &[u8]) -> anyhow::Result<()> {
        let sequence = match lane {
            MediaLane::Control => &mut self.control_sequence,
            MediaLane::Video => &mut self.video_sequence,
            MediaLane::Audio => &mut self.audio_sequence,
        };
        *sequence = sequence
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("media-host event sequence overflow"))?;
        let sequence = *sequence;
        let observed_at_ms = now_ms();
        match lane {
            MediaLane::Control => {
                let metadata = EventMetadata {
                    schema_version: MEDIA_SCHEMA_VERSION,
                    protocol: MEDIA_PROTOCOL.to_string(),
                    fence: self.fence.clone(),
                    sequence,
                    observed_at_ms,
                    body,
                };
                self.conversation.observe(lane, &metadata, payload)?;
                write_event_frame(&mut self.control, lane, &metadata, payload)?;
            }
            MediaLane::Video | MediaLane::Audio => {
                let metadata = BinaryMediaEvent {
                    sequence,
                    observed_at_ms,
                    body,
                };
                self.conversation
                    .observe_binary_media(lane, &metadata, payload)?;
                match lane {
                    MediaLane::Video => self.video.emit_event(
                        lane,
                        &self.fence,
                        sequence,
                        observed_at_ms,
                        &metadata.body,
                        payload,
                    )?,
                    MediaLane::Audio => self.audio.emit_event(
                        lane,
                        &self.fence,
                        sequence,
                        observed_at_ms,
                        &metadata.body,
                        payload,
                    )?,
                    MediaLane::Control => unreachable!(),
                }
            }
        }
        Ok(())
    }

    fn fail(&mut self, failure: BackendFailure) -> anyhow::Result<()> {
        self.host.mark_failed()?;
        self.emit(
            MediaLane::Control,
            EventBody::Failed {
                reason: failure.reason,
                detail: failure.detail,
            },
            &[],
        )
    }
}

fn run_session<Reader, Control, Video, Audio, Backend>(
    initial: Command,
    reader: Reader,
    control: Control,
    video: Video,
    audio: Audio,
    mut backend: Backend,
    executable_build_id: &str,
) -> anyhow::Result<()>
where
    Reader: Read + Send + 'static,
    Control: Write,
    Video: MediaEventOutput,
    Audio: MediaEventOutput,
    Backend: SessionBackend,
{
    anyhow::ensure!(
        initial.fence.build_id == executable_build_id,
        "media-host executable build digest does not match start fence"
    );
    let mut session = SessionCoordinator::new(initial.fence.clone(), control, video, audio)?;
    session.register(&initial)?;
    let contract = match &initial.body {
        CommandBody::StartPrepared { contract } => contract.clone(),
        _ => anyhow::bail!("media-host session initial command is not start_prepared"),
    };
    let capture_proof = match backend.prepare(&contract) {
        Ok(proof) => proof,
        Err(failure) => {
            session.fail(failure)?;
            return Ok(());
        }
    };
    session.host.mark_prepared(initial.sequence)?;
    session.emit(
        MediaLane::Control,
        EventBody::Prepared {
            command_sequence: initial.sequence,
            capture_proof,
        },
        &[],
    )?;

    let commands = spawn_command_reader(reader)?;
    loop {
        match commands.try_recv() {
            Ok(CommandInput::Command(command)) => {
                if handle_command(&mut session, &mut backend, command)? {
                    return Ok(());
                }
                continue;
            }
            Ok(CommandInput::Eof) => {
                session.fail(BackendFailure::new(
                    FailureReason::ProtocolViolation,
                    "media-host command lane reached EOF before stop",
                ))?;
                return Ok(());
            }
            Ok(CommandInput::Error(error)) => {
                session.fail(BackendFailure::new(
                    FailureReason::ProtocolViolation,
                    format!("media-host command framing failed: {error}"),
                ))?;
                return Ok(());
            }
            Err(TryRecvError::Disconnected) => {
                session.fail(BackendFailure::new(
                    FailureReason::ProtocolViolation,
                    "media-host command reader terminated",
                ))?;
                return Ok(());
            }
            Err(TryRecvError::Empty) => {}
        }

        match backend.poll(BACKEND_POLL_INTERVAL) {
            Ok(Some(event)) => emit_backend_event(&mut session, event)?,
            Ok(None) => {}
            Err(failure) => {
                session.fail(failure)?;
                return Ok(());
            }
        }
    }
}

fn handle_command<Control, Video, Audio, Backend>(
    session: &mut SessionCoordinator<Control, Video, Audio>,
    backend: &mut Backend,
    command: Command,
) -> anyhow::Result<bool>
where
    Control: Write,
    Video: MediaEventOutput,
    Audio: MediaEventOutput,
    Backend: SessionBackend,
{
    session.register(&command)?;
    match &command.body {
        CommandBody::Activate => {
            if let Err(failure) = backend.activate() {
                session.fail(failure)?;
                return Ok(true);
            }
            session.host.mark_activated(command.sequence)?;
            session.emit(
                MediaLane::Control,
                EventBody::Activated {
                    command_sequence: command.sequence,
                },
                &[],
            )?;
        }
        CommandBody::BeginMedia { .. } => {
            if let Err(failure) = backend.begin_media(session.host.media_gate()) {
                session.fail(failure)?;
                return Ok(true);
            }
        }
        CommandBody::Reconfigure {
            video,
            force_keyframe,
        } => {
            if let Err(failure) = backend.reconfigure(video, *force_keyframe) {
                session.fail(failure)?;
                return Ok(true);
            }
            session.codec_generation = session
                .codec_generation
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("media-host codec generation overflow"))?;
            session.host.mark_reconfigured(command.sequence)?;
            session.emit(
                MediaLane::Control,
                EventBody::Reconfigured {
                    command_sequence: command.sequence,
                    video: video.clone(),
                    codec_generation: session.codec_generation,
                },
                &[],
            )?;
        }
        CommandBody::ResumeMedia { .. } => {
            if let Err(failure) = backend.resume_media(session.host.media_gate()) {
                session.fail(failure)?;
                return Ok(true);
            }
        }
        CommandBody::RequestKeyframe => {
            if let Err(failure) = backend.request_keyframe() {
                session.fail(failure)?;
                return Ok(true);
            }
            session.emit(
                MediaLane::Control,
                EventBody::KeyframeRequested {
                    command_sequence: command.sequence,
                },
                &[],
            )?;
        }
        CommandBody::Stop => {
            if let Err(failure) = backend.stop() {
                session.fail(failure)?;
                return Ok(true);
            }
            session.host.mark_stopped(command.sequence)?;
            session.emit(
                MediaLane::Control,
                EventBody::Stopped {
                    command_sequence: command.sequence,
                },
                &[],
            )?;
            return Ok(true);
        }
        CommandBody::StartPrepared { .. } => {
            anyhow::bail!("media-host received a second start command")
        }
    }
    Ok(false)
}

fn emit_backend_event<Control: Write, Video: MediaEventOutput, Audio: MediaEventOutput>(
    session: &mut SessionCoordinator<Control, Video, Audio>,
    event: BackendEvent,
) -> anyhow::Result<()> {
    match event {
        BackendEvent::Video { body, payload } => session.emit(MediaLane::Video, body, &payload),
        BackendEvent::Audio { body, payload } => session.emit(MediaLane::Audio, body, &payload),
        BackendEvent::Stats(stats) => {
            session.emit(MediaLane::Control, EventBody::Stats { stats }, &[])
        }
    }
}

fn spawn_command_reader<Reader: Read + Send + 'static>(
    mut reader: Reader,
) -> io::Result<Receiver<CommandInput>> {
    let (sender, receiver) = mpsc::sync_channel(COMMAND_QUEUE_DEPTH);
    thread::Builder::new()
        .name("easynet-rd-media-host-command".into())
        .spawn(move || loop {
            let input = match read_command_frame(&mut reader) {
                Ok(Some(command)) => CommandInput::Command(command),
                Ok(None) => CommandInput::Eof,
                Err(error) => CommandInput::Error(error),
            };
            let terminal = !matches!(input, CommandInput::Command(_));
            if sender.send(input).is_err() || terminal {
                return;
            }
        })?;
    Ok(receiver)
}

fn executable_build_id() -> anyhow::Result<String> {
    let path =
        std::env::current_exe().map_err(|error| anyhow::anyhow!("locate media host: {error}"))?;
    let mut file = std::fs::File::open(&path)
        .map_err(|error| anyhow::anyhow!("open media host for build digest: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| anyhow::anyhow!("hash media host build: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(lower_hex(&digest.finalize()))
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

#[cfg(unix)]
struct InheritedLane {
    raw_fd: i32,
    file: std::fs::File,
}

#[cfg(unix)]
fn duplicate_stdin() -> anyhow::Result<std::fs::File> {
    use std::os::fd::FromRawFd;

    let fd = unsafe { libc::dup(libc::STDIN_FILENO) };
    if fd < 0 {
        return Err(anyhow::anyhow!(
            "duplicate media-host command lane: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

#[cfg(windows)]
fn duplicate_stdin() -> anyhow::Result<std::fs::File> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{
        DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let source = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if source.is_null() || source == INVALID_HANDLE_VALUE {
        return Err(anyhow::anyhow!(
            "media-host command lane is not an open Windows stdin handle"
        ));
    }
    let process = unsafe { GetCurrentProcess() };
    let mut duplicate: HANDLE = std::ptr::null_mut();
    if unsafe {
        DuplicateHandle(
            process,
            source,
            process,
            &mut duplicate,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(anyhow::anyhow!(
            "duplicate media-host command lane: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(unsafe { std::fs::File::from_raw_handle(duplicate.cast()) })
}

#[cfg(windows)]
fn required_environment(name: &str) -> anyhow::Result<String> {
    let value =
        std::env::var(name).map_err(|_| anyhow::anyhow!("media-host session missing {name}"))?;
    anyhow::ensure!(
        !value.trim().is_empty() && !value.contains('\0'),
        "media-host session has an invalid {name}"
    );
    Ok(value)
}

#[cfg(unix)]
fn take_inherited_lane(name: &str, reserved: &[i32]) -> anyhow::Result<InheritedLane> {
    use std::os::fd::FromRawFd;

    let fd = std::env::var(name)
        .map_err(|_| anyhow::anyhow!("media-host session missing {name}"))?
        .parse::<i32>()
        .map_err(|error| anyhow::anyhow!("invalid media-host lane descriptor {name}: {error}"))?;
    anyhow::ensure!(fd > libc::STDERR_FILENO, "{name} must not alias stdio");
    anyhow::ensure!(!reserved.contains(&fd), "{name} aliases another host lane");
    if unsafe { libc::fcntl(fd, libc::F_GETFD) } == -1 {
        return Err(anyhow::anyhow!(
            "{name} is not an open descriptor: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(InheritedLane {
        raw_fd: fd,
        file: unsafe { std::fs::File::from_raw_fd(fd) },
    })
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
fn start_parent_liveness_watchdog() -> anyhow::Result<i32> {
    use std::os::fd::FromRawFd;

    let fd = std::env::var(PARENT_LIVENESS_FD_ENV)
        .map_err(|_| anyhow::anyhow!("RemoteApp media host missing parent liveness descriptor"))?
        .parse::<i32>()
        .map_err(|error| anyhow::anyhow!("invalid parent liveness descriptor: {error}"))?;
    anyhow::ensure!(fd >= 0, "parent liveness descriptor must be non-negative");
    let mut liveness = unsafe { std::fs::File::from_raw_fd(fd) };
    thread::Builder::new()
        .name("easynet-rd-media-host-parent-liveness".into())
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
    Ok(fd)
}

#[cfg(feature = "remoteapp-e2e-fault-injection")]
fn apply_fault_injection() -> anyhow::Result<()> {
    match std::env::var(TEST_FAULT_ENV).as_deref() {
        Ok("hang") => loop {
            thread::park();
        },
        Ok("crash") => anyhow::bail!("injected media-host crash"),
        Ok(value) => anyhow::bail!("unsupported media-host fault injection {value:?}"),
        Err(std::env::VarError::NotPresent) => Ok(()),
        Err(error) => Err(anyhow::anyhow!("read media-host fault injection: {error}")),
    }
}

#[cfg(all(feature = "native-media", target_os = "macos"))]
fn platform_host_audio_capability() -> HostAudioCapability {
    match macos_audio::OpusPacketizer::new() {
        Ok(_) => HostAudioCapability::new(
            true,
            true,
            None::<String>,
            SourceReadiness::ready(),
            SourceReadiness::blocked(REASON_PROCESS_TREE_AUDIO_FILTER_UNVERIFIED),
            Some(
                "ScreenCaptureKit filter-scoped audio is encoded as 48 kHz stereo Opus by the active media-host generation"
                    .to_string(),
            ),
        ),
        Err(error) => HostAudioCapability::new(
            true,
            false,
            Some("opus_encoder_unavailable"),
            SourceReadiness::blocked("opus_encoder_unavailable"),
            SourceReadiness::blocked("opus_encoder_unavailable"),
            Some(format!("initialize media-host Opus encoder: {error}")),
        ),
    }
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows")
))]
fn platform_host_audio_capability() -> HostAudioCapability {
    HostAudioCapability::new(
        false,
        false,
        Some(REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE),
        SourceReadiness::blocked(REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE),
        SourceReadiness::blocked(REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE),
        Some(format!(
            "{} media-host active session cannot emit validator-checked Opus yet; host audio must remain unnegotiated",
            std::env::consts::OS
        )),
    )
}

#[cfg(not(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows", target_os = "macos")
)))]
fn platform_host_audio_capability() -> HostAudioCapability {
    HostAudioCapability::new(
        false,
        false,
        Some(REASON_NATIVE_MEDIA_DISABLED),
        SourceReadiness::blocked(REASON_NATIVE_MEDIA_DISABLED),
        SourceReadiness::blocked(REASON_NATIVE_MEDIA_DISABLED),
        None,
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use easynet_remoteapp_native_protocol::media_session::{
        read_event_frame, write_command_frame, CaptureBackend, NativeTargetPlan, TargetKind,
        VideoCodec,
    };
    use easynet_remoteapp_native_protocol::read_frame;

    use super::*;

    #[test]
    fn one_request_returns_one_bound_response() {
        let request = Request::probe_capabilities(2, 4);
        let mut output = Vec::new();
        run_capability_request(request, &mut output).unwrap();
        let response: Response = read_frame(&mut Cursor::new(output)).unwrap().unwrap();
        assert!(response.matches_request(2, 4));
        #[cfg(all(feature = "native-media", target_os = "macos"))]
        {
            assert!(response.capability.compiled_supported);
            assert!(response.capability.runtime_reachable);
            assert_eq!(response.capability.runtime_blocker, None);
            assert!(response.capability.system_loopback.ready);
            assert!(!response.capability.process_tree_loopback.ready);
        }
        #[cfg(not(all(feature = "native-media", target_os = "macos")))]
        {
            assert!(!response.capability.compiled_supported);
            assert!(!response.capability.runtime_reachable);
            #[cfg(all(
                feature = "native-media",
                any(target_os = "linux", target_os = "windows")
            ))]
            assert_eq!(
                response.capability.runtime_blocker.as_deref(),
                Some(REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE)
            );
            assert!(!response.capability.system_loopback.ready);
            assert!(!response.capability.process_tree_loopback.ready);
        }
    }

    #[test]
    fn permission_status_is_bound_to_the_canonical_media_host_process() {
        let request =
            ScreenCapturePermissionRequest::new(3, 5, ScreenCapturePermissionOperation::Status);
        let mut output = Vec::new();
        run_screen_capture_permission_request(request.clone(), &mut output).unwrap();
        let response: ScreenCapturePermissionResponse =
            read_frame(&mut Cursor::new(output)).unwrap().unwrap();
        assert!(response.matches_request(&request));
        assert!(!response.requested);
        assert_eq!(response.previously_granted, response.granted);
        assert!(response
            .executable_path
            .as_deref()
            .is_some_and(|path| path.contains("easynet_remoteapp_media_host")));
    }

    struct TestBackend {
        proof: CaptureProof,
        stop_fails: bool,
    }

    impl SessionBackend for TestBackend {
        fn prepare(&mut self, _: &StartContract) -> Result<CaptureProof, BackendFailure> {
            Ok(self.proof.clone())
        }

        fn activate(&mut self) -> Result<(), BackendFailure> {
            Ok(())
        }

        fn begin_media(&mut self, _: u32) -> Result<(), BackendFailure> {
            Ok(())
        }

        fn reconfigure(&mut self, _: &VideoConfig, _: bool) -> Result<(), BackendFailure> {
            Ok(())
        }

        fn resume_media(&mut self, _: u32) -> Result<(), BackendFailure> {
            Ok(())
        }

        fn request_keyframe(&mut self) -> Result<(), BackendFailure> {
            Ok(())
        }

        fn poll(&mut self, timeout: Duration) -> Result<Option<BackendEvent>, BackendFailure> {
            thread::park_timeout(timeout);
            Ok(None)
        }

        fn stop(&mut self) -> Result<(), BackendFailure> {
            if self.stop_fails {
                Err(BackendFailure::new(
                    FailureReason::DeviceLost,
                    "injected backend stop failure",
                ))
            } else {
                Ok(())
            }
        }
    }

    fn test_contract() -> StartContract {
        StartContract {
            target: NativeTargetPlan {
                kind: TargetKind::Display,
                display_id: Some(1),
                window_id: None,
                pid: None,
                process_instance_id: None,
                app_identity: None,
                bundle_id: None,
                application: None,
            },
            video: VideoConfig {
                codec: VideoCodec::H264AnnexB,
                width: 640,
                height: 360,
                fps: 30,
                bitrate_kbps: 1_500,
                keyframe_interval_frames: 30,
                max_pending_frames: 3,
                max_access_unit_bytes: 1024 * 1024,
                max_nal_unit_bytes: 1_160,
                h264_profile_idc: 66,
                h264_level_idc: 31,
            },
            audio: None,
        }
    }

    fn command(fence: &GenerationFence, sequence: u64, body: CommandBody) -> Command {
        Command {
            schema_version: MEDIA_SCHEMA_VERSION,
            protocol: MEDIA_PROTOCOL.to_string(),
            fence: fence.clone(),
            sequence,
            body,
        }
    }

    #[test]
    fn one_generation_requires_prepare_activation_barrier_and_stop() {
        let contract = test_contract();
        let build_id = "a".repeat(64);
        let fence = test_fence(&contract, &build_id);
        let initial = command(
            &fence,
            1,
            CommandBody::StartPrepared {
                contract: contract.clone(),
            },
        );
        let mut input = Vec::new();
        write_command_frame(&mut input, &command(&fence, 2, CommandBody::Activate)).unwrap();
        write_command_frame(
            &mut input,
            &command(
                &fence,
                3,
                CommandBody::BeginMedia {
                    activation_command_sequence: 2,
                },
            ),
        )
        .unwrap();
        write_command_frame(&mut input, &command(&fence, 4, CommandBody::Stop)).unwrap();
        let mut control = Vec::new();
        let mut video = Vec::new();
        let mut audio = Vec::new();
        run_session(
            initial,
            Cursor::new(input),
            &mut control,
            &mut video,
            &mut audio,
            TestBackend {
                proof: test_proof(&contract),
                stop_fails: false,
            },
            &build_id,
        )
        .unwrap();

        let mut frames = Cursor::new(control);
        let prepared = read_event_frame(&mut frames, MediaLane::Control, None)
            .unwrap()
            .unwrap()
            .0;
        let activated = read_event_frame(&mut frames, MediaLane::Control, None)
            .unwrap()
            .unwrap()
            .0;
        let stopped = read_event_frame(&mut frames, MediaLane::Control, None)
            .unwrap()
            .unwrap()
            .0;
        assert!(matches!(prepared.body, EventBody::Prepared { .. }));
        assert!(matches!(activated.body, EventBody::Activated { .. }));
        assert!(matches!(stopped.body, EventBody::Stopped { .. }));
        assert!(read_event_frame(&mut frames, MediaLane::Control, None)
            .unwrap()
            .is_none());
        assert!(video.is_empty());
        assert!(audio.is_empty());
    }

    fn test_fence(contract: &StartContract, build_id: &str) -> GenerationFence {
        GenerationFence {
            process_generation: 1,
            build_id: build_id.to_string(),
            session_nonce: "b".repeat(32),
            transport_epoch: 2,
            media_source_epoch: 3,
            contract_digest: contract.digest().unwrap(),
        }
    }

    fn test_proof(contract: &StartContract) -> CaptureProof {
        CaptureProof {
            backend: CaptureBackend::XcapX11,
            observed_target: contract.target.clone(),
            native_width: 640,
            native_height: 360,
            verified_at_ms: now_ms(),
        }
    }

    #[test]
    fn command_eof_after_prepare_emits_one_protocol_failure() {
        let contract = test_contract();
        let build_id = "c".repeat(64);
        let fence = test_fence(&contract, &build_id);
        let initial = command(
            &fence,
            1,
            CommandBody::StartPrepared {
                contract: contract.clone(),
            },
        );
        let mut control = Vec::new();
        run_session(
            initial,
            Cursor::new(Vec::<u8>::new()),
            &mut control,
            Vec::new(),
            Vec::new(),
            TestBackend {
                proof: test_proof(&contract),
                stop_fails: false,
            },
            &build_id,
        )
        .unwrap();

        let mut frames = Cursor::new(control);
        let prepared = read_event_frame(&mut frames, MediaLane::Control, None)
            .unwrap()
            .unwrap()
            .0;
        let failed = read_event_frame(&mut frames, MediaLane::Control, None)
            .unwrap()
            .unwrap()
            .0;
        assert!(matches!(prepared.body, EventBody::Prepared { .. }));
        assert!(matches!(
            failed.body,
            EventBody::Failed {
                reason: FailureReason::ProtocolViolation,
                ..
            }
        ));
        assert!(read_event_frame(&mut frames, MediaLane::Control, None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn backend_stop_failure_replaces_stopped_with_one_typed_failure() {
        let contract = test_contract();
        let build_id = "d".repeat(64);
        let fence = test_fence(&contract, &build_id);
        let initial = command(
            &fence,
            1,
            CommandBody::StartPrepared {
                contract: contract.clone(),
            },
        );
        let mut input = Vec::new();
        write_command_frame(&mut input, &command(&fence, 2, CommandBody::Activate)).unwrap();
        write_command_frame(
            &mut input,
            &command(
                &fence,
                3,
                CommandBody::BeginMedia {
                    activation_command_sequence: 2,
                },
            ),
        )
        .unwrap();
        write_command_frame(&mut input, &command(&fence, 4, CommandBody::Stop)).unwrap();
        let mut control = Vec::new();
        run_session(
            initial,
            Cursor::new(input),
            &mut control,
            Vec::new(),
            Vec::new(),
            TestBackend {
                proof: test_proof(&contract),
                stop_fails: true,
            },
            &build_id,
        )
        .unwrap();

        let mut frames = Cursor::new(control);
        assert!(matches!(
            read_event_frame(&mut frames, MediaLane::Control, None)
                .unwrap()
                .unwrap()
                .0
                .body,
            EventBody::Prepared { .. }
        ));
        assert!(matches!(
            read_event_frame(&mut frames, MediaLane::Control, None)
                .unwrap()
                .unwrap()
                .0
                .body,
            EventBody::Activated { .. }
        ));
        assert!(matches!(
            read_event_frame(&mut frames, MediaLane::Control, None)
                .unwrap()
                .unwrap()
                .0
                .body,
            EventBody::Failed {
                reason: FailureReason::DeviceLost,
                ..
            }
        ));
        assert!(read_event_frame(&mut frames, MediaLane::Control, None)
            .unwrap()
            .is_none());
    }
}

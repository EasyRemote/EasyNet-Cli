//! Real-process Linux/X11 proof for the canonical RemoteApp media host.
//!
//! This ignored test is run only by the repository live-host gate. It launches
//! the packaged helper protocol shape against two real X11 windows owned by one
//! process and proves window/application capture, H264 recovery and orderly
//! terminal cleanup without importing daemon policy into the helper crate.

#![cfg(all(target_os = "linux", feature = "native-media"))]

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use bytes::Bytes;
use easynet_remoteapp_native_protocol::media_session::{
    binary_media_frame_capacity, decode_binary_media_event_frame_compact, generation_nonce_bytes,
    read_event_frame, write_command_frame, ApplicationSurface, ApplicationWindowSet,
    BinaryMediaEvent, Command, CommandBody, EventBody, EventMetadata, FailureReason,
    GenerationFence, MediaConversationValidator, MediaLane, MediaObservation, NativeTargetPlan,
    StartContract, TargetKind, VideoCodec, VideoConfig, AUDIO_LANE_FD_ENV, MAX_OPUS_PACKET_BYTES,
    PROTOCOL, SCHEMA_VERSION, VIDEO_LANE_FD_ENV,
};
use easynet_remoteapp_native_protocol::shared_media_lane::{
    SharedFrameIdentity, SharedMediaLaneConsumer, SharedMediaLaneError, SharedMediaLaneFile,
    SharedMediaLaneLayout, SharedSlotNotification, AUDIO_SHARED_LANE_FD_ENV,
    VIDEO_SHARED_LANE_FD_ENV,
};
use easynet_remoteapp_native_protocol::{FrameError, PARENT_LIVENESS_FD_ENV};
use sha2::{Digest, Sha256};

const EVENT_DEADLINE: Duration = Duration::from_secs(10);
const PROCESS_EXIT_DEADLINE: Duration = Duration::from_secs(3);
static NEXT_PROCESS_GENERATION: AtomicU64 = AtomicU64::new(1);

type ControlLaneMessage = Result<Option<(EventMetadata, Vec<u8>)>, FrameError>;
type MediaLaneMessage = Result<Option<MediaLaneEvent>, FrameError>;

enum MediaLaneEvent {
    Frame(BinaryMediaEvent, Bytes),
    Dropped(SharedFrameIdentity),
}

struct Sentinel {
    child: Child,
    state: PathBuf,
}

impl Drop for Sentinel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.state);
        let _ = std::fs::remove_file(format!("{}.command.json", self.state.display()));
    }
}

impl Sentinel {
    fn move_secondary(&self, x: i64, y: i64) -> anyhow::Result<()> {
        let command_path = PathBuf::from(format!("{}.command.json", self.state.display()));
        std::fs::write(
            command_path,
            format!(
                "{{\"command_id\":\"media-host-invalidate-{}\",\"action\":\"move\",\"surface\":\"B\",\"x\":{x},\"y\":{y}}}",
                NEXT_PROCESS_GENERATION.load(Ordering::Relaxed)
            ),
        )?;
        Ok(())
    }
}

#[derive(Clone)]
struct X11WindowProof {
    id: u64,
    x: i64,
    y: i64,
    width: u32,
    height: u32,
    app_identity: String,
}

struct MediaHostHarness {
    child: Child,
    stdin: Option<ChildStdin>,
    _parent_liveness: File,
    control: Receiver<ControlLaneMessage>,
    video: Receiver<MediaLaneMessage>,
    audio: Receiver<MediaLaneMessage>,
    readers: Vec<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<Vec<u8>>>,
    validator: MediaConversationValidator,
    fence: GenerationFence,
    command_sequence: u64,
}

impl MediaHostHarness {
    fn spawn(contract: StartContract) -> anyhow::Result<Self> {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_easynet-remoteapp-media-host"));
        let build_id = sha256_file(&binary)?;
        let process_generation = NEXT_PROCESS_GENERATION.fetch_add(1, Ordering::Relaxed);
        let fence = GenerationFence {
            process_generation,
            build_id,
            session_nonce: format!("{process_generation:032x}"),
            transport_epoch: 1,
            media_source_epoch: process_generation,
            contract_digest: contract.digest()?,
        };
        let generation_nonce = generation_nonce_bytes(&fence)?;
        let video_capacity = binary_media_frame_capacity(
            MediaLane::Video,
            contract.video.max_access_unit_bytes as usize,
        )?;
        let video_shared = SharedMediaLaneFile::create(SharedMediaLaneLayout::new(
            MediaLane::Video,
            contract.video.max_pending_frames,
            video_capacity as u32,
            generation_nonce,
        )?)?;
        let audio_slot_count = contract
            .audio
            .as_ref()
            .map_or(1, |audio| audio.max_pending_packets);
        let audio_capacity = binary_media_frame_capacity(MediaLane::Audio, MAX_OPUS_PACKET_BYTES)?;
        let audio_shared = SharedMediaLaneFile::create(SharedMediaLaneLayout::new(
            MediaLane::Audio,
            audio_slot_count,
            audio_capacity as u32,
            generation_nonce,
        )?)?;
        let video_consumer = SharedMediaLaneConsumer::open(
            &video_shared.try_clone_file()?,
            MediaLane::Video,
            generation_nonce,
        )?;
        let audio_consumer = SharedMediaLaneConsumer::open(
            &audio_shared.try_clone_file()?,
            MediaLane::Audio,
            generation_nonce,
        )?;
        let (liveness_read, liveness_write) = pipe_for_child_reader()?;
        let (video_read, video_write) = pipe_for_child_writer()?;
        let (audio_read, audio_write) = pipe_for_child_writer()?;
        let video_shared_fd = video_shared.as_raw_fd();
        let audio_shared_fd = audio_shared.as_raw_fd();
        let mut command = ProcessCommand::new(&binary);
        command
            .env_clear()
            .env(
                "DISPLAY",
                std::env::var_os("DISPLAY")
                    .ok_or_else(|| anyhow::anyhow!("real X11 media-host test requires DISPLAY"))?,
            )
            .env(PARENT_LIVENESS_FD_ENV, liveness_read.to_string())
            .env(VIDEO_LANE_FD_ENV, video_write.to_string())
            .env(AUDIO_LANE_FD_ENV, audio_write.to_string())
            .env(VIDEO_SHARED_LANE_FD_ENV, video_shared_fd.to_string())
            .env(AUDIO_SHARED_LANE_FD_ENV, audio_shared_fd.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(move || {
                libc::close(liveness_write);
                libc::close(video_read);
                libc::close(audio_read);
                clear_close_on_exec(video_shared_fd)?;
                clear_close_on_exec(audio_shared_fd)?;
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        unsafe {
            libc::close(liveness_read);
            libc::close(video_write);
            libc::close(audio_write);
        }
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("media-host command lane missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("media-host control lane missing"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("media-host stderr missing"))?;
        let (control_tx, control) = mpsc::channel();
        let (video_tx, video) = mpsc::channel();
        let (audio_tx, audio) = mpsc::channel();
        let readers = vec![
            spawn_control_lane_reader(stdout, control_tx, fence.clone())?,
            spawn_shared_lane_reader(
                MediaLane::Video,
                unsafe { File::from_raw_fd(video_read) },
                video_consumer,
                video_tx,
                generation_nonce,
            )?,
            spawn_shared_lane_reader(
                MediaLane::Audio,
                unsafe { File::from_raw_fd(audio_read) },
                audio_consumer,
                audio_tx,
                generation_nonce,
            )?,
        ];
        let stderr_reader = Some(spawn_stderr_reader(stderr)?);
        let validator = MediaConversationValidator::new(fence.clone())?;
        let mut harness = Self {
            child,
            stdin: Some(stdin),
            _parent_liveness: unsafe { File::from_raw_fd(liveness_write) },
            control,
            video,
            audio,
            readers,
            stderr_reader,
            validator,
            fence,
            command_sequence: 0,
        };
        harness.send(CommandBody::StartPrepared { contract })?;
        Ok(harness)
    }

    fn send(&mut self, body: CommandBody) -> anyhow::Result<u64> {
        self.command_sequence = self
            .command_sequence
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("test command sequence overflow"))?;
        let command = Command {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            fence: self.fence.clone(),
            sequence: self.command_sequence,
            body,
        };
        self.validator.register_command(&command)?;
        write_command_frame(
            self.stdin
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("media-host command lane closed"))?,
            &command,
        )?;
        Ok(self.command_sequence)
    }

    fn await_control(
        &mut self,
        mut matches: impl FnMut(&EventBody) -> bool,
    ) -> anyhow::Result<EventMetadata> {
        let deadline = Instant::now() + EVENT_DEADLINE;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            anyhow::ensure!(!remaining.is_zero(), "media-host control deadline exceeded");
            let (metadata, payload) =
                require_control_lane_message(self.control.recv_timeout(remaining)?)?;
            let observation = self
                .validator
                .observe(MediaLane::Control, &metadata, &payload)?;
            anyhow::ensure!(observation == MediaObservation::Accepted);
            if let EventBody::Failed { reason, detail } = &metadata.body {
                anyhow::bail!("media-host failed ({reason:?}): {detail}");
            }
            if matches(&metadata.body) {
                return Ok(metadata);
            }
            anyhow::ensure!(matches!(metadata.body, EventBody::Stats { .. }));
        }
    }

    fn await_accepted_video(&mut self) -> anyhow::Result<(BinaryMediaEvent, Bytes)> {
        let deadline = Instant::now() + EVENT_DEADLINE;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            anyhow::ensure!(!remaining.is_zero(), "media-host video deadline exceeded");
            let (metadata, payload) =
                match require_media_lane_message(self.video.recv_timeout(remaining)?)? {
                    MediaLaneEvent::Frame(metadata, payload) => (metadata, payload),
                    MediaLaneEvent::Dropped(identity) => {
                        self.validator.observe_backpressure_drop(
                            MediaLane::Video,
                            identity.sequence,
                            identity.observed_at_ms,
                            identity.media_gate,
                        )?;
                        continue;
                    }
                };
            match self
                .validator
                .observe_binary_media(MediaLane::Video, &metadata, &payload)?
            {
                MediaObservation::Accepted => return Ok((metadata, payload)),
                MediaObservation::StaleDiscarded | MediaObservation::BackpressureDiscarded => {
                    continue
                }
            }
        }
    }

    fn await_failure(&mut self) -> anyhow::Result<(FailureReason, String)> {
        let deadline = Instant::now() + EVENT_DEADLINE;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            anyhow::ensure!(!remaining.is_zero(), "media-host failure deadline exceeded");
            let (metadata, payload) =
                require_control_lane_message(self.control.recv_timeout(remaining)?)?;
            let observation = self
                .validator
                .observe(MediaLane::Control, &metadata, &payload)?;
            anyhow::ensure!(observation == MediaObservation::Accepted);
            match metadata.body {
                EventBody::Failed { reason, detail } => return Ok((reason, detail)),
                EventBody::Stats { .. } => continue,
                other => {
                    anyhow::bail!("unexpected control event while awaiting failure: {other:?}")
                }
            }
        }
    }

    fn finish(mut self) -> anyhow::Result<()> {
        self.stdin.take();
        let deadline = Instant::now() + PROCESS_EXIT_DEADLINE;
        let status = loop {
            if let Some(status) = self.child.try_wait()? {
                break status;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "media-host did not exit after stopped acknowledgement"
            );
            thread::sleep(Duration::from_millis(10));
        };
        let diagnostics = self
            .stderr_reader
            .take()
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        anyhow::ensure!(
            status.success(),
            "media-host exited with {status}: {}",
            String::from_utf8_lossy(&diagnostics)
        );
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        Ok(())
    }
}

impl Drop for MediaHostHarness {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
#[ignore = "requires a real X11 display and the repository sentinel fixture"]
fn real_x11_window_and_application_sessions_emit_recoverable_h264() -> anyhow::Result<()> {
    let (mut sentinel, process_instance, windows) = start_sentinel()?;
    let window_target = NativeTargetPlan {
        kind: TargetKind::Window,
        display_id: None,
        window_id: Some(windows[0].id),
        pid: Some(i64::from(sentinel.child.id())),
        process_instance_id: Some(process_instance.clone()),
        app_identity: Some(windows[0].app_identity.clone()),
        bundle_id: None,
        application: None,
    };
    run_media_session(window_target)?;

    let mut window_ids = windows.iter().map(|window| window.id).collect::<Vec<_>>();
    window_ids.sort_unstable();
    let application_target = NativeTargetPlan {
        kind: TargetKind::Application,
        display_id: None,
        window_id: None,
        pid: Some(i64::from(sentinel.child.id())),
        process_instance_id: Some(process_instance.clone()),
        app_identity: Some(windows[0].app_identity.clone()),
        bundle_id: None,
        application: Some(ApplicationWindowSet {
            display_id: None,
            display_ids: Vec::new(),
            primary_pid: i64::from(sentinel.child.id()),
            process_instance_id: Some(process_instance),
            app_identity: Some(windows[0].app_identity.clone()),
            bundle_id: None,
            window_ids,
            window_set_epoch: 1,
            front_to_back_surfaces: windows
                .iter()
                .map(|window| ApplicationSurface {
                    window_id: window.id,
                    x: window.x,
                    y: window.y,
                    width: window.width,
                    height: window.height,
                })
                .collect(),
            surface_layout_epoch: 1,
        }),
    };
    run_media_session(application_target.clone())?;
    run_application_invalidation_session(application_target, &sentinel)?;
    let _ = sentinel.child.kill();
    let _ = sentinel.child.wait();
    Ok(())
}

fn run_media_session(target: NativeTargetPlan) -> anyhow::Result<()> {
    let contract = StartContract {
        target,
        video: video_config(320, 240, 10, 900),
        audio: None,
    };
    let mut host = MediaHostHarness::spawn(contract)?;
    host.await_control(|body| matches!(body, EventBody::Prepared { .. }))?;
    let activate = host.send(CommandBody::Activate)?;
    host.await_control(|body| {
        matches!(
            body,
            EventBody::Activated { command_sequence } if *command_sequence == activate
        )
    })?;
    host.send(CommandBody::BeginMedia {
        activation_command_sequence: activate,
    })?;
    let (first, first_payload) = host.await_accepted_video()?;
    assert!(!first_payload.is_empty());
    assert!(matches!(
        first.body,
        EventBody::VideoH264 {
            media_gate: 1,
            keyframe: true,
            sps_pps_present: true,
            discontinuity: true,
            width: 320,
            height: 240,
            ..
        }
    ));

    let keyframe = host.send(CommandBody::RequestKeyframe)?;
    host.await_control(|body| {
        matches!(
            body,
            EventBody::KeyframeRequested { command_sequence } if *command_sequence == keyframe
        )
    })?;
    let (recovery, _) = host.await_accepted_video()?;
    assert!(matches!(
        recovery.body,
        EventBody::VideoH264 {
            keyframe: true,
            sps_pps_present: true,
            ..
        }
    ));

    let reconfigured_video = video_config(256, 144, 8, 600);
    let reconfigure = host.send(CommandBody::Reconfigure {
        video: reconfigured_video.clone(),
        force_keyframe: true,
    })?;
    host.await_control(|body| {
        matches!(
            body,
            EventBody::Reconfigured {
                command_sequence,
                video,
                codec_generation: 2,
            } if *command_sequence == reconfigure && video == &reconfigured_video
        )
    })?;
    host.send(CommandBody::ResumeMedia {
        reconfigure_command_sequence: reconfigure,
    })?;
    let (resumed, _) = host.await_accepted_video()?;
    assert!(matches!(
        resumed.body,
        EventBody::VideoH264 {
            media_gate: 2,
            keyframe: true,
            sps_pps_present: true,
            discontinuity: true,
            codec_generation: 2,
            width: 256,
            height: 144,
            ..
        }
    ));
    assert!(host.audio.try_recv().is_err());

    let stop = host.send(CommandBody::Stop)?;
    host.await_control(|body| {
        matches!(
            body,
            EventBody::Stopped { command_sequence } if *command_sequence == stop
        )
    })?;
    host.finish()
}

fn run_application_invalidation_session(
    target: NativeTargetPlan,
    sentinel: &Sentinel,
) -> anyhow::Result<()> {
    let contract = StartContract {
        target,
        video: video_config(320, 240, 10, 900),
        audio: None,
    };
    let mut host = MediaHostHarness::spawn(contract)?;
    host.await_control(|body| matches!(body, EventBody::Prepared { .. }))?;
    let activate = host.send(CommandBody::Activate)?;
    host.await_control(|body| {
        matches!(
            body,
            EventBody::Activated { command_sequence } if *command_sequence == activate
        )
    })?;
    host.send(CommandBody::BeginMedia {
        activation_command_sequence: activate,
    })?;
    host.await_accepted_video()?;

    sentinel.move_secondary(720, 420)?;
    let (reason, detail) = host.await_failure()?;
    assert_eq!(reason, FailureReason::TargetInvalidated);
    assert!(
        detail.contains("geometry changed"),
        "unexpected failure detail: {detail}"
    );
    host.finish()
}

fn video_config(width: u32, height: u32, fps: u32, bitrate_kbps: u32) -> VideoConfig {
    VideoConfig {
        codec: VideoCodec::H264AnnexB,
        width,
        height,
        fps,
        bitrate_kbps,
        keyframe_interval_frames: fps,
        max_pending_frames: 3,
        max_access_unit_bytes: 1024 * 1024,
        max_nal_unit_bytes: 1_160,
        h264_profile_idc: 66,
        h264_level_idc: 31,
    }
}

fn start_sentinel() -> anyhow::Result<(Sentinel, String, Vec<X11WindowProof>)> {
    let fixture = std::env::var_os("EASYNET_REMOTEAPP_SENTINEL_FIXTURE")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("EASYNET_REMOTEAPP_SENTINEL_FIXTURE is required"))?;
    anyhow::ensure!(
        fixture.is_file(),
        "sentinel fixture is missing: {}",
        fixture.display()
    );
    let state = std::env::temp_dir().join(format!(
        "easynet-remoteapp-media-host-{}-{}.json",
        std::process::id(),
        NEXT_PROCESS_GENERATION.load(Ordering::Relaxed)
    ));
    let title_prefix = format!("EasyNet Media Host E2E {}", std::process::id());
    let child = ProcessCommand::new("python3")
        .arg(&fixture)
        .env("DISPLAY", std::env::var_os("DISPLAY").unwrap_or_default())
        .env("EASYNET_REMOTEAPP_LINUX_SENTINEL_STATE", &state)
        .env(
            "EASYNET_REMOTEAPP_LINUX_SENTINEL_CLASS",
            "EasyNetMediaHostE2E",
        )
        .env(
            "EASYNET_REMOTEAPP_LINUX_SENTINEL_TITLE_PREFIX",
            &title_prefix,
        )
        .env("EASYNET_REMOTEAPP_LINUX_SENTINEL_ROLE", "media_host_e2e")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let sentinel = Sentinel { child, state };
    let pid = sentinel.child.id();
    let deadline = Instant::now() + EVENT_DEADLINE;
    let mut last_inventory: Vec<String>;
    let windows = loop {
        let all_windows = xcap::Window::all().unwrap_or_default();
        last_inventory = all_windows
            .iter()
            .map(|window| {
                format!(
                    "id={:?} pid={:?} title={:?} app={:?}",
                    window.id(),
                    window.pid(),
                    window.title(),
                    window.app_name()
                )
            })
            .collect();
        let observed = all_windows
            .into_iter()
            .filter(|window| window.pid().ok() == Some(pid))
            .filter(|window| {
                window
                    .title()
                    .ok()
                    .is_some_and(|title| title.starts_with(&title_prefix))
            })
            .filter_map(|window| {
                Some(X11WindowProof {
                    id: u64::from(window.id().ok()?),
                    x: i64::from(window.x().ok()?),
                    y: i64::from(window.y().ok()?),
                    width: window.width().ok()?,
                    height: window.height().ok()?,
                    app_identity: window.app_name().ok()?,
                })
            })
            .collect::<Vec<_>>();
        if observed.len() == 2 {
            break observed;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "sentinel X11 windows did not appear; expected_pid={pid}; state={}; inventory={last_inventory:?}",
            std::fs::read_to_string(&sentinel.state).unwrap_or_else(|error| error.to_string())
        );
        thread::sleep(Duration::from_millis(20));
    };
    anyhow::ensure!(
        windows[0].app_identity == windows[1].app_identity,
        "sentinel windows have inconsistent application identity: {:?}",
        windows
            .iter()
            .map(|window| (&window.app_identity, window.id))
            .collect::<Vec<_>>()
    );
    Ok((sentinel, linux_process_instance(pid)?, windows))
}

fn linux_process_instance(pid: u32) -> anyhow::Result<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let command_end = stat
        .rfind(')')
        .ok_or_else(|| anyhow::anyhow!("sentinel process stat is malformed"))?;
    let start_ticks = stat[command_end + 1..]
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| anyhow::anyhow!("sentinel process stat has no starttime"))?;
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    Ok(format!("linux:{}:{pid}:{start_ticks}", boot_id.trim()))
}

fn spawn_control_lane_reader(
    mut reader: impl Read + Send + 'static,
    sender: mpsc::Sender<ControlLaneMessage>,
    fence: GenerationFence,
) -> anyhow::Result<JoinHandle<()>> {
    Ok(thread::Builder::new()
        .name("remoteapp-media-host-e2e-control".into())
        .spawn(move || loop {
            let message = read_event_frame(&mut reader, MediaLane::Control, Some(&fence));
            let terminal = !matches!(message, Ok(Some(_)));
            if sender.send(message).is_err() || terminal {
                return;
            }
        })?)
}

fn spawn_shared_lane_reader(
    lane: MediaLane,
    mut reader: File,
    consumer: SharedMediaLaneConsumer,
    sender: mpsc::Sender<MediaLaneMessage>,
    generation_nonce: [u8; 16],
) -> anyhow::Result<JoinHandle<()>> {
    Ok(thread::Builder::new()
        .name(format!("remoteapp-media-host-e2e-{lane:?}"))
        .spawn(move || loop {
            let message = read_shared_lane_message(&mut reader, &consumer, lane, generation_nonce);
            let terminal = !matches!(message, Ok(Some(_)));
            if sender.send(message).is_err() || terminal {
                return;
            }
        })?)
}

fn read_shared_lane_message(
    reader: &mut File,
    consumer: &SharedMediaLaneConsumer,
    lane: MediaLane,
    generation_nonce: [u8; 16],
) -> Result<Option<MediaLaneEvent>, FrameError> {
    let Some(notification) = SharedSlotNotification::read_from(reader, lane)
        .map_err(|error| FrameError::Decode(error.to_string()))?
    else {
        return Ok(None);
    };
    let ticket = match notification {
        SharedSlotNotification::Dropped { identity } => {
            return Ok(Some(MediaLaneEvent::Dropped(identity)))
        }
        SharedSlotNotification::Published(ticket) => ticket,
    };
    let lease = match consumer.claim(ticket) {
        Ok(lease) => lease,
        Err(SharedMediaLaneError::StaleTicket | SharedMediaLaneError::SlotUnavailable) => {
            return Ok(Some(MediaLaneEvent::Dropped(ticket.identity)))
        }
        Err(error) => return Err(FrameError::Decode(error.to_string())),
    };
    let frame = Bytes::from_owner(lease);
    let frame_start = frame.as_ptr() as usize;
    let (metadata, payload_view) =
        decode_binary_media_event_frame_compact(&frame, lane, generation_nonce)?;
    let payload_start = (payload_view.as_ptr() as usize)
        .checked_sub(frame_start)
        .ok_or_else(|| FrameError::Decode("mapped payload precedes its frame".into()))?;
    let payload_end = payload_start
        .checked_add(payload_view.len())
        .ok_or(FrameError::Oversized)?;
    let media_gate = match metadata.body {
        EventBody::VideoH264 { media_gate, .. } | EventBody::AudioOpus { media_gate, .. } => {
            media_gate
        }
        _ => {
            return Err(FrameError::Decode(
                "shared lane carried control metadata".into(),
            ))
        }
    };
    if metadata.sequence != ticket.identity.sequence
        || metadata.observed_at_ms != ticket.identity.observed_at_ms
        || media_gate != ticket.identity.media_gate
    {
        return Err(FrameError::Decode(
            "shared slot notification differs from fixed frame identity".into(),
        ));
    }
    let payload = frame.slice(payload_start..payload_end);
    Ok(Some(MediaLaneEvent::Frame(metadata, payload)))
}

fn spawn_stderr_reader(mut stderr: ChildStderr) -> anyhow::Result<JoinHandle<Vec<u8>>> {
    Ok(thread::Builder::new()
        .name("remoteapp-media-host-e2e-stderr".into())
        .spawn(move || {
            let mut diagnostics = Vec::new();
            let _ = stderr.read_to_end(&mut diagnostics);
            diagnostics
        })?)
}

fn require_control_lane_message(
    message: ControlLaneMessage,
) -> anyhow::Result<(EventMetadata, Vec<u8>)> {
    message?.ok_or_else(|| anyhow::anyhow!("media-host physical lane reached unexpected EOF"))
}

fn require_media_lane_message(message: MediaLaneMessage) -> anyhow::Result<MediaLaneEvent> {
    message?.ok_or_else(|| anyhow::anyhow!("media-host physical lane reached unexpected EOF"))
}

fn pipe_for_child_reader() -> anyhow::Result<(RawFd, RawFd)> {
    let (read, write) = pipe()?;
    set_close_on_exec(write)?;
    Ok((read, write))
}

fn pipe_for_child_writer() -> anyhow::Result<(RawFd, RawFd)> {
    let (read, write) = pipe()?;
    set_close_on_exec(read)?;
    Ok((read, write))
}

fn pipe() -> anyhow::Result<(RawFd, RawFd)> {
    let mut descriptors = [-1; 2];
    if unsafe { libc::pipe(descriptors.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok((descriptors[0], descriptors[1]))
}

fn set_close_on_exec(fd: RawFd) -> anyhow::Result<()> {
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn clear_close_on_exec(fd: RawFd) -> io::Result<()> {
    if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

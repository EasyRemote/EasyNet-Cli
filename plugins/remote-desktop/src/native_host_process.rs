// EasyNet CLI — plugin-private native host process boundary
// ==========================================================
//
// Owns the process lifecycle and bounded framed-I/O mechanics shared by the
// RemoteApp target-observation and media-capability clients. Domain request and
// response validation stays with each caller and the private protocol crate.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::time::Instant;

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
use bytes::Bytes;

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
use std::collections::VecDeque;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
use std::sync::atomic::AtomicU64;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
use std::sync::{Mutex, MutexGuard};

#[cfg(target_os = "macos")]
use easynet_remoteapp_native_protocol::macos_launch_services::{
    send_file_descriptors, ARG as MACOS_LAUNCH_SERVICES_ARG,
};
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
use easynet_remoteapp_native_protocol::media_session::{
    binary_media_frame_capacity, decode_binary_media_event_frame_compact, generation_nonce_bytes,
    read_event_frame, write_command_frame, BinaryMediaEvent, Command as MediaHostCommand,
    EventBody, EventMetadata, GenerationFence, MediaConversationValidator, MediaLane,
    MediaObservation, StartContract,
};
#[cfg(all(
    unix,
    feature = "native-media",
    any(target_os = "linux", target_os = "macos")
))]
use easynet_remoteapp_native_protocol::media_session::{AUDIO_LANE_FD_ENV, VIDEO_LANE_FD_ENV};
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
use easynet_remoteapp_native_protocol::shared_media_lane::{
    DetachedMediaBufferPool, SharedMediaLaneConsumer, SharedMediaLaneError, SharedMediaLaneFile,
    SharedMediaLaneLayout, SharedSlotNotification,
};
#[cfg(all(feature = "native-media", target_os = "windows"))]
use easynet_remoteapp_native_protocol::shared_media_lane::{
    WindowsNotificationPipeServer, AUDIO_NOTIFICATION_PIPE_NAME_ENV, AUDIO_SHARED_LANE_NAME_ENV,
    VIDEO_NOTIFICATION_PIPE_NAME_ENV, VIDEO_SHARED_LANE_NAME_ENV,
};
#[cfg(all(
    unix,
    feature = "native-media",
    any(target_os = "linux", target_os = "macos")
))]
use easynet_remoteapp_native_protocol::shared_media_lane::{
    AUDIO_SHARED_LANE_FD_ENV, VIDEO_SHARED_LANE_FD_ENV,
};
#[cfg(unix)]
use easynet_remoteapp_native_protocol::PARENT_LIVENESS_FD_ENV;
use easynet_remoteapp_native_protocol::{read_frame, write_frame, FrameError};
use serde::de::DeserializeOwned;
use serde::Serialize;

const STDERR_MAX_BYTES: usize = 16 * 1024;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
const MEDIA_CONTROL_QUEUE_DEPTH: usize = 8;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
const MEDIA_VIDEO_QUEUE_DEPTH: usize = 3;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
const MEDIA_AUDIO_QUEUE_DEPTH: usize = 4;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
const MEDIA_VIDEO_TRANSPORT_POOL_BUFFERS: usize = 32;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
const MEDIA_VIDEO_TRANSPORT_POOL_BYTES: usize = 32 * 1024 * 1024;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
const MEDIA_AUDIO_TRANSPORT_POOL_BUFFERS: usize = 64;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
const MEDIA_AUDIO_TRANSPORT_POOL_BYTES: usize = 256 * 1024;

#[cfg(windows)]
struct WindowsKillOnCloseJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for WindowsKillOnCloseJob {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
            self.0 = std::ptr::null_mut();
        }
    }
}

pub(super) struct NativeHostProcess<Response> {
    id: u64,
    child: Child,
    stdin: Option<Box<dyn Write + Send>>,
    responses: Receiver<Result<Response, FrameError>>,
    protocol_violation: Arc<AtomicBool>,
    stdout_reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<Vec<u8>>>,
    #[cfg(unix)]
    parent_liveness: Option<std::fs::File>,
    #[cfg(windows)]
    kill_on_close_job: Option<WindowsKillOnCloseJob>,
}

impl<Response> NativeHostProcess<Response>
where
    Response: DeserializeOwned + Send + 'static,
{
    pub(super) fn spawn(
        id: u64,
        executable_name: &str,
        thread_label: &str,
        extra_environment: &[(OsString, OsString)],
    ) -> io::Result<Self> {
        let executable = sibling_executable(executable_name)?;
        #[cfg(target_os = "macos")]
        if executable_name == super::MEDIA_HOST_EXECUTABLE {
            return spawn_macos_one_shot_media_host(id, executable, thread_label);
        }
        let mut command = Command::new(&executable);
        command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        project_os_bootstrap_environment(&mut command);
        for (name, value) in extra_environment {
            command.env(name, value);
        }
        #[cfg(unix)]
        let (parent_liveness_read_fd, parent_liveness) = configure_parent_liveness(&mut command)?;
        let child_result = command.spawn();
        #[cfg(unix)]
        unsafe {
            libc::close(parent_liveness_read_fd);
        }
        let mut child = child_result?;
        #[cfg(windows)]
        let kill_on_close_job = match assign_kill_on_close_job(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdin = child.stdin.take().ok_or_else(|| {
            terminate_spawn_failure(&mut child, None);
            io::Error::other("native host stdin unavailable")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            terminate_spawn_failure(&mut child, None);
            io::Error::other("native host stdout unavailable")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            terminate_spawn_failure(&mut child, None);
            io::Error::other("native host stderr unavailable")
        })?;
        let (response_tx, responses) = mpsc::sync_channel(1);
        let protocol_violation = Arc::new(AtomicBool::new(false));
        let protocol_violation_for_reader = Arc::clone(&protocol_violation);
        let stdout_name = format!("easynet-rd-{thread_label}-{id}-stdout");
        let stdout_reader = match thread::Builder::new()
            .name(stdout_name)
            .spawn(move || read_responses(stdout, response_tx, protocol_violation_for_reader))
        {
            Ok(reader) => reader,
            Err(error) => {
                terminate_spawn_failure(&mut child, None);
                return Err(error);
            }
        };
        let stderr_name = format!("easynet-rd-{thread_label}-{id}-stderr");
        let stderr_reader = match thread::Builder::new()
            .name(stderr_name)
            .spawn(move || read_capped_diagnostics(stderr, STDERR_MAX_BYTES))
        {
            Ok(reader) => reader,
            Err(error) => {
                terminate_spawn_failure(&mut child, Some(stdout_reader));
                return Err(error);
            }
        };
        Ok(Self {
            id,
            child,
            stdin: Some(Box::new(stdin)),
            responses,
            protocol_violation,
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
            #[cfg(unix)]
            parent_liveness: Some(parent_liveness),
            #[cfg(windows)]
            kill_on_close_job: Some(kill_on_close_job),
        })
    }

    pub(super) const fn id(&self) -> u64 {
        self.id
    }

    pub(super) fn protocol_violated(&self) -> bool {
        self.protocol_violation.load(Ordering::Acquire)
    }

    pub(super) fn write_request<Request: Serialize>(
        &mut self,
        request: &Request,
    ) -> Result<(), FrameError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| FrameError::Io("native host stdin closed".into()))?;
        write_frame(stdin, request)
    }

    pub(super) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Result<Response, FrameError>, RecvTimeoutError> {
        self.responses.recv_timeout(timeout)
    }

    pub(super) fn try_recv(&self) -> Result<Result<Response, FrameError>, TryRecvError> {
        self.responses.try_recv()
    }

    pub(super) fn close_stdin(&mut self) {
        self.stdin.take();
    }

    pub(super) fn wait_for_success(&mut self, timeout: Duration) -> io::Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait()? {
                if let Some(reader) = self.stdout_reader.take() {
                    let _ = reader.join();
                }
                return Ok(status.success());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }
            thread::park_timeout(remaining.min(Duration::from_millis(5)));
        }
    }
}

#[cfg(target_os = "macos")]
fn spawn_macos_one_shot_media_host<Response>(
    id: u64,
    executable: std::path::PathBuf,
    thread_label: &str,
) -> io::Result<NativeHostProcess<Response>>
where
    Response: DeserializeOwned + Send + 'static,
{
    use std::os::fd::AsRawFd;

    let (child_stdin, parent_stdin) = macos_pipe()?;
    let (parent_stdout, child_stdout) = macos_pipe()?;
    let (parent_stderr, child_stderr) = macos_pipe()?;
    let (child_liveness, parent_liveness) = macos_pipe()?;
    let null = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")?;
    let descriptors = [
        child_stdin.as_raw_fd(),
        child_stdout.as_raw_fd(),
        child_stderr.as_raw_fd(),
        child_liveness.as_raw_fd(),
        null.as_raw_fd(),
        null.as_raw_fd(),
        null.as_raw_fd(),
        null.as_raw_fd(),
    ];
    let child = spawn_macos_launch_services_app(&executable, &descriptors, &[])?;
    drop((
        child_stdin,
        child_stdout,
        child_stderr,
        child_liveness,
        null,
    ));

    let (response_tx, responses) = mpsc::sync_channel(1);
    let protocol_violation = Arc::new(AtomicBool::new(false));
    let violation_for_reader = Arc::clone(&protocol_violation);
    let stdout_name = format!("easynet-rd-{thread_label}-{id}-stdout");
    let stdout_reader = thread::Builder::new()
        .name(stdout_name)
        .spawn(move || read_responses(parent_stdout, response_tx, violation_for_reader))?;
    let stderr_name = format!("easynet-rd-{thread_label}-{id}-stderr");
    let stderr_reader = match thread::Builder::new()
        .name(stderr_name)
        .spawn(move || read_capped_diagnostics(parent_stderr, STDERR_MAX_BYTES))
    {
        Ok(reader) => reader,
        Err(error) => {
            let mut child = child;
            terminate_spawn_failure(&mut child, Some(stdout_reader));
            return Err(error);
        }
    };
    Ok(NativeHostProcess {
        id,
        child,
        stdin: Some(Box::new(parent_stdin)),
        responses,
        protocol_violation,
        stdout_reader: Some(stdout_reader),
        stderr_reader: Some(stderr_reader),
        parent_liveness: Some(parent_liveness),
    })
}

#[cfg(target_os = "macos")]
fn macos_pipe() -> io::Result<(std::fs::File, std::fs::File)> {
    use std::os::fd::FromRawFd;

    let mut descriptors = [-1_i32; 2];
    if unsafe { libc::pipe(descriptors.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe {
        (
            std::fs::File::from_raw_fd(descriptors[0]),
            std::fs::File::from_raw_fd(descriptors[1]),
        )
    })
}

#[cfg(target_os = "macos")]
fn spawn_macos_launch_services_app(
    executable: &std::path::Path,
    descriptors: &[std::os::fd::RawFd],
    extra_environment: &[(OsString, OsString)],
) -> io::Result<Child> {
    let app = executable
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
        .ok_or_else(|| io::Error::other("media host executable is not inside an app bundle"))?;
    let bootstrap = tempfile::tempdir()?;
    let socket = bootstrap.path().join("bootstrap.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    let mut command = Command::new("/usr/bin/open");
    command.arg("-W").arg("-n").arg("-g");
    for (name, value) in extra_environment {
        let mut assignment = name.clone();
        assignment.push("=");
        assignment.push(value);
        command.arg("--env").arg(assignment);
    }
    command
        .arg(app)
        .arg("--args")
        .arg(MACOS_LAUNCH_SERVICES_ARG)
        .arg(&socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "media host app did not connect to LaunchServices bootstrap",
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
    };
    if let Err(error) = send_file_descriptors(&stream, descriptors) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok(child)
}

/// Execute one exact request/response exchange against a plugin-private host.
///
/// This helper centralizes the kill/reap, bounded-response and no-extra-frame
/// contract used by one-shot media-host control operations. Domain envelope
/// validation remains with the caller because each private protocol owns its
/// own generation and operation semantics.
pub(super) fn execute_one_shot_native_host<Request, Response>(
    id: u64,
    executable_name: &str,
    thread_label: &str,
    extra_environment: &[(OsString, OsString)],
    request: &Request,
    deadline: Duration,
) -> Result<Response, String>
where
    Request: Serialize,
    Response: DeserializeOwned + Send + 'static,
{
    let mut process =
        NativeHostProcess::<Response>::spawn(id, executable_name, thread_label, extra_environment)
            .map_err(|error| format!("spawn native host: {error}"))?;
    process
        .write_request(request)
        .map_err(|error| format!("write native-host request: {error}"))?;
    let started = Instant::now();
    let response = process
        .recv_timeout(deadline)
        .map_err(|error| format!("await native-host response: {error}"))?
        .map_err(|error| format!("decode native-host response: {error}"))?;
    process.close_stdin();
    let remaining = deadline.saturating_sub(started.elapsed());
    if !process
        .wait_for_success(remaining)
        .map_err(|error| format!("wait for native host: {error}"))?
    {
        return Err("native host did not exit successfully before deadline".into());
    }
    if process.protocol_violated() {
        return Err("native host violated the one-shot response protocol".into());
    }
    match process.try_recv() {
        Err(TryRecvError::Disconnected) => Ok(response),
        Err(TryRecvError::Empty) => {
            Err("native-host response reader remained open after process exit".into())
        }
        Ok(_) => Err("native host returned more than one response".into()),
    }
}

impl<Response> NativeHostProcess<Response> {
    pub(super) fn terminate(&mut self) {
        self.stdin.take();
        #[cfg(unix)]
        self.parent_liveness.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(windows)]
        self.kill_on_close_job.take();
        if let Some(reader) = self.stdout_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

impl<Response> Drop for NativeHostProcess<Response> {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// One supervised active-media process generation.
///
/// The process boundary owns only framed I/O, generation validation and
/// killability. RemoteApp session, consent, WebRTC and receipt policy remain in
/// the daemon callers of this object.
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
pub(super) struct MediaHostProcess {
    id: u64,
    child: Child,
    stdin: Option<Box<dyn Write + Send>>,
    conversation: Arc<Mutex<MediaConversationValidator>>,
    control: Receiver<Result<Option<MediaHostEvent>, FrameError>>,
    video: Arc<VideoEventMailbox>,
    audio: Receiver<Result<Option<MediaHostMediaEvent>, FrameError>>,
    protocol_violation: Arc<AtomicBool>,
    readers: Vec<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<Vec<u8>>>,
    #[cfg(unix)]
    parent_liveness: Option<std::fs::File>,
    #[cfg(windows)]
    kill_on_close_job: Option<WindowsKillOnCloseJob>,
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
pub(super) struct MediaHostEvent {
    pub(super) metadata: EventMetadata,
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
pub(super) struct MediaHostMediaEvent {
    pub(super) metadata: BinaryMediaEvent,
    pub(super) payload: Bytes,
    pub(super) observation: MediaObservation,
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
impl MediaHostProcess {
    pub(super) fn spawn(
        id: u64,
        executable_name: &str,
        fence: GenerationFence,
        contract: &StartContract,
        extra_environment: &[(OsString, OsString)],
    ) -> io::Result<Self> {
        let executable = sibling_executable(executable_name)?;
        let executable_digest = sha256_file(&executable)?;
        if executable_digest != fence.build_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "media-host fence build_id differs from installed executable digest",
            ));
        }
        let conversation = Arc::new(Mutex::new(
            MediaConversationValidator::new(fence.clone())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?,
        ));
        let mut command = Command::new(&executable);
        command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        project_os_bootstrap_environment(&mut command);
        for (name, value) in extra_environment {
            command.env(name, value);
        }
        #[cfg(unix)]
        let (parent_liveness_read_fd, parent_liveness) = configure_parent_liveness(&mut command)?;
        #[cfg(unix)]
        let video_lane = match configure_media_output_lane(&mut command, VIDEO_LANE_FD_ENV) {
            Ok(lane) => lane,
            Err(error) => {
                unsafe { libc::close(parent_liveness_read_fd) };
                return Err(error);
            }
        };
        #[cfg(unix)]
        let audio_lane = match configure_media_output_lane(&mut command, AUDIO_LANE_FD_ENV) {
            Ok(lane) => lane,
            Err(error) => {
                unsafe { libc::close(parent_liveness_read_fd) };
                return Err(error);
            }
        };
        let generation_nonce = generation_nonce_bytes(&fence)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
        #[cfg(windows)]
        let video_lane = WindowsNotificationPipeServer::create(MediaLane::Video, generation_nonce)
            .map_err(shared_lane_io_error)?;
        #[cfg(windows)]
        let audio_lane = WindowsNotificationPipeServer::create(MediaLane::Audio, generation_nonce)
            .map_err(shared_lane_io_error)?;
        #[cfg(windows)]
        {
            command.env(
                VIDEO_NOTIFICATION_PIPE_NAME_ENV,
                video_lane.bootstrap_name(),
            );
            command.env(
                AUDIO_NOTIFICATION_PIPE_NAME_ENV,
                audio_lane.bootstrap_name(),
            );
        }
        let video_capacity = binary_media_frame_capacity(
            MediaLane::Video,
            contract.video.max_access_unit_bytes as usize,
        )
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
        let video_layout = SharedMediaLaneLayout::new(
            MediaLane::Video,
            contract
                .video
                .max_pending_frames
                .min(MEDIA_VIDEO_QUEUE_DEPTH as u32),
            u32::try_from(video_capacity).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "video shared capacity overflow",
                )
            })?,
            generation_nonce,
        )
        .map_err(shared_lane_io_error)?;
        let audio_slot_count = contract
            .audio
            .as_ref()
            .map_or(1, |audio| audio.max_pending_packets)
            .min(MEDIA_AUDIO_QUEUE_DEPTH as u32);
        let audio_capacity = binary_media_frame_capacity(MediaLane::Audio, 1_275)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
        let audio_layout = SharedMediaLaneLayout::new(
            MediaLane::Audio,
            audio_slot_count,
            u32::try_from(audio_capacity).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "audio shared capacity overflow",
                )
            })?,
            generation_nonce,
        )
        .map_err(shared_lane_io_error)?;
        let video_shared =
            SharedMediaLaneFile::create(video_layout).map_err(shared_lane_io_error)?;
        let audio_shared =
            SharedMediaLaneFile::create(audio_layout).map_err(shared_lane_io_error)?;
        #[cfg(unix)]
        {
            configure_shared_media_lane(&mut command, VIDEO_SHARED_LANE_FD_ENV, &video_shared)?;
            configure_shared_media_lane(&mut command, AUDIO_SHARED_LANE_FD_ENV, &audio_shared)?;
        }
        #[cfg(windows)]
        {
            command.env(VIDEO_SHARED_LANE_NAME_ENV, video_shared.bootstrap_name());
            command.env(AUDIO_SHARED_LANE_NAME_ENV, audio_shared.bootstrap_name());
        }
        #[cfg(unix)]
        let video_consumer = SharedMediaLaneConsumer::open(
            &video_shared
                .try_clone_file()
                .map_err(shared_lane_io_error)?,
            MediaLane::Video,
            generation_nonce,
        )
        .map_err(shared_lane_io_error)?;
        #[cfg(windows)]
        let video_consumer = SharedMediaLaneConsumer::open_named(
            video_shared.bootstrap_name(),
            MediaLane::Video,
            generation_nonce,
        )
        .map_err(shared_lane_io_error)?;
        #[cfg(unix)]
        let audio_consumer = SharedMediaLaneConsumer::open(
            &audio_shared
                .try_clone_file()
                .map_err(shared_lane_io_error)?,
            MediaLane::Audio,
            generation_nonce,
        )
        .map_err(shared_lane_io_error)?;
        #[cfg(windows)]
        let audio_consumer = SharedMediaLaneConsumer::open_named(
            audio_shared.bootstrap_name(),
            MediaLane::Audio,
            generation_nonce,
        )
        .map_err(shared_lane_io_error)?;
        #[cfg(target_os = "macos")]
        let (child_stdin, parent_stdin) = macos_pipe()?;
        #[cfg(target_os = "macos")]
        let (parent_stdout, child_stdout) = macos_pipe()?;
        #[cfg(target_os = "macos")]
        let (parent_stderr, child_stderr) = macos_pipe()?;
        #[cfg(target_os = "macos")]
        let child_result = {
            use std::os::fd::AsRawFd;

            let descriptors = [
                child_stdin.as_raw_fd(),
                child_stdout.as_raw_fd(),
                child_stderr.as_raw_fd(),
                parent_liveness_read_fd,
                video_lane.child_write.as_raw_fd(),
                audio_lane.child_write.as_raw_fd(),
                video_shared.as_raw_fd(),
                audio_shared.as_raw_fd(),
            ];
            spawn_macos_launch_services_app(&executable, &descriptors, extra_environment)
        };
        #[cfg(not(target_os = "macos"))]
        let child_result = command.spawn();
        #[cfg(unix)]
        unsafe {
            libc::close(parent_liveness_read_fd);
        }
        #[cfg(unix)]
        drop(video_lane.child_write);
        #[cfg(unix)]
        drop(audio_lane.child_write);
        #[cfg(target_os = "macos")]
        drop((child_stdin, child_stdout, child_stderr));
        let mut child = child_result?;
        #[cfg(windows)]
        let kill_on_close_job = match assign_kill_on_close_job(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        #[cfg(target_os = "macos")]
        let stdin: Box<dyn Write + Send> = Box::new(parent_stdin);
        #[cfg(not(target_os = "macos"))]
        let stdin: Box<dyn Write + Send> = Box::new(child.stdin.take().ok_or_else(|| {
            terminate_spawn_failure(&mut child, None);
            io::Error::other("media host command lane unavailable")
        })?);
        #[cfg(target_os = "macos")]
        let stdout: Box<dyn Read + Send> = Box::new(parent_stdout);
        #[cfg(not(target_os = "macos"))]
        let stdout: Box<dyn Read + Send> = Box::new(child.stdout.take().ok_or_else(|| {
            terminate_spawn_failure(&mut child, None);
            io::Error::other("media host control lane unavailable")
        })?);
        #[cfg(target_os = "macos")]
        let stderr: Box<dyn Read + Send> = Box::new(parent_stderr);
        #[cfg(not(target_os = "macos"))]
        let stderr: Box<dyn Read + Send> = Box::new(child.stderr.take().ok_or_else(|| {
            terminate_spawn_failure(&mut child, None);
            io::Error::other("media host stderr unavailable")
        })?);
        let protocol_violation = Arc::new(AtomicBool::new(false));
        let (control_tx, control) = mpsc::sync_channel(MEDIA_CONTROL_QUEUE_DEPTH);
        let (audio_tx, audio) = mpsc::sync_channel(MEDIA_AUDIO_QUEUE_DEPTH);
        let video = Arc::new(VideoEventMailbox::new(MEDIA_VIDEO_QUEUE_DEPTH));
        let video_transport_pool = DetachedMediaBufferPool::new(
            MEDIA_VIDEO_TRANSPORT_POOL_BUFFERS,
            MEDIA_VIDEO_TRANSPORT_POOL_BYTES,
        )
        .map_err(shared_lane_io_error)?;
        let audio_transport_pool = DetachedMediaBufferPool::new(
            MEDIA_AUDIO_TRANSPORT_POOL_BUFFERS,
            MEDIA_AUDIO_TRANSPORT_POOL_BYTES,
        )
        .map_err(shared_lane_io_error)?;
        let mut readers: Vec<JoinHandle<()>> = Vec::with_capacity(3);
        let control_violation = Arc::clone(&protocol_violation);
        let control_validator = Arc::clone(&conversation);
        let control_fence = fence.clone();
        let control_reader = thread::Builder::new()
            .name(format!("easynet-rd-media-{id}-control"))
            .spawn(move || {
                read_bounded_media_events(
                    stdout,
                    MediaLane::Control,
                    control_tx,
                    control_validator,
                    control_violation,
                    control_fence,
                )
            });
        match control_reader {
            Ok(reader) => readers.push(reader),
            Err(error) => {
                terminate_spawn_failure(&mut child, None);
                return Err(error);
            }
        }
        let audio_validator = Arc::clone(&conversation);
        let audio_violation = Arc::clone(&protocol_violation);
        let audio_generation_nonce = generation_nonce;
        #[cfg(unix)]
        let audio_notification_reader = audio_lane.parent_read;
        #[cfg(windows)]
        let audio_notification_reader = audio_lane;
        let audio_reader = thread::Builder::new()
            .name(format!("easynet-rd-media-{id}-audio"))
            .spawn(move || {
                read_shared_audio_events(
                    audio_notification_reader,
                    audio_consumer,
                    audio_tx,
                    audio_validator,
                    audio_violation,
                    audio_generation_nonce,
                    audio_transport_pool,
                )
            });
        match audio_reader {
            Ok(reader) => readers.push(reader),
            Err(error) => {
                terminate_spawn_failure(&mut child, None);
                for reader in readers {
                    let _ = reader.join();
                }
                return Err(error);
            }
        }
        let video_mailbox = Arc::clone(&video);
        let video_validator = Arc::clone(&conversation);
        let video_violation = Arc::clone(&protocol_violation);
        let video_generation_nonce = generation_nonce;
        #[cfg(unix)]
        let video_notification_reader = video_lane.parent_read;
        #[cfg(windows)]
        let video_notification_reader = video_lane;
        let video_reader = thread::Builder::new()
            .name(format!("easynet-rd-media-{id}-video"))
            .spawn(move || {
                read_video_events(
                    video_notification_reader,
                    video_consumer,
                    video_mailbox,
                    video_validator,
                    video_violation,
                    video_generation_nonce,
                    video_transport_pool,
                )
            });
        match video_reader {
            Ok(reader) => readers.push(reader),
            Err(error) => {
                terminate_spawn_failure(&mut child, None);
                for reader in readers {
                    let _ = reader.join();
                }
                return Err(error);
            }
        }
        let stderr_reader = match thread::Builder::new()
            .name(format!("easynet-rd-media-{id}-stderr"))
            .spawn(move || read_capped_diagnostics(stderr, STDERR_MAX_BYTES))
        {
            Ok(reader) => reader,
            Err(error) => {
                terminate_spawn_failure(&mut child, None);
                for reader in readers {
                    let _ = reader.join();
                }
                return Err(error);
            }
        };
        Ok(Self {
            id,
            child,
            stdin: Some(stdin),
            conversation,
            control,
            video,
            audio,
            protocol_violation,
            readers,
            stderr_reader: Some(stderr_reader),
            #[cfg(unix)]
            parent_liveness: Some(parent_liveness),
            #[cfg(windows)]
            kill_on_close_job: Some(kill_on_close_job),
        })
    }

    pub(super) const fn id(&self) -> u64 {
        self.id
    }

    pub(super) fn protocol_violated(&self) -> bool {
        self.protocol_violation.load(Ordering::Acquire)
    }

    pub(super) fn send_command(&mut self, command: &MediaHostCommand) -> Result<(), FrameError> {
        lock_conversation(&self.conversation)?
            .register_command(command)
            .map_err(|error| FrameError::Encode(error.to_string()))?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| FrameError::Io("media host command lane closed".into()))?;
        if let Err(error) = write_command_frame(stdin, command) {
            self.protocol_violation.store(true, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn recv_control_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<MediaHostEvent, FrameError> {
        let message = self.control.recv_timeout(timeout).map_err(|error| {
            FrameError::Io(format!("media host control receive failed: {error}"))
        })?;
        require_media_event(message)
    }

    pub(super) fn try_recv_control(&mut self) -> Result<Option<MediaHostEvent>, FrameError> {
        match self.control.try_recv() {
            Ok(message) => require_media_event(message).map(Some),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(FrameError::UnexpectedEof),
        }
    }

    pub(super) fn try_recv_video(&mut self) -> Result<Option<MediaHostMediaEvent>, FrameError> {
        match self.video.try_pop() {
            Some(message) => require_media_event(message).map(Some),
            None if self.video.is_closed() => Err(FrameError::UnexpectedEof),
            None => Ok(None),
        }
    }

    pub(super) fn try_recv_audio(&mut self) -> Result<Option<MediaHostMediaEvent>, FrameError> {
        match self.audio.try_recv() {
            Ok(message) => require_media_event(message).map(Some),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(FrameError::UnexpectedEof),
        }
    }

    pub(super) fn take_video_recovery_request(&self) -> bool {
        self.video.take_recovery_request()
    }

    pub(super) fn video_frames_dropped(&self) -> u64 {
        self.video.frames_dropped()
    }

    pub(super) fn video_queue_depth(&self) -> usize {
        self.video.depth()
    }

    pub(super) fn video_recovery_overdue(&self, deadline: Duration) -> bool {
        self.video.recovery_overdue(deadline)
    }

    pub(super) fn close_commands(&mut self) {
        self.stdin.take();
    }

    pub(super) fn terminate(&mut self) {
        self.stdin.take();
        #[cfg(unix)]
        self.parent_liveness.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(windows)]
        self.kill_on_close_job.take();
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
impl Drop for MediaHostProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn read_bounded_media_events(
    mut reader: impl Read,
    lane: MediaLane,
    sender: SyncSender<Result<Option<MediaHostEvent>, FrameError>>,
    conversation: Arc<Mutex<MediaConversationValidator>>,
    protocol_violation: Arc<AtomicBool>,
    fence: GenerationFence,
) {
    loop {
        let message = validate_media_message(
            lane,
            read_event_frame(&mut reader, lane, Some(&fence)),
            &conversation,
        );
        let terminal = !matches!(message, Ok(Some(_)));
        if message.is_err() {
            protocol_violation.store(true, Ordering::Release);
        }
        match sender.try_send(message) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                protocol_violation.store(true, Ordering::Release);
                return;
            }
            Err(TrySendError::Disconnected(_)) => return,
        }
        if terminal {
            return;
        }
    }
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn read_video_events<Reader: Read>(
    mut reader: Reader,
    consumer: SharedMediaLaneConsumer,
    mailbox: Arc<VideoEventMailbox>,
    conversation: Arc<Mutex<MediaConversationValidator>>,
    protocol_violation: Arc<AtomicBool>,
    generation_nonce: [u8; 16],
    transport_pool: DetachedMediaBufferPool,
) {
    loop {
        let message = read_shared_media_event(
            &mut reader,
            &consumer,
            MediaLane::Video,
            &conversation,
            generation_nonce,
            &transport_pool,
        );
        match message {
            Ok(SharedMediaRead::Event(event)) => mailbox.push(event),
            Ok(SharedMediaRead::Dropped(MediaObservation::BackpressureDiscarded)) => {
                mailbox.register_transport_drop()
            }
            Ok(SharedMediaRead::Dropped(_)) => {}
            Ok(SharedMediaRead::Eof) => {
                mailbox.close(Ok(None));
                return;
            }
            Err(error) => {
                if !matches!(error, FrameError::UnexpectedEof) {
                    protocol_violation.store(true, Ordering::Release);
                }
                mailbox.close(Err(error));
                return;
            }
        }
    }
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn read_shared_audio_events<Reader: Read>(
    mut reader: Reader,
    consumer: SharedMediaLaneConsumer,
    sender: SyncSender<Result<Option<MediaHostMediaEvent>, FrameError>>,
    conversation: Arc<Mutex<MediaConversationValidator>>,
    protocol_violation: Arc<AtomicBool>,
    generation_nonce: [u8; 16],
    transport_pool: DetachedMediaBufferPool,
) {
    loop {
        let message = read_shared_media_event(
            &mut reader,
            &consumer,
            MediaLane::Audio,
            &conversation,
            generation_nonce,
            &transport_pool,
        );
        match message {
            Ok(SharedMediaRead::Event(event)) => match sender.try_send(Ok(Some(event))) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => continue,
                Err(TrySendError::Disconnected(_)) => return,
            },
            Ok(SharedMediaRead::Dropped(_)) => continue,
            Ok(SharedMediaRead::Eof) => {
                let _ = sender.try_send(Ok(None));
                return;
            }
            Err(error) => {
                protocol_violation.store(true, Ordering::Release);
                let _ = sender.try_send(Err(error));
                return;
            }
        }
    }
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
enum SharedMediaRead {
    Event(MediaHostMediaEvent),
    Dropped(MediaObservation),
    Eof,
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn read_shared_media_event(
    reader: &mut impl Read,
    consumer: &SharedMediaLaneConsumer,
    lane: MediaLane,
    conversation: &Arc<Mutex<MediaConversationValidator>>,
    generation_nonce: [u8; 16],
    transport_pool: &DetachedMediaBufferPool,
) -> Result<SharedMediaRead, FrameError> {
    let Some(notification) = SharedSlotNotification::read_from(reader, lane)
        .map_err(|error| FrameError::Decode(error.to_string()))?
    else {
        return Ok(SharedMediaRead::Eof);
    };
    let ticket = match notification {
        SharedSlotNotification::Published(ticket) => ticket,
        SharedSlotNotification::Dropped { identity } => {
            let observation = lock_conversation(conversation)?
                .observe_backpressure_drop(
                    lane,
                    identity.sequence,
                    identity.observed_at_ms,
                    identity.media_gate,
                )
                .map_err(|error| FrameError::Decode(error.to_string()))?;
            return Ok(SharedMediaRead::Dropped(observation));
        }
    };
    let lease = match consumer.claim(ticket) {
        Ok(lease) => lease,
        Err(SharedMediaLaneError::StaleTicket | SharedMediaLaneError::SlotUnavailable) => {
            let identity = ticket.identity;
            let observation = lock_conversation(conversation)?
                .observe_backpressure_drop(
                    lane,
                    identity.sequence,
                    identity.observed_at_ms,
                    identity.media_gate,
                )
                .map_err(|error| FrameError::Decode(error.to_string()))?;
            return Ok(SharedMediaRead::Dropped(observation));
        }
        Err(error) => return Err(FrameError::Decode(error.to_string())),
    };
    let frame = Bytes::from_owner(lease);
    let frame_start = frame.as_ptr() as usize;
    let (metadata, payload_view) =
        decode_binary_media_event_frame_compact(&frame, lane, generation_nonce)?;
    let payload_start = (payload_view.as_ptr() as usize)
        .checked_sub(frame_start)
        .ok_or_else(|| FrameError::Decode("shared payload pointer precedes its frame".into()))?;
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
    let payload_view = &frame[payload_start..payload_end];
    let observation = lock_conversation(conversation)?
        .observe_binary_media(lane, &metadata, payload_view)
        .map_err(|error| FrameError::Decode(error.to_string()))?;
    // The shared slot is a short-lived process-isolation lease, while the
    // WebRTC packetizer and NACK history may retain payload bytes beyond this
    // receive turn. Detach exactly once at that ownership boundary so network
    // retransmission lifetime cannot pin the producer's bounded shared ring.
    let payload = Bytes::from_owner(transport_pool.copy_from_slice(payload_view));
    drop(frame);
    Ok(SharedMediaRead::Event(MediaHostMediaEvent {
        metadata,
        payload,
        observation,
    }))
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn validate_media_message(
    lane: MediaLane,
    message: Result<Option<(EventMetadata, Vec<u8>)>, FrameError>,
    conversation: &Arc<Mutex<MediaConversationValidator>>,
) -> Result<Option<MediaHostEvent>, FrameError> {
    match message {
        Ok(Some((metadata, payload))) => {
            let observation = lock_conversation(conversation)?
                .observe(lane, &metadata, &payload)
                .map_err(|error| FrameError::Decode(error.to_string()))?;
            debug_assert_eq!(observation, MediaObservation::Accepted);
            debug_assert!(payload.is_empty());
            Ok(Some(MediaHostEvent { metadata }))
        }
        Ok(None) => {
            if lane == MediaLane::Control {
                lock_conversation(conversation)?
                    .finish_control_eof()
                    .map_err(|_| FrameError::UnexpectedEof)?;
            }
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn lock_conversation(
    conversation: &Arc<Mutex<MediaConversationValidator>>,
) -> Result<MutexGuard<'_, MediaConversationValidator>, FrameError> {
    conversation
        .lock()
        .map_err(|_| FrameError::Decode("media conversation validator lock poisoned".into()))
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn require_media_event<T>(message: Result<Option<T>, FrameError>) -> Result<T, FrameError> {
    match message {
        Ok(Some(event)) => Ok(event),
        Ok(None) => Err(FrameError::UnexpectedEof),
        Err(error) => Err(error),
    }
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
struct VideoEventMailbox {
    capacity: usize,
    state: Mutex<VideoMailboxState>,
    frames_dropped: AtomicU64,
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
#[derive(Default)]
struct VideoMailboxState {
    queue: VecDeque<Result<Option<MediaHostMediaEvent>, FrameError>>,
    recovery: VideoRecoveryState,
    queued_recovery: bool,
    closed: bool,
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
#[derive(Debug, Default)]
enum VideoRecoveryState {
    #[default]
    Stable,
    RequestPending,
    AwaitingIdr {
        requested_at: Instant,
    },
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
impl VideoEventMailbox {
    fn new(capacity: usize) -> Self {
        debug_assert!(capacity > 0);
        Self {
            capacity,
            state: Mutex::new(VideoMailboxState::default()),
            frames_dropped: AtomicU64::new(0),
        }
    }

    fn push(&self, event: MediaHostMediaEvent) {
        let recovery_point = matches!(
            event.metadata.body,
            easynet_remoteapp_native_protocol::media_session::EventBody::VideoH264 {
                keyframe: true,
                sps_pps_present: true,
                ..
            }
        );
        if event.observation == MediaObservation::StaleDiscarded {
            if recovery_point {
                eprintln!(
                    "[remoteapp-media-recovery] kind=recovery_idr_stale_discarded sequence={}",
                    event.metadata.sequence
                );
            }
            self.frames_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.closed {
            self.frames_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if !matches!(state.recovery, VideoRecoveryState::Stable) {
            if recovery_point {
                eprintln!(
                    "[remoteapp-media-recovery] kind=recovery_idr_queued sequence={} prior_state={:?}",
                    event.metadata.sequence, state.recovery
                );
                let discarded = state.queue.len() as u64;
                state.queue.clear();
                self.frames_dropped.fetch_add(discarded, Ordering::Relaxed);
                state.recovery = VideoRecoveryState::Stable;
                state.queue.push_back(Ok(Some(event)));
                state.queued_recovery = true;
            } else {
                self.frames_dropped.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
        if state.queue.len() >= self.capacity {
            if !recovery_point && state.queued_recovery {
                let queued = state.queue.len();
                state.queue.retain(is_queued_recovery_event);
                let discarded = queued.saturating_sub(state.queue.len()) as u64;
                self.frames_dropped
                    .fetch_add(discarded.saturating_add(1), Ordering::Relaxed);
                return;
            }
            let discarded = state.queue.len() as u64;
            state.queue.clear();
            self.frames_dropped.fetch_add(
                discarded.saturating_add(u64::from(!recovery_point)),
                Ordering::Relaxed,
            );
            if recovery_point {
                eprintln!(
                    "[remoteapp-media-recovery] kind=recovery_idr_replaced_full_queue sequence={}",
                    event.metadata.sequence
                );
                state.queue.push_back(Ok(Some(event)));
                state.queued_recovery = true;
            } else {
                state.queued_recovery = false;
                eprintln!(
                    "[remoteapp-media-recovery] kind=recovery_request_pending source=mailbox_overflow"
                );
                state.recovery = VideoRecoveryState::RequestPending;
            }
            return;
        }
        state.queue.push_back(Ok(Some(event)));
        state.queued_recovery |= recovery_point;
    }

    fn register_transport_drop(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.closed {
            return;
        }
        let queued = state.queue.len();
        if state.queued_recovery {
            state.queue.retain(is_queued_recovery_event);
        } else {
            state.queue.clear();
        }
        let discarded = queued.saturating_sub(state.queue.len()) as u64;
        self.frames_dropped
            .fetch_add(discarded.saturating_add(1), Ordering::Relaxed);
        if state.queued_recovery {
            state.recovery = VideoRecoveryState::Stable;
            return;
        }
        if matches!(state.recovery, VideoRecoveryState::Stable) {
            eprintln!(
                "[remoteapp-media-recovery] kind=recovery_request_pending source=shared_lane_drop"
            );
            state.recovery = VideoRecoveryState::RequestPending;
        }
    }

    fn close(&self, terminal: Result<Option<MediaHostMediaEvent>, FrameError>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.closed {
            return;
        }
        let discarded = state.queue.len() as u64;
        state.queue.clear();
        state.queued_recovery = false;
        self.frames_dropped.fetch_add(discarded, Ordering::Relaxed);
        state.queue.push_back(terminal);
        state.closed = true;
    }

    fn try_pop(&self) -> Option<Result<Option<MediaHostMediaEvent>, FrameError>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let event = state.queue.pop_front();
        if event.as_ref().is_some_and(is_queued_recovery_event) {
            state.queued_recovery = state.queue.iter().any(is_queued_recovery_event);
        }
        event
    }

    fn is_closed(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.closed && state.queue.is_empty()
    }

    fn take_recovery_request(&self) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(state.recovery, VideoRecoveryState::RequestPending) {
            eprintln!("[remoteapp-media-recovery] kind=recovery_request_dispatched");
            state.recovery = VideoRecoveryState::AwaitingIdr {
                requested_at: Instant::now(),
            };
            true
        } else {
            false
        }
    }

    fn frames_dropped(&self) -> u64 {
        self.frames_dropped.load(Ordering::Acquire)
    }

    fn depth(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .queue
            .iter()
            .filter(|item| matches!(item, Ok(Some(_))))
            .count()
    }

    fn recovery_overdue(&self, deadline: Duration) -> bool {
        matches!(
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recovery,
            VideoRecoveryState::AwaitingIdr { requested_at }
                if requested_at.elapsed() >= deadline
        )
    }
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn is_queued_recovery_event(event: &Result<Option<MediaHostMediaEvent>, FrameError>) -> bool {
    matches!(
        event,
        Ok(Some(MediaHostMediaEvent {
            metadata: easynet_remoteapp_native_protocol::media_session::BinaryMediaEvent {
                body: easynet_remoteapp_native_protocol::media_session::EventBody::VideoH264 {
                    keyframe: true,
                    sps_pps_present: true,
                    ..
                },
                ..
            },
            ..
        }))
    )
}

#[cfg(all(
    unix,
    feature = "native-media",
    any(target_os = "linux", target_os = "macos")
))]
struct ConfiguredMediaLane {
    child_write: std::fs::File,
    parent_read: std::fs::File,
}

#[cfg(all(
    unix,
    feature = "native-media",
    any(target_os = "linux", target_os = "macos")
))]
fn configure_media_output_lane(
    command: &mut Command,
    environment_name: &'static str,
) -> io::Result<ConfiguredMediaLane> {
    use std::os::fd::FromRawFd;
    use std::os::unix::process::CommandExt;

    let mut fds = [-1_i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let read_fd = fds[0];
    let write_fd = fds[1];
    if unsafe { libc::fcntl(read_fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        let error = io::Error::last_os_error();
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return Err(error);
    }
    command.env(environment_name, write_fd.to_string());
    unsafe {
        command.pre_exec(move || {
            libc::close(read_fd);
            Ok(())
        });
    }
    Ok(ConfiguredMediaLane {
        child_write: unsafe { std::fs::File::from_raw_fd(write_fd) },
        parent_read: unsafe { std::fs::File::from_raw_fd(read_fd) },
    })
}

#[cfg(all(
    unix,
    feature = "native-media",
    any(target_os = "linux", target_os = "macos")
))]
fn configure_shared_media_lane(
    command: &mut Command,
    environment_name: &'static str,
    lane: &SharedMediaLaneFile,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;

    let fd = lane.as_raw_fd();
    command.env(environment_name, fd.to_string());
    unsafe {
        command.pre_exec(move || {
            if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn shared_lane_io_error(error: SharedMediaLaneError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn sha256_file(path: &std::path::Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};

    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let bytes = digest.finalize();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(value)
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
pub(super) fn media_host_build_id(executable_name: &str) -> io::Result<String> {
    sha256_file(&sibling_executable(executable_name)?)
}

fn terminate_spawn_failure(child: &mut Child, stdout_reader: Option<JoinHandle<()>>) {
    child.stdin.take();
    let _ = child.kill();
    let _ = child.wait();
    if let Some(reader) = stdout_reader {
        let _ = reader.join();
    }
}

pub(super) fn sibling_executable(name: &str) -> io::Result<std::path::PathBuf> {
    let current = std::env::current_exe()?;
    let directory = current
        .parent()
        .ok_or_else(|| io::Error::other("current executable has no directory for native host"))?;
    #[allow(unused_mut)]
    let mut candidate = sibling_executable_in(directory, name);
    #[cfg(test)]
    if !candidate.is_file() && directory.file_name().is_some_and(|name| name == "deps") {
        if let Some(profile_directory) = directory.parent() {
            candidate = sibling_executable_in(profile_directory, name);
            #[cfg(target_os = "macos")]
            if !candidate.is_file() && name == super::MEDIA_HOST_EXECUTABLE {
                // Unit/integration tests execute Cargo's directly built helper;
                // production daemon builds never compile this branch.
                candidate = profile_directory.join(name);
            }
        }
    }
    if !candidate.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "required RemoteApp native host is not installed beside the daemon: {}",
                candidate.display()
            ),
        ));
    }
    Ok(candidate)
}

fn sibling_executable_in(directory: &std::path::Path, name: &str) -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    if name == super::MEDIA_HOST_EXECUTABLE {
        return directory
            .join(format!("{name}.app"))
            .join("Contents")
            .join("MacOS")
            .join(name);
    }
    directory.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    })
}

fn project_os_bootstrap_environment(_command: &mut Command) {
    #[cfg(target_os = "linux")]
    for name in [
        "DISPLAY",
        "XAUTHORITY",
        "XDG_RUNTIME_DIR",
        "PIPEWIRE_REMOTE",
    ] {
        if let Some(value) = std::env::var_os(name) {
            _command.env(name, value);
        }
    }
    #[cfg(windows)]
    for name in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            _command.env(name, value);
        }
    }
}

#[cfg(unix)]
fn configure_parent_liveness(command: &mut Command) -> io::Result<(i32, std::fs::File)> {
    use std::os::fd::FromRawFd;
    use std::os::unix::process::CommandExt;

    let mut fds = [-1_i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let read_fd = fds[0];
    let write_fd = fds[1];
    if unsafe { libc::fcntl(write_fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        let error = io::Error::last_os_error();
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return Err(error);
    }
    command.env(PARENT_LIVENESS_FD_ENV, read_fd.to_string());
    unsafe {
        command.pre_exec(move || {
            libc::close(write_fd);
            #[cfg(target_os = "linux")]
            {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() == 1 {
                    return Err(io::Error::other(
                        "RemoteApp native-host parent exited before exec",
                    ));
                }
            }
            Ok(())
        });
    }
    Ok((read_fd, unsafe { std::fs::File::from_raw_fd(write_fd) }))
}

#[cfg(windows)]
fn assign_kill_on_close_job(child: &Child) -> io::Result<WindowsKillOnCloseJob> {
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const std::ffi::c_void,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    let process = child.as_raw_handle() as HANDLE;
    let assigned = configured != 0 && unsafe { AssignProcessToJobObject(job, process) } != 0;
    if !assigned {
        let error = io::Error::last_os_error();
        unsafe { CloseHandle(job) };
        return Err(error);
    }
    Ok(WindowsKillOnCloseJob(job))
}

pub(super) fn read_responses<Response: DeserializeOwned>(
    mut stdout: impl Read,
    response_tx: SyncSender<Result<Response, FrameError>>,
    protocol_violation: Arc<AtomicBool>,
) {
    loop {
        match read_frame(&mut stdout) {
            Ok(Some(response)) => match response_tx.try_send(Ok(response)) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    protocol_violation.store(true, Ordering::Release);
                    return;
                }
                Err(TrySendError::Disconnected(_)) => return,
            },
            Ok(None) => return,
            Err(error) => {
                let _ = response_tx.try_send(Err(error));
                return;
            }
        }
    }
}

fn read_capped_diagnostics(mut reader: impl Read, max_bytes: usize) -> Vec<u8> {
    let mut diagnostics = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => return diagnostics,
            Ok(read) => {
                let remaining = max_bytes.saturating_sub(diagnostics.len());
                diagnostics.extend_from_slice(&buffer[..read.min(remaining)]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    struct TestResponse {
        request_id: u64,
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn media_host_sibling_resolves_to_the_application_executable() {
        let directory = std::path::Path::new("/opt/easynet/bin");
        assert_eq!(
            sibling_executable_in(directory, super::super::MEDIA_HOST_EXECUTABLE),
            directory
                .join("easynet-remoteapp-media-host.app")
                .join("Contents")
                .join("MacOS")
                .join("easynet-remoteapp-media-host")
        );
        assert_eq!(
            sibling_executable_in(directory, "easynet-remoteapp-native-host"),
            directory.join("easynet-remoteapp-native-host")
        );
    }

    #[test]
    fn unsolicited_response_marks_protocol_violation_without_blocking_reader() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &TestResponse { request_id: 1 }).unwrap();
        write_frame(&mut bytes, &TestResponse { request_id: 2 }).unwrap();
        let (tx, rx) = mpsc::sync_channel::<Result<TestResponse, FrameError>>(1);
        let violation = Arc::new(AtomicBool::new(false));
        read_responses(Cursor::new(bytes), tx, Arc::clone(&violation));
        assert!(violation.load(Ordering::Acquire));
        assert_eq!(rx.try_recv().unwrap().unwrap().request_id, 1);
    }

    #[cfg(all(
        unix,
        feature = "native-media",
        any(target_os = "linux", target_os = "macos")
    ))]
    fn queued_video(sequence: u64, recovery_point: bool) -> MediaHostMediaEvent {
        use easynet_remoteapp_native_protocol::media_session::{BinaryMediaEvent, EventBody};

        MediaHostMediaEvent {
            metadata: BinaryMediaEvent {
                sequence,
                observed_at_ms: sequence,
                body: EventBody::VideoH264 {
                    media_gate: 1,
                    pts_90khz: sequence * 3_000,
                    duration_90khz: 3_000,
                    keyframe: recovery_point,
                    sps_pps_present: recovery_point,
                    discontinuity: recovery_point,
                    codec_generation: 1,
                    width: 640,
                    height: 360,
                    encode_submitted_at_ms: sequence,
                    encoded_at_ms: sequence,
                },
            },
            payload: vec![0, 0, 0, 1, if recovery_point { 0x65 } else { 0x41 }].into(),
            observation: MediaObservation::Accepted,
        }
    }

    #[cfg(all(
        unix,
        feature = "native-media",
        any(target_os = "linux", target_os = "macos")
    ))]
    #[test]
    fn video_backpressure_drops_dependency_chain_until_recovery_idr() {
        let mailbox = VideoEventMailbox::new(3);
        mailbox.push(queued_video(1, false));
        mailbox.push(queued_video(2, false));
        mailbox.push(queued_video(3, false));
        assert_eq!(mailbox.depth(), 3);

        mailbox.push(queued_video(4, false));
        assert_eq!(mailbox.depth(), 0);
        assert!(!mailbox.recovery_overdue(Duration::ZERO));
        assert!(mailbox.take_recovery_request());
        assert!(mailbox.recovery_overdue(Duration::ZERO));
        assert!(!mailbox.take_recovery_request());
        assert_eq!(mailbox.frames_dropped(), 4);

        mailbox.push(queued_video(5, false));
        assert_eq!(mailbox.depth(), 0);
        assert_eq!(mailbox.frames_dropped(), 5);

        mailbox.push(queued_video(6, true));
        assert_eq!(mailbox.depth(), 1);
        assert!(!mailbox.recovery_overdue(Duration::ZERO));
        assert!(!mailbox.take_recovery_request());
        let recovered = require_media_event(mailbox.try_pop().unwrap()).unwrap();
        assert!(matches!(
            recovered.metadata.body,
            easynet_remoteapp_native_protocol::media_session::EventBody::VideoH264 {
                keyframe: true,
                sps_pps_present: true,
                ..
            }
        ));
    }

    #[cfg(all(
        unix,
        feature = "native-media",
        any(target_os = "linux", target_os = "macos")
    ))]
    #[test]
    fn queued_recovery_idr_replaces_full_gop_without_restart() {
        let mailbox = VideoEventMailbox::new(3);
        mailbox.push(queued_video(1, false));
        mailbox.push(queued_video(2, false));
        mailbox.push(queued_video(3, false));
        mailbox.push(queued_video(4, true));

        assert_eq!(mailbox.depth(), 1);
        assert_eq!(mailbox.frames_dropped(), 3);
        assert!(!mailbox.take_recovery_request());
        let recovered = require_media_event(mailbox.try_pop().unwrap()).unwrap();
        assert_eq!(recovered.metadata.sequence, 4);
    }

    #[cfg(all(
        unix,
        feature = "native-media",
        any(target_os = "linux", target_os = "macos")
    ))]
    #[test]
    fn queued_recovery_idr_survives_following_delta_overflow() {
        let mailbox = VideoEventMailbox::new(3);
        mailbox.push(queued_video(1, true));
        mailbox.push(queued_video(2, false));
        mailbox.push(queued_video(3, false));
        mailbox.push(queued_video(4, false));

        assert_eq!(mailbox.depth(), 1);
        assert_eq!(mailbox.frames_dropped(), 3);
        assert!(!mailbox.take_recovery_request());
        assert!(!mailbox.recovery_overdue(Duration::ZERO));
        let recovered = require_media_event(mailbox.try_pop().unwrap()).unwrap();
        assert_eq!(recovered.metadata.sequence, 1);
    }

    #[cfg(all(
        unix,
        feature = "native-media",
        any(target_os = "linux", target_os = "macos")
    ))]
    #[test]
    fn queued_recovery_idr_survives_later_shared_lane_drop() {
        let mailbox = VideoEventMailbox::new(3);
        mailbox.push(queued_video(1, true));
        mailbox.push(queued_video(2, false));
        mailbox.push(queued_video(3, false));
        mailbox.register_transport_drop();

        assert_eq!(mailbox.depth(), 1);
        assert_eq!(mailbox.frames_dropped(), 3);
        assert!(!mailbox.take_recovery_request());
        let recovered = require_media_event(mailbox.try_pop().unwrap()).unwrap();
        assert_eq!(recovered.metadata.sequence, 1);
    }
}

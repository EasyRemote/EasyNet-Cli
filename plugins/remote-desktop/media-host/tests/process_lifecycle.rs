//! Real-process failure semantics for the canonical RemoteApp media host.
//!
//! These tests exercise the packaged helper process and its physical liveness
//! descriptor. Fault injection is compiled only for this explicit E2E feature;
//! ordinary product binaries have no injectable crash or hang surface.

#![cfg(all(unix, feature = "remoteapp-e2e-fault-injection"))]

use std::fs::File;
use std::io::Read;
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{
    Child, ChildStderr, ChildStdin, ChildStdout, Command as ProcessCommand, ExitStatus, Stdio,
};
use std::thread;
use std::time::{Duration, Instant};

use easynet_remoteapp_native_protocol::media_session::{
    write_command_frame, Command, CommandBody, GenerationFence, NativeTargetPlan, StartContract,
    TargetKind, VideoCodec, VideoConfig, AUDIO_LANE_FD_ENV, PROTOCOL, SCHEMA_VERSION,
    VIDEO_LANE_FD_ENV,
};
use easynet_remoteapp_native_protocol::screen_capture_permission::{
    Operation as ScreenCapturePermissionOperation, Request as ScreenCapturePermissionRequest,
    Response as ScreenCapturePermissionResponse,
};
use easynet_remoteapp_native_protocol::{read_frame, write_frame, PARENT_LIVENESS_FD_ENV};

const FAULT_ENV: &str = "EASYNET_REMOTEAPP_MEDIA_HOST_TEST_FAULT";
const EXIT_DEADLINE: Duration = Duration::from_secs(3);

struct ProcessHarness {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    stderr: ChildStderr,
    parent_liveness: Option<File>,
    _video_lane: File,
    _audio_lane: File,
}

impl ProcessHarness {
    fn spawn(fault: Option<&str>) -> anyhow::Result<Self> {
        let binary = env!("CARGO_BIN_EXE_easynet-remoteapp-media-host");
        let (liveness_read, liveness_write) = pipe()?;
        let (video_read, video_write) = pipe()?;
        let (audio_read, audio_write) = pipe()?;
        let mut command = ProcessCommand::new(binary);
        command
            .env_clear()
            .env(PARENT_LIVENESS_FD_ENV, liveness_read.to_string())
            .env(VIDEO_LANE_FD_ENV, video_write.to_string())
            .env(AUDIO_LANE_FD_ENV, audio_write.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(fault) = fault {
            command.env(FAULT_ENV, fault);
        }
        unsafe {
            command.pre_exec(move || {
                libc::close(liveness_write);
                libc::close(video_read);
                libc::close(audio_read);
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
            .ok_or_else(|| anyhow::anyhow!("media-host diagnostic lane missing"))?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout,
            stderr,
            parent_liveness: Some(unsafe { File::from_raw_fd(liveness_write) }),
            _video_lane: unsafe { File::from_raw_fd(video_read) },
            _audio_lane: unsafe { File::from_raw_fd(audio_read) },
        })
    }

    fn send_initial(&mut self) -> anyhow::Result<()> {
        write_command_frame(
            self.stdin
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("media-host command lane closed"))?,
            &initial_command(),
        )?;
        Ok(())
    }

    fn wait(&mut self) -> anyhow::Result<ExitStatus> {
        let deadline = Instant::now() + EXIT_DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "media-host did not terminate within {:?}",
                EXIT_DEADLINE
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn read_outputs(&mut self) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        let mut control = Vec::new();
        let mut diagnostics = Vec::new();
        self.stdout.read_to_end(&mut control)?;
        self.stderr.read_to_end(&mut diagnostics)?;
        Ok((control, diagnostics))
    }
}

impl Drop for ProcessHarness {
    fn drop(&mut self) {
        self.stdin.take();
        self.parent_liveness.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn injected_session_crash_closes_control_without_a_false_terminal() -> anyhow::Result<()> {
    let mut host = ProcessHarness::spawn(Some("crash"))?;
    host.send_initial()?;
    let status = host.wait()?;
    let (control, diagnostics) = host.read_outputs()?;
    anyhow::ensure!(!status.success(), "injected crash exited successfully");
    anyhow::ensure!(
        control.is_empty(),
        "crashed helper fabricated a protocol terminal frame"
    );
    anyhow::ensure!(
        String::from_utf8_lossy(&diagnostics).contains("injected media-host crash"),
        "crash diagnostic did not identify the injected boundary"
    );
    Ok(())
}

#[test]
fn parent_liveness_eof_kills_a_hung_session_generation() -> anyhow::Result<()> {
    let mut host = ProcessHarness::spawn(Some("hang"))?;
    host.send_initial()?;
    host.parent_liveness.take();
    let status = host.wait()?;
    anyhow::ensure!(
        status.code() == Some(125),
        "parent-liveness watchdog exited with {status}, expected code 125"
    );
    Ok(())
}

#[test]
fn real_process_permission_status_reports_the_media_host_identity() -> anyhow::Result<()> {
    let mut host = ProcessHarness::spawn(None)?;
    let request =
        ScreenCapturePermissionRequest::new(7, 11, ScreenCapturePermissionOperation::Status);
    write_frame(
        host.stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("media-host command lane closed"))?,
        &request,
    )?;
    host.stdin.take();
    let status = host.wait()?;
    let (control, diagnostics) = host.read_outputs()?;
    anyhow::ensure!(
        status.success(),
        "permission helper failed: {}",
        String::from_utf8_lossy(&diagnostics)
    );
    let mut control = control.as_slice();
    let response: ScreenCapturePermissionResponse = read_frame(&mut control)?
        .ok_or_else(|| anyhow::anyhow!("permission helper returned no response"))?;
    anyhow::ensure!(read_frame::<ScreenCapturePermissionResponse>(&mut control)?.is_none());
    anyhow::ensure!(response.matches_request(&request));
    anyhow::ensure!(!response.requested);
    anyhow::ensure!(response.previously_granted == response.granted);
    anyhow::ensure!(response.executable_path.as_deref().is_some_and(|path| {
        path.contains("easynet-remoteapp-media-host")
            || path.contains("easynet_remoteapp_media_host")
    }));
    Ok(())
}

fn initial_command() -> Command {
    let contract = StartContract {
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
    };
    Command {
        schema_version: SCHEMA_VERSION,
        protocol: PROTOCOL.to_string(),
        fence: GenerationFence {
            process_generation: 1,
            build_id: "1".repeat(64),
            session_nonce: "2".repeat(32),
            transport_epoch: 1,
            media_source_epoch: 1,
            contract_digest: contract.digest().expect("test contract must validate"),
        },
        sequence: 1,
        body: CommandBody::StartPrepared { contract },
    }
}

fn pipe() -> std::io::Result<(RawFd, RawFd)> {
    let mut fds = [-1_i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((fds[0], fds[1]))
}

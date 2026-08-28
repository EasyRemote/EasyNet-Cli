// EasyNet CLI — target-scoped RemoteApp host-audio capture
// =========================================================
//
// File: plugins/remote-desktop/src/media/host_audio.rs
// Description: Derives and owns Windows/Linux host-audio capture for a bound
// RemoteApp target.
//
// Protocol Responsibility:
// - None. The RemoteDesktop session aggregate owns authority, consent,
//   transport epochs and terminal lifecycle.
//
// Implementation Approach:
// - Derive a fail-closed source plan from the committed target kind and PID.
// - Normalize platform audio to bounded 48 kHz stereo 20 ms chunks through a
//   dedicated capture engine, then publish chunks through the shared AudioSink.
// - Keep one capture stream per transport generation. Rebind pauses delivery,
//   transactionally switches its backend, and resumes only after the session
//   commits the new media-source epoch.
//
// Usage Contract:
// - Display capture is system loopback. Window/application capture is scoped
//   to the bound PID and never widens to system loopback.
// - Linux window/application capture fans in every PipeWire output node owned
//   by the start-time-anchored process tree and periodically revokes stale
//   links independently of PipeWire graph activity.
//
// Architectural Position:
// - RemoteDesktop plugin platform adapter below the shared codec/transport and
//   above native WASAPI/PipeWire capture mechanics.

#[cfg(all(feature = "native-media", target_os = "windows"))]
use crate::daemon::plugins::remote_desktop::media::audio::{AudioSink, CapturedAudioChunk};
use crate::daemon::plugins::remote_desktop::media::host_audio_capability::HostAudioSourceClass;
use crate::daemon::plugins::remote_desktop::target::RemoteAppTargetBinding;
use crate::daemon::plugins::remote_desktop::target::RemoteDesktopTargetKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostAudioSourceKind {
    SystemLoopback,
    ProcessTreeLoopback { pid: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(in crate::daemon::plugins::remote_desktop) enum HostAudioSourcePlanError {
    #[error("baseline host audio is not available for display targets on {platform}")]
    DisplayUnsupported { platform: String },
    #[error("target-scoped host audio requires native_locator.pid for {target_kind}")]
    TargetPidMissing { target_kind: &'static str },
    #[error("target-scoped host audio requires a positive u32 PID, got {pid}")]
    TargetPidInvalid { pid: i64 },
    #[error("baseline target-scoped host audio is not available on {platform}")]
    TargetScopedUnsupported { platform: String },
}

impl HostAudioSourcePlanError {
    pub(in crate::daemon::plugins::remote_desktop) const fn reason_code(&self) -> &'static str {
        match self {
            Self::DisplayUnsupported { .. } | Self::TargetScopedUnsupported { .. } => {
                "baseline_host_audio_unavailable"
            }
            Self::TargetPidMissing { .. } => "target_scoped_host_audio_pid_missing",
            Self::TargetPidInvalid { .. } => "target_scoped_host_audio_pid_invalid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct HostAudioSourcePlan {
    source: HostAudioSourceKind,
}

impl HostAudioSourcePlan {
    #[cfg_attr(
        not(all(
            feature = "native-media",
            any(target_os = "windows", target_os = "linux")
        )),
        allow(dead_code)
    )]
    pub(in crate::daemon::plugins::remote_desktop) fn for_binding(
        binding: &RemoteAppTargetBinding,
    ) -> Result<Self, HostAudioSourcePlanError> {
        Self::for_target(
            std::env::consts::OS,
            binding.target_kind(),
            binding.native_locator().pid(),
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn for_target(
        platform: &str,
        target_kind: RemoteDesktopTargetKind,
        native_pid: Option<i64>,
    ) -> Result<Self, HostAudioSourcePlanError> {
        match target_kind {
            RemoteDesktopTargetKind::Display => match platform {
                "windows" | "linux" => Ok(Self {
                    source: HostAudioSourceKind::SystemLoopback,
                }),
                _ => Err(HostAudioSourcePlanError::DisplayUnsupported {
                    platform: platform.to_string(),
                }),
            },
            RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
                let pid = native_pid.ok_or(HostAudioSourcePlanError::TargetPidMissing {
                    target_kind: target_kind.as_str(),
                })?;
                let pid = u32::try_from(pid)
                    .ok()
                    .filter(|pid| *pid > 0)
                    .ok_or(HostAudioSourcePlanError::TargetPidInvalid { pid })?;
                match platform {
                    "windows" => Ok(Self {
                        source: HostAudioSourceKind::ProcessTreeLoopback { pid },
                    }),
                    "linux" => Ok(Self {
                        source: HostAudioSourceKind::ProcessTreeLoopback { pid },
                    }),
                    _ => Err(HostAudioSourcePlanError::TargetScopedUnsupported {
                        platform: platform.to_string(),
                    }),
                }
            }
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn source_label(self) -> &'static str {
        match self.source {
            HostAudioSourceKind::SystemLoopback => "system_loopback",
            HostAudioSourceKind::ProcessTreeLoopback { .. } => "process_tree_loopback",
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn source_class(
        self,
    ) -> HostAudioSourceClass {
        HostAudioSourceClass::for_target_kind(match self.source {
            HostAudioSourceKind::SystemLoopback => RemoteDesktopTargetKind::Display,
            HostAudioSourceKind::ProcessTreeLoopback { .. } => RemoteDesktopTargetKind::Application,
        })
    }
}

#[cfg(all(feature = "native-media", target_os = "windows"))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct HostAudioCaptureStats {
    pub(in crate::daemon::plugins::remote_desktop) source: Option<&'static str>,
    pub(in crate::daemon::plugins::remote_desktop) chunks_forwarded: u64,
    pub(in crate::daemon::plugins::remote_desktop) backend_chunks_dropped: u64,
    pub(in crate::daemon::plugins::remote_desktop) stall_events: u64,
    pub(in crate::daemon::plugins::remote_desktop) recovery_events: u64,
    pub(in crate::daemon::plugins::remote_desktop) precommit_chunks_discarded: u64,
    pub(in crate::daemon::plugins::remote_desktop) terminal_error: Option<String>,
}

#[cfg(all(feature = "native-media", target_os = "windows"))]
#[derive(Debug, Clone)]
struct HostAudioStreamPlan {
    source: HostAudioSourcePlan,
    config: flexaudio::StreamConfig,
}

#[cfg(all(feature = "native-media", target_os = "windows"))]
impl HostAudioStreamPlan {
    fn for_binding(binding: &RemoteAppTargetBinding) -> anyhow::Result<Self> {
        use flexaudio::{OutputFormat, ProcessMode, SourceKind, StreamConfig};

        let source = HostAudioSourcePlan::for_binding(binding)?;
        let (kind, target_pid) = match source.source {
            // Display means the selected/default output mix. `exclude_self`
            // would replace that monitor with an application-node fan-in on
            // Linux and silently omit non-application/system sounds.
            HostAudioSourceKind::SystemLoopback => (SourceKind::SystemLoopback, None),
            HostAudioSourceKind::ProcessTreeLoopback { pid } => {
                (SourceKind::ProcessLoopback, Some(pid))
            }
        };
        Ok(Self {
            source,
            config: StreamConfig {
                kind,
                target_pid,
                mode: ProcessMode::Include,
                exclude_self: false,
                output: OutputFormat {
                    sample_rate: 48_000,
                    channels: 2,
                },
                // The WebRTC pipeline is independently bounded. Four 20 ms
                // chunks retain jitter tolerance without stale accumulation.
                ring_capacity_chunks: 4,
                ..Default::default()
            },
        })
    }

    fn open(&self) -> anyhow::Result<flexaudio::Stream> {
        #[cfg(target_os = "linux")]
        {
            return flexaudio::Stream::open(self.config.clone(), self.linux_backend()?)
                .map_err(|error| anyhow::anyhow!("open host-audio stream engine: {error}"));
        }
        #[cfg(target_os = "windows")]
        {
            flexaudio::open(self.config.clone())
                .map_err(|error| anyhow::anyhow!("open host-audio capture: {error}"))
        }
    }

    fn switch(&self, stream: &mut flexaudio::Stream) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            return stream
                .switch_backend(self.linux_backend()?)
                .map_err(|error| anyhow::anyhow!("switch host-audio source: {error}"));
        }
        #[cfg(target_os = "windows")]
        {
            stream
                .switch_source(self.config.clone())
                .map_err(|error| anyhow::anyhow!("switch host-audio source: {error}"))
        }
    }

    #[cfg(target_os = "linux")]
    fn linux_backend(&self) -> anyhow::Result<Box<dyn flexaudio::CaptureBackend>> {
        match self.source.source {
            HostAudioSourceKind::SystemLoopback => Ok(Box::new(
                flexaudio_os_linux::PwSystemBackend::new(false, None),
            )),
            HostAudioSourceKind::ProcessTreeLoopback { pid } => Ok(Box::new(
                crate::daemon::plugins::remote_desktop::media::linux_process_tree_audio::LinuxProcessTreeAudioBackend::new(pid)
                    .map_err(|error| anyhow::anyhow!("prepare Linux process-tree audio backend: {error}"))?,
            )),
        }
    }
}

#[cfg(all(feature = "native-media", target_os = "windows"))]
pub(in crate::daemon::plugins::remote_desktop) struct PreparedHostAudioRebind {
    previous: HostAudioStreamPlan,
    next: HostAudioStreamPlan,
    switched_backend: bool,
}

#[cfg(all(feature = "native-media", target_os = "windows"))]
pub(in crate::daemon::plugins::remote_desktop) struct RunningHostAudioCapture {
    stream: flexaudio::Stream,
    plan: HostAudioStreamPlan,
    admit_after_pts_ns: i64,
    stats: HostAudioCaptureStats,
    terminal_error_delivered: bool,
    stopped: bool,
}

#[cfg(all(feature = "native-media", target_os = "windows"))]
impl RunningHostAudioCapture {
    pub(in crate::daemon::plugins::remote_desktop) fn start(
        binding: &RemoteAppTargetBinding,
    ) -> anyhow::Result<Self> {
        let plan = HostAudioStreamPlan::for_binding(binding)?;
        let mut stream = plan.open()?;
        stream
            .start()
            .map_err(|error| anyhow::anyhow!("start host-audio capture: {error}"))?;
        let mut capture = Self {
            stream,
            admit_after_pts_ns: flexaudio::core::monotonic_now_ns(),
            stats: HostAudioCaptureStats {
                source: Some(plan.source.source_label()),
                ..HostAudioCaptureStats::default()
            },
            plan,
            terminal_error_delivered: false,
            stopped: false,
        };
        capture.discard_pending()?;
        Ok(capture)
    }

    /// Pause delivery and transactionally switch the live backend. The stream
    /// stays paused until either `commit_rebind` or `rollback_rebind`; this is
    /// the capture half of the media-source two-phase commit.
    pub(in crate::daemon::plugins::remote_desktop) fn prepare_rebind(
        &mut self,
        binding: &RemoteAppTargetBinding,
    ) -> anyhow::Result<PreparedHostAudioRebind> {
        let next = HostAudioStreamPlan::for_binding(binding)?;
        let previous = self.plan.clone();
        self.stream.pause();
        if let Err(error) = self.discard_pending() {
            self.stream.resume();
            return Err(error);
        }
        let switched_backend = next.source != previous.source;
        if switched_backend {
            if let Err(error) = next.switch(&mut self.stream) {
                self.stream.resume();
                return Err(error);
            }
        }
        if let Err(error) = self.discard_pending() {
            let rollback = if switched_backend {
                previous.switch(&mut self.stream).err()
            } else {
                None
            };
            self.stream.resume();
            return match rollback {
                Some(rollback) => Err(anyhow::anyhow!(
                    "prepare host-audio rebind failed: {error}; rollback failed: {rollback}"
                )),
                None => Err(error),
            };
        }
        Ok(PreparedHostAudioRebind {
            previous,
            next,
            switched_backend,
        })
    }

    /// Complete a prepared source change after the canonical session commit.
    /// This path is deliberately infallible: all backend work happened during
    /// preparation. PTS admission rejects any delayed pre-commit sample.
    pub(in crate::daemon::plugins::remote_desktop) fn commit_rebind(
        &mut self,
        prepared: PreparedHostAudioRebind,
    ) {
        self.plan = prepared.next;
        self.reset_generation_stats();
        self.arm_postcommit_pts_barrier();
        self.stream.resume();
    }

    /// Restore the old source when the session compare-and-commit loses a
    /// race. Rollback is still paused, so a failure cannot leak the prepared
    /// target into the old canonical media generation.
    pub(in crate::daemon::plugins::remote_desktop) fn rollback_rebind(
        &mut self,
        prepared: PreparedHostAudioRebind,
    ) -> anyhow::Result<()> {
        if prepared.switched_backend {
            prepared.previous.switch(&mut self.stream)?;
        }
        self.plan = prepared.previous;
        self.discard_pending()?;
        self.reset_generation_stats();
        self.arm_postcommit_pts_barrier();
        self.stream.resume();
        Ok(())
    }

    /// Discard media produced before the owning media generation commits.
    pub(in crate::daemon::plugins::remote_desktop) fn discard_pending(
        &mut self,
    ) -> anyhow::Result<()> {
        while self.stream.poll_chunk().is_some() {}
        while let Some(event) = self.stream.poll_event() {
            self.observe_event(event)?;
        }
        Ok(())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn pump(&mut self, sink: &AudioSink) {
        while let Some(event) = self.stream.poll_event() {
            if let Err(error) = self.observe_event(event) {
                self.deliver_terminal_error(sink, error.to_string());
            }
        }
        if self.stats.terminal_error.is_some() {
            while self.stream.poll_chunk().is_some() {}
            return;
        }
        while let Some(chunk) = self.stream.poll_chunk() {
            if chunk.pts_ns < self.admit_after_pts_ns {
                self.stats.precommit_chunks_discarded =
                    self.stats.precommit_chunks_discarded.saturating_add(1);
                continue;
            }
            if chunk.frames != 960 || chunk.data.len() != 1_920 {
                self.deliver_terminal_error(
                    sink,
                    format!(
                        "host-audio normalizer returned invalid 48k stereo chunk: frames={} samples={}",
                        chunk.frames,
                        chunk.data.len()
                    ),
                );
                break;
            }
            self.stats.chunks_forwarded = self.stats.chunks_forwarded.saturating_add(1);
            sink(Ok(CapturedAudioChunk {
                samples: chunk.data,
            }));
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn stats(&self) -> HostAudioCaptureStats {
        self.stats.clone()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn stop(&mut self) {
        if !self.stopped {
            self.stream.stop();
            self.stopped = true;
        }
    }

    fn observe_event(&mut self, event: flexaudio::Event) -> anyhow::Result<()> {
        match event {
            flexaudio::Event::ChunkDropped { count } => {
                self.stats.backend_chunks_dropped =
                    self.stats.backend_chunks_dropped.saturating_add(count);
            }
            flexaudio::Event::StreamStalled => {
                self.stats.stall_events = self.stats.stall_events.saturating_add(1);
            }
            flexaudio::Event::StreamRecovered => {
                self.stats.recovery_events = self.stats.recovery_events.saturating_add(1);
            }
            flexaudio::Event::PermissionDenied => {
                anyhow::bail!("host-audio capture permission denied")
            }
            flexaudio::Event::DeviceLost => anyhow::bail!("host-audio capture device lost"),
            flexaudio::Event::Error(reason) => anyhow::bail!("host-audio backend error: {reason}"),
            _ => {}
        }
        Ok(())
    }

    fn deliver_terminal_error(&mut self, sink: &AudioSink, reason: String) {
        if self.stats.terminal_error.is_none() {
            self.stats.terminal_error = Some(reason.clone());
        }
        if !self.terminal_error_delivered {
            self.terminal_error_delivered = true;
            sink(Err(reason));
        }
        self.stop();
    }

    fn arm_postcommit_pts_barrier(&mut self) {
        self.admit_after_pts_ns = flexaudio::core::monotonic_now_ns();
        while self.stream.poll_chunk().is_some() {}
    }

    fn reset_generation_stats(&mut self) {
        self.stats = HostAudioCaptureStats {
            source: Some(self.plan.source.source_label()),
            ..HostAudioCaptureStats::default()
        };
        self.terminal_error_delivered = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_display_and_process_targets_keep_distinct_audio_scope() {
        assert_eq!(
            HostAudioSourcePlan::for_target("windows", RemoteDesktopTargetKind::Display, None)
                .unwrap()
                .source,
            HostAudioSourceKind::SystemLoopback
        );
        assert_eq!(
            HostAudioSourcePlan::for_target(
                "windows",
                RemoteDesktopTargetKind::Application,
                Some(4242),
            )
            .unwrap()
            .source,
            HostAudioSourceKind::ProcessTreeLoopback { pid: 4242 }
        );
    }

    #[test]
    fn process_target_without_valid_pid_fails_closed() {
        for pid in [None, Some(0), Some(-1), Some(i64::MAX)] {
            let error =
                HostAudioSourcePlan::for_target("windows", RemoteDesktopTargetKind::Window, pid)
                    .unwrap_err();
            assert!(matches!(
                error,
                HostAudioSourcePlanError::TargetPidMissing { .. }
                    | HostAudioSourcePlanError::TargetPidInvalid { .. }
            ));
            assert!(error.reason_code().contains("pid"));
        }
    }

    #[test]
    fn linux_process_target_uses_process_tree_without_widening_to_system_audio() {
        let plan = HostAudioSourcePlan::for_target(
            "linux",
            RemoteDesktopTargetKind::Application,
            Some(4242),
        )
        .unwrap();
        assert_eq!(
            plan.source,
            HostAudioSourceKind::ProcessTreeLoopback { pid: 4242 }
        );
    }
}

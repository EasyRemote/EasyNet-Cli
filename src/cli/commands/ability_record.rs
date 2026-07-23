// EasyNet CLI
// ===========
//
// File: src/cli/ability_record.rs
// Description: Resource-backed media recording wrapper.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use clap::Args;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::core::ura::AbilitySelector;
use crate::daemon::persistence::config::{
    atomic_write_with_permissions, state_dir, WritePermissions,
};
use crate::support::platform::local_invoke::{
    LocalAbilityTarget, LocalDaemonSystemAbilityIssuer, LocalRuntimeStateReadIssuer,
    LocalStreamFrame,
};
use crate::support::platform::{output, timeouts};

const DEFAULT_MAX_FRAMES: usize = 250;
const DEFAULT_CAMERA_RECORD_DURATION_MS: u64 = 5_000;
const CAMERA_RECORDING_GUARD_MS: u64 = 1_000;
const CAMERA_RECORDING_MAX_DURATION_MS: u64 = 30 * 60 * 1_000;

#[derive(Debug, Args)]
pub struct RecordArgs {
    /// Canonical mic.subscribe, camera.subscribe, or camera.record_start Ability URA.
    pub ability_ura: String,
    /// JSON object passed to mic.subscribe or camera.record_start.
    #[arg(long, value_name = "JSON")]
    pub args: Option<String>,
    /// Explicit resource URA. Omit to use the first local resource for the media kind.
    #[arg(long, value_name = "URA")]
    pub subject: Option<String>,
    /// Stop after this many non-terminal mic stream frames.
    #[arg(long, value_name = "N", default_value_t = DEFAULT_MAX_FRAMES)]
    pub max_frames: usize,
    /// Camera recording duration in milliseconds.
    #[arg(long, value_name = "MS", default_value_t = DEFAULT_CAMERA_RECORD_DURATION_MS)]
    pub duration_ms: u64,
    /// Per-invocation transport deadline in seconds. '0' inherits the runtime default.
    #[arg(long, value_name = "SECS", default_value_t = timeouts::INVOKE_DEFAULT_SECS)]
    pub timeout: u64,
    /// Directory where mic stream recording files are created.
    #[arg(long, value_name = "DIR")]
    pub output_dir: Option<PathBuf>,
    /// Print captured payload frames as JSON after recording.
    #[arg(long)]
    pub print_frames: bool,
}

pub fn run(args: RecordArgs) -> anyhow::Result<()> {
    let plan = RecordingPlan::from_args(args)?;
    match plan.kind {
        MediaRecordingKind::Mic => run_mic_stream_recording(&plan),
        MediaRecordingKind::Camera => run_camera_transition_recording(&plan),
    }
}

fn run_mic_stream_recording(plan: &RecordingPlan) -> anyhow::Result<()> {
    let frames = plan.invoke_stream()?;
    let mut sink = RecordingSink::create(plan)?;
    let summary = sink.write_frames(&frames)?;

    if plan.print_frames {
        for frame in frames.iter().filter(|frame| !frame.terminal) {
            println!("{}", serde_json::to_string(&frame.payload)?);
        }
    }

    output::success(&format!(
        "{} -> recorded {} artifact frame(s) from {}",
        plan.selector.ability_ura(),
        summary.artifact_count,
        plan.subject
    ));
    output::detail("directory", &summary.directory.display().to_string());
    output::detail("manifest", &summary.manifest_path.display().to_string());
    output::detail("bytes", &summary.byte_count.to_string());
    Ok(())
}

fn run_camera_transition_recording(plan: &RecordingPlan) -> anyhow::Result<()> {
    let start_selector = sibling_ability_selector(&plan.selector, "camera.record_start")?;
    let stop_selector = sibling_ability_selector(&plan.selector, "camera.record_stop")?;
    let start_target = LocalAbilityTarget::from_selector(&start_selector);
    let stop_target = LocalAbilityTarget::from_selector(&stop_selector);
    let start_args = camera_start_arguments(plan)?;

    let start = LocalDaemonSystemAbilityIssuer::invoke_target_root_timeout(
        &start_target,
        start_args,
        &plan.subject,
        plan.timeout,
    )
    .context("invoke camera.record_start")?;
    let session_id = start
        .get("recording_session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("camera.record_start response missing recording_session_id")
        })?
        .to_string();

    std::thread::sleep(plan.duration);

    let stop = LocalDaemonSystemAbilityIssuer::invoke_target_root_timeout(
        &stop_target,
        json!({ "recording_session_id": session_id }),
        &plan.subject,
        plan.timeout,
    )
    .context("invoke camera.record_stop")?;

    if plan.print_frames {
        println!("{}", serde_json::to_string_pretty(&stop)?);
    }

    output::success(&format!(
        "{} -> recorded camera artifact from {}",
        plan.selector.ability_ura(),
        plan.subject
    ));
    if let Some(local_path) = stop.get("local_path").and_then(Value::as_str) {
        output::detail("local_path", local_path);
    }
    if let Some(frame_count) = stop.get("frame_count").and_then(Value::as_u64) {
        output::detail("frames", &frame_count.to_string());
    }
    if let Some(byte_size) = stop.get("byte_size").and_then(Value::as_u64) {
        output::detail("bytes", &byte_size.to_string());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaRecordingKind {
    Mic,
    Camera,
}

impl MediaRecordingKind {
    fn from_public_name(public_name: &str) -> anyhow::Result<Self> {
        match public_name {
            "mic.subscribe" => Ok(Self::Mic),
            "camera.subscribe" | "camera.record_start" => Ok(Self::Camera),
            other => anyhow::bail!(
                "ability record supports mic.subscribe, camera.subscribe, and camera.record_start; got {other}"
            ),
        }
    }

    fn resource_type(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::Camera => "camera",
        }
    }

    fn directory_name(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::Camera => "camera",
        }
    }

    fn payload_key(self) -> &'static str {
        match self {
            Self::Mic => "samples_b64",
            Self::Camera => "image_bytes_b64",
        }
    }

    fn file_extension(self) -> &'static str {
        match self {
            Self::Mic => "pcm",
            Self::Camera => "jpg",
        }
    }
}

#[derive(Debug)]
struct RecordingPlan {
    selector: AbilitySelector,
    target: LocalAbilityTarget,
    kind: MediaRecordingKind,
    arguments: Value,
    subject: String,
    timeout: Duration,
    duration: Duration,
    max_frames: usize,
    output_dir: Option<PathBuf>,
    print_frames: bool,
}

impl RecordingPlan {
    fn from_args(args: RecordArgs) -> anyhow::Result<Self> {
        if args.max_frames == 0 {
            anyhow::bail!("--max-frames must be greater than 0");
        }
        let selector = AbilitySelector::parse(&args.ability_ura).context("parse <ability-ura>")?;
        let kind = MediaRecordingKind::from_public_name(selector.public_name())?;
        if kind == MediaRecordingKind::Camera && args.max_frames != DEFAULT_MAX_FRAMES {
            anyhow::bail!(
                "--max-frames applies to mic.subscribe; camera recording uses --duration-ms"
            );
        }
        if kind == MediaRecordingKind::Camera && args.output_dir.is_some() {
            anyhow::bail!(
                "--output-dir applies to mic.subscribe; camera recording is persisted by camera.record_stop"
            );
        }
        if kind == MediaRecordingKind::Camera && args.duration_ms == 0 {
            anyhow::bail!("--duration-ms must be greater than 0 for camera recording");
        }
        let arguments = parse_json_args(args.args.as_deref())?;
        let subject = match args
            .subject
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(subject) => subject.to_string(),
            None => default_resource_ura(kind)?,
        };
        let timeout_ms = timeouts::effective_ms(args.timeout)
            .map_err(anyhow::Error::msg)?
            .unwrap_or(timeouts::INVOKE_DEFAULT_SECS * 1000);
        let target = LocalAbilityTarget::from_selector(&selector);

        Ok(Self {
            selector,
            target,
            kind,
            arguments,
            subject,
            timeout: Duration::from_millis(timeout_ms),
            duration: Duration::from_millis(args.duration_ms),
            max_frames: args.max_frames,
            output_dir: args.output_dir,
            print_frames: args.print_frames,
        })
    }

    fn invoke_stream(&self) -> anyhow::Result<Vec<LocalStreamFrame>> {
        LocalDaemonSystemAbilityIssuer::stream_target_root(
            &self.target,
            self.arguments.clone(),
            &self.subject,
            self.timeout,
            Some(self.max_frames),
        )
    }
}

fn parse_json_args(raw: Option<&str>) -> anyhow::Result<Value> {
    match raw {
        Some(s) => serde_json::from_str(s).context("parse --args JSON"),
        None => Ok(Value::Object(Default::default())),
    }
}

fn sibling_ability_selector(
    selector: &AbilitySelector,
    public_name: &str,
) -> anyhow::Result<AbilitySelector> {
    let prefix = selector
        .ability_ura()
        .strip_suffix(selector.public_name())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ability URA {} does not end with public name {}",
                selector.ability_ura(),
                selector.public_name()
            )
        })?;
    AbilitySelector::parse(&format!("{prefix}{public_name}"))
}

fn camera_start_arguments(plan: &RecordingPlan) -> anyhow::Result<Value> {
    let mut value = plan.arguments.clone();
    if !value.is_object() {
        anyhow::bail!("camera recording --args must be a JSON object; got {value}");
    }
    let map = value
        .as_object_mut()
        .expect("checked camera recording args object above");
    map.entry("codec".to_string()).or_insert(json!("mjpeg"));
    map.entry("max_duration_ms".to_string()).or_insert_with(|| {
        json!(plan
            .duration
            .as_millis()
            .saturating_add(u128::from(CAMERA_RECORDING_GUARD_MS))
            .min(u128::from(CAMERA_RECORDING_MAX_DURATION_MS)) as u64)
    });
    Ok(value)
}

fn default_resource_ura(kind: MediaRecordingKind) -> anyhow::Result<String> {
    let resource_type = kind.resource_type();
    let response = LocalRuntimeStateReadIssuer::invoke(
        "meta.list_resources",
        json!({"types": [resource_type]}),
    )
    .with_context(|| format!("invoke meta.list_resources(types=[\"{resource_type}\"])"))?;
    select_default_resource_ura(kind, &response)
}

fn select_default_resource_ura(
    kind: MediaRecordingKind,
    response: &Value,
) -> anyhow::Result<String> {
    let resource_type = kind.resource_type();
    let resources = response
        .get("resources")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("meta.list_resources response missing resources array"))?;
    let mut selected = None;
    for (idx, entry) in resources.iter().enumerate() {
        let resource_ura = resource_row_ura(idx, entry, resource_type)?;
        if selected.is_none() {
            selected = Some(resource_ura.to_string());
        }
    }
    if let Some(resource_ura) = selected {
        return Ok(resource_ura);
    }
    anyhow::bail!(
        "no {resource_type} resource is registered on this daemon; restart the daemon so \
         media resource bootstrap can scan devices, or pass --subject with a resource_ura"
    )
}

fn resource_row_ura<'a>(
    idx: usize,
    entry: &'a Value,
    expected_type: &str,
) -> anyhow::Result<&'a str> {
    let object = entry
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("meta.list_resources resources[{idx}] must be an object"))?;
    let resource_type = object
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("meta.list_resources resources[{idx}] missing non-empty type")
        })?;
    if resource_type != expected_type {
        anyhow::bail!(
            "meta.list_resources resources[{idx}] type {resource_type:?} did not match requested {expected_type:?}"
        );
    }
    let resource_ura = object
        .get("resource_ura")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("meta.list_resources resources[{idx}] missing non-empty resource_ura")
        })?;
    let parsed = crate::core::ura::parse_ura(resource_ura).map_err(|error| {
        anyhow::anyhow!(
            "meta.list_resources resources[{idx}] resource_ura {resource_ura:?} is not canonical: {error}"
        )
    })?;
    if parsed.kind != crate::core::ura::URAKind::Resource {
        anyhow::bail!(
            "meta.list_resources resources[{idx}] resource_ura {resource_ura:?} is not a Resource URA"
        );
    }
    Ok(resource_ura)
}

#[derive(Debug)]
struct RecordingSink {
    kind: MediaRecordingKind,
    directory: PathBuf,
    frames_file: File,
    started_at: DateTime<Utc>,
    stream_frame_count: usize,
    artifact_count: usize,
    byte_count: usize,
    artifacts: Vec<RecordingArtifact>,
}

impl RecordingSink {
    fn create(plan: &RecordingPlan) -> anyhow::Result<Self> {
        if plan.kind != MediaRecordingKind::Mic {
            anyhow::bail!("stream artifact sink is only valid for mic.subscribe recording");
        }
        let started_at = Utc::now();
        let directory = recording_directory(plan, started_at)?;
        fs::create_dir_all(&directory)
            .with_context(|| format!("create recording directory {}", directory.display()))?;
        let frames_path = directory.join("frames.jsonl");
        let frames_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&frames_path)
            .with_context(|| format!("create {}", frames_path.display()))?;

        Ok(Self {
            kind: plan.kind,
            directory,
            frames_file,
            started_at,
            stream_frame_count: 0,
            artifact_count: 0,
            byte_count: 0,
            artifacts: Vec::new(),
        })
    }

    fn write_frames(&mut self, frames: &[LocalStreamFrame]) -> anyhow::Result<RecordingSummary> {
        for frame in frames {
            self.write_transport_frame(frame)?;
            if !frame.terminal {
                self.stream_frame_count += 1;
                self.write_artifact_frame(frame)?;
            }
        }
        self.frames_file.flush().context("flush frames.jsonl")?;
        let completed_at = Utc::now();
        let manifest_path = self.write_manifest(completed_at)?;

        Ok(RecordingSummary {
            directory: self.directory.clone(),
            manifest_path,
            artifact_count: self.artifact_count,
            byte_count: self.byte_count,
        })
    }

    fn write_transport_frame(&mut self, frame: &LocalStreamFrame) -> anyhow::Result<()> {
        let value = json!({
            "sequence": frame.sequence,
            "content_type": frame.content_type,
            "terminal": frame.terminal,
            "payload": frame.payload,
        });
        writeln!(self.frames_file, "{}", serde_json::to_string(&value)?)
            .context("append frames.jsonl")
    }

    fn write_artifact_frame(&mut self, frame: &LocalStreamFrame) -> anyhow::Result<()> {
        let Some(encoded) = frame
            .payload
            .get(self.kind.payload_key())
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(());
        };
        let bytes = BASE64_STANDARD.decode(encoded).with_context(|| {
            format!(
                "decode {} from stream frame {}",
                self.kind.payload_key(),
                frame.sequence
            )
        })?;
        let file_name = format!(
            "frame-{sequence:06}.{extension}",
            sequence = frame.sequence,
            extension = self.kind.file_extension()
        );
        let path = self.directory.join(&file_name);
        atomic_write_with_permissions(&path, &bytes, WritePermissions::OwnerReadWrite)
            .with_context(|| format!("write recording artifact {}", path.display()))?;
        let content_type = frame_content_type(frame, self.kind)?;

        self.byte_count += bytes.len();
        self.artifact_count += 1;
        self.artifacts.push(RecordingArtifact {
            sequence: frame.sequence,
            file: file_name,
            byte_size: bytes.len(),
            content_type,
        });
        Ok(())
    }

    fn write_manifest(&self, completed_at: DateTime<Utc>) -> anyhow::Result<PathBuf> {
        let path = self.directory.join("manifest.json");
        let manifest = RecordingManifest {
            started_at: self.started_at,
            completed_at,
            kind: self.kind.directory_name(),
            stream_frame_count: self.stream_frame_count,
            artifact_count: self.artifact_count,
            byte_count: self.byte_count,
            frames_jsonl: "frames.jsonl",
            artifacts: &self.artifacts,
        };
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        atomic_write_with_permissions(&path, &bytes, WritePermissions::OwnerReadWrite)
            .with_context(|| format!("write recording manifest {}", path.display()))?;
        Ok(path)
    }
}

#[derive(Debug)]
struct RecordingSummary {
    directory: PathBuf,
    manifest_path: PathBuf,
    artifact_count: usize,
    byte_count: usize,
}

#[derive(Debug, Serialize)]
struct RecordingManifest<'a> {
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
    kind: &'static str,
    stream_frame_count: usize,
    artifact_count: usize,
    byte_count: usize,
    frames_jsonl: &'static str,
    artifacts: &'a [RecordingArtifact],
}

#[derive(Debug, Clone, Serialize)]
struct RecordingArtifact {
    sequence: u64,
    file: String,
    byte_size: usize,
    content_type: String,
}

fn frame_content_type(
    frame: &LocalStreamFrame,
    kind: MediaRecordingKind,
) -> anyhow::Result<String> {
    let content_type = frame
        .payload
        .get("content_type")
        .and_then(Value::as_str)
        .or_else(|| (!frame.content_type.trim().is_empty()).then_some(frame.content_type.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "recording artifact frame {} for {} omitted content_type",
                frame.sequence,
                kind.directory_name()
            )
        })?;
    Ok(content_type.to_string())
}

fn recording_directory(plan: &RecordingPlan, started_at: DateTime<Utc>) -> anyhow::Result<PathBuf> {
    let root = plan
        .output_dir
        .clone()
        .unwrap_or_else(|| state_dir().join("recordings"));
    let run_id = format!(
        "{}-{}",
        started_at.format("%Y%m%dT%H%M%SZ"),
        Uuid::new_v4().simple()
    );
    Ok(root
        .join(plan.kind.directory_name())
        .join(safe_path_component(&run_id)))
}

fn safe_path_component(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mic_plan(output_dir: PathBuf) -> RecordingPlan {
        let selector =
            AbilitySelector::parse("easynet:///r/acme/ability/device.dev.mic.subscribe").unwrap();
        let target = LocalAbilityTarget::from_selector(&selector);
        RecordingPlan {
            selector,
            target,
            kind: MediaRecordingKind::Mic,
            arguments: json!({}),
            subject: "easynet:///r/acme/resource/device.dev/streams/mic.1".to_string(),
            timeout: Duration::from_secs(1),
            duration: Duration::from_millis(DEFAULT_CAMERA_RECORD_DURATION_MS),
            max_frames: 1,
            output_dir: Some(output_dir),
            print_frames: false,
        }
    }

    #[test]
    fn record_rejects_zero_frame_budget_before_ipc() {
        let err = RecordingPlan::from_args(RecordArgs {
            ability_ura: "easynet:///r/acme/ability/device.dev.mic.subscribe".to_string(),
            args: None,
            subject: Some("easynet:///r/acme/resource/device.dev/streams/mic.1".to_string()),
            max_frames: 0,
            duration_ms: DEFAULT_CAMERA_RECORD_DURATION_MS,
            timeout: 60,
            output_dir: None,
            print_frames: false,
        })
        .expect_err("zero max frame count must fail before daemon IPC");
        assert!(format!("{err}").contains("--max-frames"));
    }

    #[test]
    fn recording_kind_accepts_camera_subscribe_and_start() {
        assert_eq!(
            MediaRecordingKind::from_public_name("camera.subscribe").unwrap(),
            MediaRecordingKind::Camera
        );
        assert_eq!(
            MediaRecordingKind::from_public_name("camera.record_start").unwrap(),
            MediaRecordingKind::Camera
        );
    }

    #[test]
    fn recording_kind_rejects_non_stream_camera_snapshot() {
        let err = MediaRecordingKind::from_public_name("camera.snapshot").unwrap_err();
        assert!(format!("{err}").contains("camera.record_start"));
    }

    #[test]
    fn camera_recording_uses_same_owner_transition_abilities() {
        let selector =
            AbilitySelector::parse("easynet:///r/acme/ability/device.dev.camera.subscribe")
                .unwrap();
        let start = sibling_ability_selector(&selector, "camera.record_start").unwrap();
        let stop = sibling_ability_selector(&selector, "camera.record_stop").unwrap();

        assert_eq!(
            start.ability_ura(),
            "easynet:///r/acme/ability/device.dev.camera.record_start"
        );
        assert_eq!(
            stop.ability_ura(),
            "easynet:///r/acme/ability/device.dev.camera.record_stop"
        );
        assert_eq!(start.owner_ura(), selector.owner_ura());
        assert_eq!(stop.owner_ura(), selector.owner_ura());
    }

    #[test]
    fn camera_start_arguments_add_recording_defaults() {
        let selector =
            AbilitySelector::parse("easynet:///r/acme/ability/device.dev.camera.subscribe")
                .unwrap();
        let target = LocalAbilityTarget::from_selector(&selector);
        let plan = RecordingPlan {
            selector,
            target,
            kind: MediaRecordingKind::Camera,
            arguments: json!({"fps": 5}),
            subject: "easynet:///r/acme/resource/device.dev/streams/camera.1".to_string(),
            timeout: Duration::from_secs(1),
            duration: Duration::from_millis(2_000),
            max_frames: DEFAULT_MAX_FRAMES,
            output_dir: None,
            print_frames: false,
        };
        let args = camera_start_arguments(&plan).unwrap();

        assert_eq!(args["codec"], "mjpeg");
        assert_eq!(args["fps"], 5);
        assert_eq!(args["max_duration_ms"], 3_000);
    }

    #[test]
    fn default_resource_selection_returns_schema_bound_resource_ura() {
        let selected = select_default_resource_ura(
            MediaRecordingKind::Mic,
            &json!({
                "resources": [{
                    "type": "mic",
                    "resource_ura": "easynet:///r/acme/resource/device.dev/streams/mic.1",
                    "display_name": "Built-in Mic"
                }]
            }),
        )
        .expect("valid resource row");

        assert_eq!(
            selected,
            "easynet:///r/acme/resource/device.dev/streams/mic.1"
        );
    }

    #[test]
    fn default_resource_selection_rejects_matching_row_without_resource_ura() {
        let err = select_default_resource_ura(
            MediaRecordingKind::Mic,
            &json!({
                "resources": [{
                    "type": "mic",
                    "display_name": "Built-in Mic"
                }]
            }),
        )
        .expect_err("malformed resource row must fail closed");

        assert!(
            err.to_string()
                .contains("resources[0] missing non-empty resource_ura"),
            "unexpected error: {err}"
        );
        assert!(
            !err.to_string().contains("no mic resource"),
            "malformed read-model row must not be projected as empty inventory: {err}"
        );
    }

    #[test]
    fn default_resource_selection_rejects_non_resource_ura() {
        let err = select_default_resource_ura(
            MediaRecordingKind::Camera,
            &json!({
                "resources": [{
                    "type": "camera",
                    "resource_ura": "easynet:///r/acme/device/dev"
                }]
            }),
        )
        .expect_err("resource row must carry Resource URA");

        assert!(
            err.to_string().contains("is not a Resource URA"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn default_resource_selection_validates_all_returned_rows_before_selecting() {
        let err = select_default_resource_ura(
            MediaRecordingKind::Mic,
            &json!({
                "resources": [
                    {
                        "type": "mic",
                        "resource_ura": "easynet:///r/acme/resource/device.dev/streams/mic.1"
                    },
                    {
                        "type": "mic",
                        "resource_ura": ""
                    }
                ]
            }),
        )
        .expect_err("later malformed rows must not be hidden by first valid row");

        assert!(
            err.to_string()
                .contains("resources[1] missing non-empty resource_ura"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn recording_sink_persists_mic_artifact_and_manifest() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let output_dir = state_dir().join("test-recordings");
        let plan = mic_plan(output_dir);
        let bytes = vec![0, 1, 2, 3];
        let frames = vec![LocalStreamFrame {
            sequence: 3,
            content_type: "application/json".to_string(),
            terminal: false,
            payload: json!({
                "content_type": "audio/L16; rate=48000; channels=1",
                "samples_b64": BASE64_STANDARD.encode(&bytes),
            }),
        }];

        let mut sink = RecordingSink::create(&plan).unwrap();
        let summary = sink.write_frames(&frames).unwrap();

        let artifact = summary.directory.join("frame-000003.pcm");
        assert_eq!(fs::read(artifact).unwrap(), bytes);
        let manifest: Value =
            serde_json::from_slice(&fs::read(summary.manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["kind"], "mic");
        assert_eq!(manifest["artifact_count"], 1);
        assert_eq!(manifest["artifacts"][0]["file"], "frame-000003.pcm");
    }

    #[test]
    fn recording_sink_rejects_artifact_without_content_type() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let output_dir = state_dir().join("test-recordings");
        let plan = mic_plan(output_dir);
        let frames = vec![LocalStreamFrame {
            sequence: 3,
            content_type: String::new(),
            terminal: false,
            payload: json!({
                "samples_b64": BASE64_STANDARD.encode([0, 1, 2, 3]),
            }),
        }];

        let mut sink = RecordingSink::create(&plan).unwrap();
        let error = sink
            .write_frames(&frames)
            .expect_err("artifact content_type is mandatory");
        assert!(
            error.to_string().contains("omitted content_type"),
            "unexpected error: {error}"
        );
    }
}

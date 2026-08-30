// EasyNet RemoteApp native-host protocol
// =======================================
//
// This crate owns only the private, versioned process-boundary contract shared
// by the RemoteApp plugin client and its native host. It has no Runtime,
// authority, session, resource, transport, or platform API dependency.

use std::collections::BTreeSet;
use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub mod capture_probe;
#[cfg(target_os = "macos")]
pub mod macos_launch_services;
pub mod media_capabilities;
pub mod media_session;
pub mod screen_capture_permission;
#[cfg(any(unix, windows))]
pub mod shared_media_lane;

pub const SCHEMA_VERSION: u32 = 1;
pub const PROTOCOL: &str = "remoteapp_native_host_v1";
pub const REQUEST_KIND: &str = "sample_target_inventory";
pub const RESPONSE_KIND: &str = "target_inventory_sample";
pub const PARENT_LIVENESS_FD_ENV: &str = "EASYNET_REMOTEAPP_PARENT_LIVENESS_FD";
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

const MAX_WINDOWS: usize = 2_048;
const MAX_DISPLAYS: usize = 64;
const MAX_STRING_BYTES: usize = 1_024;
const MAX_ABS_COORDINATE: f64 = 10_000_000.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: u32,
    pub protocol: String,
    pub kind: String,
    pub process_generation: u64,
    pub request_id: u64,
}

impl Request {
    pub fn sample_target_inventory(process_generation: u64, request_id: u64) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            kind: REQUEST_KIND.to_string(),
            process_generation,
            request_id,
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION
            || self.protocol != PROTOCOL
            || self.kind != REQUEST_KIND
            || self.process_generation == 0
            || self.request_id == 0
        {
            return Err(ValidationError::new(
                "unsupported native-host request envelope",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub schema_version: u32,
    pub protocol: String,
    pub kind: String,
    pub process_generation: u64,
    pub request_id: u64,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub observation: TargetObservationSample,
}

impl Response {
    pub fn target_inventory(
        request: &Request,
        started_at_ms: u64,
        completed_at_ms: u64,
        observation: TargetObservationSample,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            kind: RESPONSE_KIND.to_string(),
            process_generation: request.process_generation,
            request_id: request.request_id,
            started_at_ms,
            completed_at_ms,
            observation,
        }
    }

    pub fn matches_request(&self, process_generation: u64, request_id: u64) -> bool {
        self.schema_version == SCHEMA_VERSION
            && self.protocol == PROTOCOL
            && self.kind == RESPONSE_KIND
            && self.process_generation == process_generation
            && self.request_id == request_id
            && self.started_at_ms > 0
            && self.completed_at_ms >= self.started_at_ms
            && self.observation.validate().is_ok()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetObservationSample {
    pub state: TargetObservationSampleState,
}

impl TargetObservationSample {
    pub fn host_snapshot(windows: Vec<ObservedWindow>, display_ids: BTreeSet<u64>) -> Self {
        Self {
            state: TargetObservationSampleState::HostSnapshot(HostTargetSnapshot {
                windows,
                display_ids,
            }),
        }
    }

    pub fn snapshot_failed(detail: impl Into<String>, observed_at_ms: u64) -> Self {
        Self {
            state: TargetObservationSampleState::SnapshotFailed {
                detail: detail.into(),
                observed_at_ms,
            },
        }
    }

    pub fn permission_revoked(detail: impl Into<String>, observed_at_ms: u64) -> Self {
        Self {
            state: TargetObservationSampleState::PermissionRevoked {
                detail: detail.into(),
                observed_at_ms,
            },
        }
    }

    pub fn unsupported_platform() -> Self {
        Self {
            state: TargetObservationSampleState::UnsupportedPlatform,
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        match &self.state {
            TargetObservationSampleState::HostSnapshot(snapshot) => {
                if snapshot.windows.len() > MAX_WINDOWS {
                    return Err(ValidationError::new(format!(
                        "native host returned more than {MAX_WINDOWS} windows"
                    )));
                }
                if snapshot.display_ids.len() > MAX_DISPLAYS {
                    return Err(ValidationError::new(format!(
                        "native host returned more than {MAX_DISPLAYS} displays"
                    )));
                }
                let mut window_ids = BTreeSet::new();
                for window in &snapshot.windows {
                    if window.window_id == 0 || !window_ids.insert(window.window_id) {
                        return Err(ValidationError::new(
                            "native host returned a zero or duplicate window id",
                        ));
                    }
                    for value in [
                        window.process_instance_id.as_deref(),
                        window.bundle_id.as_deref(),
                        window.title.as_deref(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if value.len() > MAX_STRING_BYTES {
                            return Err(ValidationError::new(
                                "native host returned an oversized string",
                            ));
                        }
                    }
                    for value in [
                        window.geometry.x,
                        window.geometry.y,
                        window.geometry.width,
                        window.geometry.height,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if !value.is_finite() || value.abs() > MAX_ABS_COORDINATE {
                            return Err(ValidationError::new(
                                "native host geometry is outside the finite coordinate bound",
                            ));
                        }
                    }
                    if window.geometry.width.is_some_and(|value| value <= 0.0)
                        || window.geometry.height.is_some_and(|value| value <= 0.0)
                    {
                        return Err(ValidationError::new(
                            "native host window dimensions must be positive",
                        ));
                    }
                }
                Ok(())
            }
            TargetObservationSampleState::SnapshotFailed {
                detail,
                observed_at_ms,
            }
            | TargetObservationSampleState::PermissionRevoked {
                detail,
                observed_at_ms,
            } => {
                if detail.len() > MAX_STRING_BYTES || *observed_at_ms == 0 {
                    return Err(ValidationError::new(
                        "native host diagnostic is oversized or has no observation time",
                    ));
                }
                Ok(())
            }
            TargetObservationSampleState::UnsupportedPlatform => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", content = "payload", rename_all = "snake_case")]
pub enum TargetObservationSampleState {
    HostSnapshot(HostTargetSnapshot),
    SnapshotFailed { detail: String, observed_at_ms: u64 },
    PermissionRevoked { detail: String, observed_at_ms: u64 },
    UnsupportedPlatform,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostTargetSnapshot {
    pub windows: Vec<ObservedWindow>,
    pub display_ids: BTreeSet<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedWindow {
    pub window_id: u64,
    pub pid: Option<i64>,
    pub process_instance_id: Option<String>,
    pub bundle_id: Option<String>,
    pub display_id: Option<u64>,
    pub title: Option<String>,
    pub focused: bool,
    pub geometry: Geometry,
    pub visibility_state: VisibilityState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityState {
    Visible,
    Hidden,
    Minimized,
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    detail: String,
}

impl ValidationError {
    pub(crate) fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for ValidationError {}

#[derive(Debug)]
pub enum FrameError {
    UnexpectedEof,
    Oversized,
    Encode(String),
    Decode(String),
    Io(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnexpectedEof => formatter.write_str("unexpected EOF"),
            Self::Oversized => formatter.write_str("frame exceeds protocol limit"),
            Self::Encode(detail) => write!(formatter, "encode frame: {detail}"),
            Self::Decode(detail) => write!(formatter, "decode frame: {detail}"),
            Self::Io(detail) => write!(formatter, "frame I/O: {detail}"),
        }
    }
}

impl std::error::Error for FrameError {}

pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), FrameError> {
    let bytes = serde_json::to_vec(value).map_err(|error| FrameError::Encode(error.to_string()))?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES || bytes.len() > u32::MAX as usize {
        return Err(FrameError::Oversized);
    }
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .and_then(|()| writer.write_all(&bytes))
        .and_then(|()| writer.flush())
        .map_err(|error| FrameError::Io(error.to_string()))
}

pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<Option<T>, FrameError> {
    let mut first = [0_u8; 1];
    loop {
        match reader.read(&mut first) {
            Ok(0) => return Ok(None),
            Ok(1) => break,
            Ok(_) => unreachable!("one-byte read returned more than one byte"),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(FrameError::Io(error.to_string())),
        }
    }
    let mut header = [0_u8; 4];
    header[0] = first[0];
    reader
        .read_exact(&mut header[1..])
        .map_err(|error| FrameError::Io(error.to_string()))?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(FrameError::Oversized);
    }
    let mut body = vec![0_u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| FrameError::Io(error.to_string()))?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| FrameError::Decode(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip_and_validation() {
        let request = Request::sample_target_inventory(7, 11);
        let mut frame = Vec::new();
        write_frame(&mut frame, &request).unwrap();
        let decoded: Request = read_frame(&mut frame.as_slice()).unwrap().unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded.process_generation, 7);
        assert_eq!(decoded.request_id, 11);
    }

    #[test]
    fn oversized_length_fails_before_body_allocation() {
        let mut frame = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(b"{}");
        assert!(matches!(
            read_frame::<Request>(&mut frame.as_slice()),
            Err(FrameError::Oversized)
        ));
    }

    #[test]
    fn observation_rejects_duplicate_window_identity_and_non_finite_geometry() {
        let window = ObservedWindow {
            window_id: 9,
            pid: Some(42),
            process_instance_id: None,
            bundle_id: None,
            display_id: Some(1),
            title: None,
            focused: false,
            geometry: Geometry {
                x: Some(0.0),
                y: Some(0.0),
                width: Some(640.0),
                height: Some(480.0),
            },
            visibility_state: VisibilityState::Visible,
        };
        let duplicate = TargetObservationSample::host_snapshot(
            vec![window.clone(), window.clone()],
            BTreeSet::from([1]),
        );
        assert!(duplicate.validate().is_err());

        let non_finite = TargetObservationSample::host_snapshot(
            vec![ObservedWindow {
                geometry: Geometry {
                    width: Some(f64::INFINITY),
                    ..window.geometry.clone()
                },
                ..window
            }],
            BTreeSet::from([1]),
        );
        assert!(non_finite.validate().is_err());
    }

    #[test]
    fn response_is_bound_to_generation_request_and_monotonic_completion() {
        let request = Request::sample_target_inventory(7, 11);
        let response = Response::target_inventory(
            &request,
            100,
            101,
            TargetObservationSample::snapshot_failed("fixture", 100),
        );
        assert!(response.matches_request(7, 11));
        assert!(!response.matches_request(6, 11));
        assert!(!response.matches_request(7, 10));

        let regressed = Response {
            completed_at_ms: 99,
            ..response
        };
        assert!(!regressed.matches_request(7, 11));
    }
}

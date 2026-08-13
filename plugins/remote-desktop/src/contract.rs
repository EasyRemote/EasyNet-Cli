//! Product-owned remote desktop lifecycle and media contract.
//!
//! Axon transports canonical invocations and receipts. Remote desktop wire
//! vocabulary belongs to this product provider because its states, transports,
//! and backend rules have no meaning in the shared runtime model.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RemoteDesktopSessionState {
    Unspecified,
    Pending,
    Negotiating,
    Connected,
    ConnectedPreview,
    Suspended,
    Degraded,
    Closing,
    Closed,
    Failed,
}

impl RemoteDesktopSessionState {
    pub(super) fn wire_name(self) -> &'static str {
        match self {
            Self::Unspecified => "REMOTE_DESKTOP_SESSION_STATE_UNSPECIFIED",
            Self::Pending => "REMOTE_DESKTOP_SESSION_STATE_PENDING",
            Self::Negotiating => "REMOTE_DESKTOP_SESSION_STATE_NEGOTIATING",
            Self::Connected => "REMOTE_DESKTOP_SESSION_STATE_CONNECTED",
            Self::ConnectedPreview => "REMOTE_DESKTOP_SESSION_STATE_CONNECTED_PREVIEW",
            Self::Suspended => "REMOTE_DESKTOP_SESSION_STATE_SUSPENDED",
            Self::Degraded => "REMOTE_DESKTOP_SESSION_STATE_DEGRADED",
            Self::Closing => "REMOTE_DESKTOP_SESSION_STATE_CLOSING",
            Self::Closed => "REMOTE_DESKTOP_SESSION_STATE_CLOSED",
            Self::Failed => "REMOTE_DESKTOP_SESSION_STATE_FAILED",
        }
    }

    pub(super) fn json_name(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Pending => "pending",
            Self::Negotiating => "negotiating",
            Self::Connected => "connected",
            Self::ConnectedPreview => "connected_preview",
            Self::Suspended => "suspended",
            Self::Degraded => "degraded",
            Self::Closing => "closing",
            Self::Closed => "closed",
            Self::Failed => "failed",
        }
    }

    pub(super) fn is_terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RemoteDesktopTransportKind {
    Unspecified,
    #[serde(rename = "webrtc")]
    WebRtc,
    ExternalRtp,
    InvokeBidi,
    PreviewStream,
}

impl RemoteDesktopTransportKind {
    fn json_name(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::WebRtc => "webrtc",
            Self::ExternalRtp => "external_rtp",
            Self::InvokeBidi => "invoke_bidi",
            Self::PreviewStream => "preview_stream",
        }
    }

    fn is_production_media_transport(self) -> bool {
        matches!(self, Self::WebRtc | Self::ExternalRtp)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RemoteDesktopBackendStatus {
    Unspecified,
    Available,
    NotInstalled,
    PermissionDenied,
    Unavailable,
}

impl RemoteDesktopBackendStatus {
    fn json_name(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Available => "available",
            Self::NotInstalled => "not_installed",
            Self::PermissionDenied => "permission_denied",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RemoteDesktopMediaBackendContract {
    pub(super) backend_id: String,
    pub(super) sdk_id: String,
    pub(super) kind: String,
    pub(super) status: RemoteDesktopBackendStatus,
    pub(super) transport: RemoteDesktopTransportKind,
    pub(super) capture_api: String,
    pub(super) encoder: String,
    pub(super) max_capture_fps: u32,
    pub(super) max_encode_fps: u32,
    pub(super) hardware_accelerated: bool,
    pub(super) stale_frame_drop: bool,
    pub(super) external_binary_required: bool,
    pub(super) production_ready: bool,
    pub(super) transport_ready: bool,
    pub(super) supported_subjects: Vec<String>,
    pub(super) unavailable_reason: Option<String>,
}

impl RemoteDesktopMediaBackendContract {
    pub(super) fn validate(&self) -> Result<(), RemoteDesktopContractError> {
        for (field, value) in [
            ("backend_id", self.backend_id.as_str()),
            ("sdk_id", self.sdk_id.as_str()),
            ("capture_api", self.capture_api.as_str()),
            ("encoder", self.encoder.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(RemoteDesktopContractError::BackendFieldRequired(field));
            }
        }
        for (field, value) in [
            ("max_capture_fps", self.max_capture_fps),
            ("max_encode_fps", self.max_encode_fps),
        ] {
            if value == 0 {
                return Err(RemoteDesktopContractError::BackendFpsZero {
                    backend_id: self.backend_id.clone(),
                    field,
                });
            }
        }
        if self.supported_subjects.is_empty() {
            return Err(RemoteDesktopContractError::BackendHasNoSubjects(
                self.backend_id.clone(),
            ));
        }
        if let Some(subject) = self
            .supported_subjects
            .iter()
            .find(|subject| !matches!(subject.as_str(), "display" | "window" | "application"))
        {
            return Err(RemoteDesktopContractError::UnsupportedBackendSubject {
                backend_id: self.backend_id.clone(),
                subject: subject.clone(),
            });
        }
        if self.transport_ready && self.status != RemoteDesktopBackendStatus::Available {
            return Err(RemoteDesktopContractError::TransportBackendUnavailable {
                backend_id: self.backend_id.clone(),
                status: self.status.json_name(),
            });
        }
        if self.production_ready {
            if !self.transport_ready {
                return Err(
                    RemoteDesktopContractError::ProductionBackendTransportNotReady {
                        backend_id: self.backend_id.clone(),
                    },
                );
            }
            if self.status != RemoteDesktopBackendStatus::Available {
                return Err(RemoteDesktopContractError::ProductionBackendUnavailable {
                    backend_id: self.backend_id.clone(),
                    status: self.status.json_name(),
                });
            }
            if !self.transport.is_production_media_transport() {
                return Err(
                    RemoteDesktopContractError::ProductionBackendUsesDiagnosticTransport {
                        backend_id: self.backend_id.clone(),
                        transport: self.transport.json_name(),
                    },
                );
            }
            if self.external_binary_required {
                return Err(
                    RemoteDesktopContractError::ProductionBackendExternalBinary {
                        backend_id: self.backend_id.clone(),
                    },
                );
            }
        }
        Ok(())
    }

    pub(super) fn to_json(&self) -> Value {
        json!({
            "backend_id": self.backend_id,
            "sdk_id": self.sdk_id,
            "kind": self.kind,
            "status": self.status.json_name(),
            "transport": self.transport.json_name(),
            "capture_api": self.capture_api,
            "encoder": self.encoder,
            "max_capture_fps": self.max_capture_fps,
            "max_encode_fps": self.max_encode_fps,
            "hardware_accelerated": self.hardware_accelerated,
            "stale_frame_drop": self.stale_frame_drop,
            "external_binary_required": self.external_binary_required,
            "production_ready": self.production_ready,
            "transport_ready": self.transport_ready,
            "supported_subjects": self.supported_subjects,
            "unavailable_reason": self.unavailable_reason,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(super) enum RemoteDesktopContractError {
    #[error("remote desktop media backend field {0} is required")]
    BackendFieldRequired(&'static str),
    #[error("remote desktop media backend {backend_id} field {field} must be positive")]
    BackendFpsZero {
        backend_id: String,
        field: &'static str,
    },
    #[error("remote desktop media backend {0} supports no capture subjects")]
    BackendHasNoSubjects(String),
    #[error("remote desktop media backend {backend_id} has unsupported subject {subject}")]
    UnsupportedBackendSubject { backend_id: String, subject: String },
    #[error(
        "remote desktop media backend {backend_id} marks transport_ready while status={status}"
    )]
    TransportBackendUnavailable {
        backend_id: String,
        status: &'static str,
    },
    #[error("production remote desktop backend {backend_id} is not transport-ready")]
    ProductionBackendTransportNotReady { backend_id: String },
    #[error("production remote desktop backend {backend_id} is not available; status={status}")]
    ProductionBackendUnavailable {
        backend_id: String,
        status: &'static str,
    },
    #[error("production remote desktop backend {backend_id} uses diagnostic transport {transport}")]
    ProductionBackendUsesDiagnosticTransport {
        backend_id: String,
        transport: &'static str,
    },
    #[error("production remote desktop backend {backend_id} requires an external binary")]
    ProductionBackendExternalBinary { backend_id: String },
}

#[cfg(test)]
mod tests {
    use super::{
        RemoteDesktopMediaBackendContract, RemoteDesktopSessionState, RemoteDesktopTransportKind,
    };
    use serde_json::json;

    #[test]
    fn public_state_projection_remains_stable() {
        assert_eq!(
            RemoteDesktopSessionState::Connected.json_name(),
            "connected"
        );
        assert_eq!(
            RemoteDesktopSessionState::Connected.wire_name(),
            "REMOTE_DESKTOP_SESSION_STATE_CONNECTED"
        );
        assert!(RemoteDesktopSessionState::Closed.is_terminal());
        assert!(RemoteDesktopSessionState::Failed.is_terminal());
    }

    #[test]
    fn media_backend_contract_accepts_only_canonical_webrtc_transport_name() {
        let decoded: RemoteDesktopMediaBackendContract = serde_json::from_value(json!({
            "backend_id": "builtin.xcap.openh264.webrtc.v1",
            "sdk_id": "easynet.remote_desktop.media.v1",
            "kind": "screen_capture",
            "status": "available",
            "transport": "webrtc",
            "capture_api": "xcap",
            "encoder": "openh264",
            "max_capture_fps": 30,
            "max_encode_fps": 30,
            "hardware_accelerated": false,
            "stale_frame_drop": true,
            "external_binary_required": false,
            "production_ready": true,
            "transport_ready": true,
            "supported_subjects": ["display"],
            "unavailable_reason": null
        }))
        .expect("canonical webrtc transport must decode");

        assert_eq!(decoded.transport, RemoteDesktopTransportKind::WebRtc);
    }

    #[test]
    fn media_backend_contract_rejects_retired_web_rtc_transport_alias() {
        let error = serde_json::from_value::<RemoteDesktopMediaBackendContract>(json!({
            "backend_id": "builtin.xcap.openh264.webrtc.v1",
            "sdk_id": "easynet.remote_desktop.media.v1",
            "kind": "screen_capture",
            "status": "available",
            "transport": "web_rtc",
            "capture_api": "xcap",
            "encoder": "openh264",
            "max_capture_fps": 30,
            "max_encode_fps": 30,
            "hardware_accelerated": false,
            "stale_frame_drop": true,
            "external_binary_required": false,
            "production_ready": true,
            "transport_ready": true,
            "supported_subjects": ["display"],
            "unavailable_reason": null
        }))
        .expect_err("retired web_rtc transport alias must fail closed");

        assert!(
            error.to_string().contains("web_rtc"),
            "error must name retired input: {error}"
        );
    }
}

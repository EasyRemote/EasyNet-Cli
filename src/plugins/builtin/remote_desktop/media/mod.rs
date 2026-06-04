// EasyNet CLI — remote desktop media SDK contract
// =================================================
//
// File: src/plugins/builtin/remote_desktop/media/mod.rs
// Description: Device-side SDK/plugin catalogue for remote desktop media.
//
// Protocol Responsibility:
// - Defines the media backend contract exposed by
//   device.remote_desktop.* ability receipts.
// - Does not own Axon session/signaling semantics; those remain in
//   the ability and Axon invocation layers.
//
// Implementation Approach:
// - Keep backend capability data immutable and serializable.
// - Select only the backend that the local device runtime can
//   actually serve; platform native plugins are represented here before
//   ability handlers route media to them.
//
// Usage Contract:
// - Callers may advertise unavailable plugin descriptors, but must
//   only stream through descriptors whose status is `available`.
// - Requested fps and effective fps must remain separate values.
//
// Architectural Position:
// - EasyNet-Cli device adapter SDK layer. Axon carries signed
//   invocations; this module describes local capture/encode plugins.

use easynet_axon::{
    RemoteDesktopBackendStatus, RemoteDesktopMediaBackendContract, RemoteDesktopTransportKind,
};
use serde_json::{json, Value};

use crate::persistence::resources::{ResourceEntry, ResourceType};

pub(in crate::plugins::builtin::remote_desktop) mod encode;
pub(in crate::plugins::builtin::remote_desktop) mod native;

pub const REMOTE_DESKTOP_MEDIA_SDK_ID: &str = "easynet.remote_desktop.media.v1";
pub const XCAP_OPENH264_BACKEND_ID: &str = "builtin.xcap.openh264.annexb.v1";
pub const XCAP_OPENH264_WEBRTC_BACKEND_ID: &str = "builtin.xcap.openh264.webrtc.v1";
pub const MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID: &str =
    "plugin.macos.screencapturekit.videotoolbox.webrtc.v1";
pub const XCAP_MACOS_RECORDER_MAX_FPS: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteDesktopMediaBackendDescriptor {
    backend_id: &'static str,
    sdk_id: &'static str,
    kind: &'static str,
    status: &'static str,
    capture_api: &'static str,
    encoder: &'static str,
    carrier: &'static str,
    max_capture_fps: u32,
    max_encode_fps: u32,
    hardware_accelerated: bool,
    stale_frame_drop: bool,
    external_binary_required: bool,
    transport_ready: bool,
    production_ready: bool,
    supported_subjects: &'static [&'static str],
    unavailable_reason: Option<&'static str>,
}

impl RemoteDesktopMediaBackendDescriptor {
    pub fn backend_id(self) -> &'static str {
        self.backend_id
    }

    pub fn sdk_id(self) -> &'static str {
        self.sdk_id
    }

    pub fn capture_api(self) -> &'static str {
        self.capture_api
    }

    pub fn encoder(self) -> &'static str {
        self.encoder
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn carrier(self) -> &'static str {
        self.carrier
    }

    pub fn max_capture_fps(self) -> u32 {
        self.max_capture_fps
    }

    pub fn max_encode_fps(self) -> u32 {
        self.max_encode_fps
    }

    pub fn external_binary_required(self) -> bool {
        self.external_binary_required
    }

    pub fn production_ready(self) -> bool {
        self.production_ready
    }

    pub fn transport_ready(self) -> bool {
        self.transport_ready
    }

    pub fn is_available(self) -> bool {
        self.status == "available"
    }

    pub fn is_webrtc_transport(self) -> bool {
        self.axon_transport() == RemoteDesktopTransportKind::WebRtc
    }

    pub fn unavailable_reason(self) -> Option<&'static str> {
        self.unavailable_reason
    }

    pub fn supports_entry(self, entry: &ResourceEntry) -> bool {
        let subject = match entry.kind {
            ResourceType::Display => "display",
            ResourceType::Window => "window",
            ResourceType::Application => "application",
            _ => return false,
        };
        self.supported_subjects.contains(&subject)
    }

    pub fn effective_fps(self, requested_fps: u32) -> u32 {
        requested_fps
            .min(self.max_capture_fps)
            .min(self.max_encode_fps)
            .max(1)
    }

    pub fn axon_contract(self) -> RemoteDesktopMediaBackendContract {
        RemoteDesktopMediaBackendContract {
            backend_id: self.backend_id.to_string(),
            sdk_id: self.sdk_id().to_string(),
            kind: self.kind.to_string(),
            status: self.axon_status(),
            transport: self.axon_transport(),
            capture_api: self.capture_api.to_string(),
            encoder: self.encoder.to_string(),
            max_capture_fps: self.max_capture_fps,
            max_encode_fps: self.max_encode_fps,
            hardware_accelerated: self.hardware_accelerated,
            stale_frame_drop: self.stale_frame_drop,
            external_binary_required: self.external_binary_required(),
            production_ready: self.production_ready,
            transport_ready: self.transport_ready,
            supported_subjects: self
                .supported_subjects
                .iter()
                .map(|s| s.to_string())
                .collect(),
            unavailable_reason: self.unavailable_reason.map(str::to_string),
        }
    }

    pub fn validate_axon_contract(self) -> Result<(), easynet_axon::RemoteDesktopContractError> {
        self.axon_contract().validate()
    }

    pub fn to_json(self) -> Value {
        debug_assert!(self.validate_axon_contract().is_ok());
        let mut value = self.axon_contract().to_json();
        if let Value::Object(map) = &mut value {
            map.insert("carrier".to_string(), json!(self.carrier));
        }
        value
    }

    fn axon_status(self) -> RemoteDesktopBackendStatus {
        match self.status {
            "available" => RemoteDesktopBackendStatus::Available,
            "not_installed" => RemoteDesktopBackendStatus::NotInstalled,
            "permission_denied" => RemoteDesktopBackendStatus::PermissionDenied,
            "unavailable" => RemoteDesktopBackendStatus::Unavailable,
            _ => RemoteDesktopBackendStatus::Unspecified,
        }
    }

    fn axon_transport(self) -> RemoteDesktopTransportKind {
        match self.carrier {
            "webrtc.rtp_srtp" => RemoteDesktopTransportKind::WebRtc,
            "axon.invoke_bidi.annexb_h264" => RemoteDesktopTransportKind::InvokeBidi,
            "axon.preview_stream" => RemoteDesktopTransportKind::PreviewStream,
            _ => RemoteDesktopTransportKind::Unspecified,
        }
    }
}

pub const XCAP_OPENH264_BACKEND: RemoteDesktopMediaBackendDescriptor =
    RemoteDesktopMediaBackendDescriptor {
        backend_id: XCAP_OPENH264_BACKEND_ID,
        sdk_id: REMOTE_DESKTOP_MEDIA_SDK_ID,
        kind: "builtin",
        status: "available",
        capture_api: "xcap.avcapture_screen_input",
        encoder: "openh264.software",
        carrier: "axon.invoke_bidi.annexb_h264",
        max_capture_fps: XCAP_MACOS_RECORDER_MAX_FPS,
        max_encode_fps: XCAP_MACOS_RECORDER_MAX_FPS,
        hardware_accelerated: false,
        stale_frame_drop: true,
        external_binary_required: false,
        transport_ready: true,
        production_ready: false,
        supported_subjects: &["display"],
        unavailable_reason: None,
    };

pub const XCAP_OPENH264_WEBRTC_BACKEND: RemoteDesktopMediaBackendDescriptor =
    RemoteDesktopMediaBackendDescriptor {
        backend_id: XCAP_OPENH264_WEBRTC_BACKEND_ID,
        sdk_id: REMOTE_DESKTOP_MEDIA_SDK_ID,
        kind: "builtin",
        status: "available",
        capture_api: "xcap.avcapture_screen_input",
        encoder: "openh264.software",
        carrier: "webrtc.rtp_srtp",
        max_capture_fps: XCAP_MACOS_RECORDER_MAX_FPS,
        max_encode_fps: XCAP_MACOS_RECORDER_MAX_FPS,
        hardware_accelerated: false,
        stale_frame_drop: true,
        external_binary_required: false,
        transport_ready: true,
        production_ready: false,
        supported_subjects: &["display"],
        unavailable_reason: Some("native_media_plugin_required_for_flagship_quality"),
    };

// The native ScreenCaptureKit + VideoToolbox plugin is compiled in only on
// macOS (see plugins::remote_desktop::{screencapturekit_capture, videotoolbox_encoder}).
// On other targets the descriptor stays not_installed so the gate keeps
// blocking and the diagnostic relay remains the only path.
#[cfg(target_os = "macos")]
pub const MACOS_SCK_VIDEOTOOLBOX_BACKEND: RemoteDesktopMediaBackendDescriptor =
    RemoteDesktopMediaBackendDescriptor {
        backend_id: MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID,
        sdk_id: REMOTE_DESKTOP_MEDIA_SDK_ID,
        kind: "plugin",
        status: "available",
        capture_api: "macos.screencapturekit",
        encoder: "videotoolbox.h264",
        carrier: "webrtc.rtp_srtp",
        // ScreenCaptureKit + VideoToolbox sustain 144 Hz on Apple silicon;
        // the effective rate is still clamped to the client's request.
        max_capture_fps: 144,
        max_encode_fps: 144,
        hardware_accelerated: true,
        stale_frame_drop: true,
        external_binary_required: false,
        transport_ready: true,
        production_ready: true,
        supported_subjects: &["display", "window", "application"],
        unavailable_reason: None,
    };

#[cfg(not(target_os = "macos"))]
pub const MACOS_SCK_VIDEOTOOLBOX_BACKEND: RemoteDesktopMediaBackendDescriptor =
    RemoteDesktopMediaBackendDescriptor {
        backend_id: MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID,
        sdk_id: REMOTE_DESKTOP_MEDIA_SDK_ID,
        kind: "plugin",
        status: "not_installed",
        capture_api: "macos.screencapturekit",
        encoder: "videotoolbox.h264",
        carrier: "webrtc.rtp_srtp",
        max_capture_fps: 144,
        max_encode_fps: 144,
        hardware_accelerated: true,
        stale_frame_drop: true,
        external_binary_required: false,
        transport_ready: false,
        production_ready: false,
        supported_subjects: &["display", "window", "application"],
        unavailable_reason: Some("native_plugin_requires_macos"),
    };

pub fn sdk_contract_view() -> Value {
    json!({
        "sdk_id": REMOTE_DESKTOP_MEDIA_SDK_ID,
        "owned_by": "EasyNet-Cli device runtime",
        "selected_by": "device.remote_desktop.attach",
        "control_plane": "Axon signed invocation",
        "stream_plane": "Axon InvokeBidi for diagnostic relay; WebRTC RTP/SRTP for production plugins",
        "extension_points": ["capture", "encoder", "carrier", "input"],
        "external_binary_required": false,
    })
}

pub fn backend_catalog_view() -> Value {
    json!([
        XCAP_OPENH264_BACKEND.to_json(),
        XCAP_OPENH264_WEBRTC_BACKEND.to_json(),
        native_webrtc_backend_runtime_descriptor().to_json(),
    ])
}

pub fn production_gate_view() -> Value {
    let native = native_webrtc_backend_runtime_descriptor();
    let ready = native.production_ready();
    if ready {
        json!({
            "ready": true,
            "required_backend": MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID,
            "available_backend": MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID,
            "reason": Value::Null,
            "message": "Native ScreenCaptureKit + VideoToolbox H.264 plugin is wired over WebRTC RTP/SRTP. Screen recording permission is requested on first capture.",
        })
    } else {
        json!({
            "ready": false,
            "required_backend": MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID,
            "available_backend": Value::Null,
            "reason": native.unavailable_reason().unwrap_or("native_media_plugin_required"),
            "message": "Production remote desktop requires a native capture/encode/WebRTC plugin; diagnostic InvokeBidi H.264 relay remains available.",
        })
    }
}

pub fn production_backend_for_entry(
    entry: &ResourceEntry,
) -> Option<RemoteDesktopMediaBackendDescriptor> {
    [native_webrtc_backend_runtime_descriptor()]
        .into_iter()
        .find(|backend| {
            backend.is_available()
                && backend.is_webrtc_transport()
                && backend.transport_ready()
                && backend.production_ready()
                && backend.supports_entry(entry)
        })
}

pub fn webrtc_transport_backend_for_entry(
    entry: &ResourceEntry,
) -> Option<RemoteDesktopMediaBackendDescriptor> {
    if let Some(native) = production_backend_for_entry(entry) {
        return Some(native);
    }
    if entry.kind == ResourceType::Display
        && entry.metadata.get("backend").and_then(Value::as_str) == Some("xcap")
    {
        return Some(XCAP_OPENH264_WEBRTC_BACKEND);
    }
    None
}

fn native_webrtc_backend_runtime_descriptor() -> RemoteDesktopMediaBackendDescriptor {
    let mut backend = MACOS_SCK_VIDEOTOOLBOX_BACKEND;
    if cfg!(target_os = "macos") && !platform_screen_capture_permission_granted() {
        backend.status = "permission_denied";
        backend.transport_ready = false;
        backend.production_ready = false;
        backend.unavailable_reason = Some("screen_capture_permission_denied");
    }
    backend
}

#[cfg(target_os = "macos")]
fn platform_screen_capture_permission_granted() -> bool {
    unsafe { macos_screen_capture_tcc::preflight_screen_capture_access() }
}

#[cfg(not(target_os = "macos"))]
fn platform_screen_capture_permission_granted() -> bool {
    false
}

#[cfg(target_os = "macos")]
mod macos_screen_capture_tcc {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }

    pub unsafe fn preflight_screen_capture_access() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }
}

pub fn select_builtin_h264_backend(
    entry: &ResourceEntry,
) -> Option<RemoteDesktopMediaBackendDescriptor> {
    if entry.kind == ResourceType::Display
        && entry.metadata.get("backend").and_then(Value::as_str) == Some("xcap")
    {
        return Some(XCAP_OPENH264_BACKEND);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::resources::{ResourceBinding, ResourceEntry};
    use serde_json::json;

    fn xcap_display_entry() -> ResourceEntry {
        ResourceEntry {
            resource_ura: "easynet:///r/acme/resource/display.test".into(),
            owner_agent: "easynet:///r/acme/device/01DEV".into(),
            kind: ResourceType::Display,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "display:xcap:test".into(),
            display_name: "Display".into(),
            metadata: json!({"backend": "xcap"}),
            first_seen_at: "2026-06-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn selects_only_available_xcap_display_backend() {
        let backend = select_builtin_h264_backend(&xcap_display_entry()).unwrap();

        assert_eq!(backend.backend_id(), XCAP_OPENH264_BACKEND_ID);
        assert_eq!(backend.sdk_id(), REMOTE_DESKTOP_MEDIA_SDK_ID);
        assert_eq!(backend.effective_fps(144), XCAP_MACOS_RECORDER_MAX_FPS);
        assert!(!backend.external_binary_required());
    }

    #[test]
    fn catalog_declares_native_plugin_state_per_platform() {
        let catalog = backend_catalog_view();

        assert_eq!(
            catalog[2]["backend_id"],
            json!(MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID)
        );
        assert_eq!(catalog[2]["external_binary_required"], json!(false));

        // The native ScreenCaptureKit + VideoToolbox plugin is compiled in on
        // macOS only; elsewhere the descriptor stays not_installed.
        #[cfg(target_os = "macos")]
        {
            assert_eq!(catalog[2]["status"], json!("available"));
            assert_eq!(catalog[2]["transport_ready"], json!(true));
            assert_eq!(catalog[2]["production_ready"], json!(true));
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(catalog[2]["status"], json!("not_installed"));
            assert_eq!(catalog[2]["transport_ready"], json!(false));
            assert_eq!(catalog[2]["production_ready"], json!(false));
        }
    }

    #[test]
    fn catalog_entries_validate_against_axon_contract() {
        XCAP_OPENH264_BACKEND.validate_axon_contract().unwrap();
        XCAP_OPENH264_WEBRTC_BACKEND
            .validate_axon_contract()
            .unwrap();
        MACOS_SCK_VIDEOTOOLBOX_BACKEND
            .validate_axon_contract()
            .unwrap();

        assert_eq!(
            XCAP_OPENH264_BACKEND.to_json()["transport"],
            json!("invoke_bidi")
        );
        assert_eq!(
            XCAP_OPENH264_WEBRTC_BACKEND.to_json()["transport"],
            json!("webrtc")
        );
        assert_eq!(
            XCAP_OPENH264_WEBRTC_BACKEND.to_json()["transport_ready"],
            json!(true)
        );
    }

    // On macOS the native ScreenCaptureKit + VideoToolbox plugin is the
    // production backend and is selected for WebRTC transport; the gate opens.
    #[cfg(target_os = "macos")]
    #[test]
    fn native_plugin_is_the_production_webrtc_backend_on_macos() {
        let entry = xcap_display_entry();

        assert_eq!(
            production_backend_for_entry(&entry).unwrap().backend_id(),
            MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID
        );
        assert_eq!(
            webrtc_transport_backend_for_entry(&entry)
                .unwrap()
                .backend_id(),
            MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID
        );
        assert_eq!(production_gate_view()["ready"], json!(true));
    }

    // Off macOS the native plugin is absent, so the WebRTC transport falls
    // back to the xcap+OpenH264 software path while the production gate stays
    // closed (flagship quality requires the native plugin).
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn direct_webrtc_can_start_before_flagship_native_backend_is_available() {
        let entry = xcap_display_entry();

        assert!(production_backend_for_entry(&entry).is_none());
        assert_eq!(
            webrtc_transport_backend_for_entry(&entry)
                .unwrap()
                .backend_id(),
            XCAP_OPENH264_WEBRTC_BACKEND_ID
        );
        assert_eq!(production_gate_view()["ready"], json!(false));
        assert_eq!(
            production_gate_view()["reason"],
            json!("native_plugin_requires_macos")
        );
    }
}

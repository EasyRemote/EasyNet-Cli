// EasyNet CLI — remote app media source factory
// ==============================================
//
// File: plugins/remote-desktop/src/transport/media_source.rs
// Description: Transport-owned media-source selection from resolved remote app
// target bindings.
//
// Architectural Boundary:
// - The session aggregate owns RemoteAppTargetBinding.
// - The transport layer owns media-source startup policy.
// - Production media paths consume bindings only; they do not resolve
//   ResourceEntry into native capture targets.

use crate::daemon::plugins::remote_desktop::constants::ABILITY_SET_DESCRIPTION;
use crate::daemon::plugins::remote_desktop::media::encode::BuiltinH264Config;
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, RemoteAppTargetError, RemoteDesktopTargetKind, TargetResolutionError,
};

/// Immutable request metadata for selecting a media source from a committed
/// target binding.
pub(in crate::daemon::plugins::remote_desktop) struct MediaStartRequest<'a> {
    pub(in crate::daemon::plugins::remote_desktop) config: &'a BuiltinH264Config,
}

/// Concrete media source selected for one direct WebRTC session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum RemoteAppMediaSource {
    NativeProduction,
    DisplayBaseline,
}

/// Factory boundary required by the remote-app targeted session model.
///
/// Implementations receive a resolved `RemoteAppTargetBinding`; `ResourceEntry`
/// is intentionally absent from this API so production transport cannot
/// re-resolve or drift from the session-owned binding.
pub(in crate::daemon::plugins::remote_desktop) trait RemoteAppMediaSourceFactory {
    fn start_from_binding(
        &self,
        binding: &RemoteAppTargetBinding,
        request: MediaStartRequest<'_>,
    ) -> Result<RemoteAppMediaSource, RemoteAppTargetError>;
}

/// Start media-source selection from the committed session target binding.
///
/// This function is the injectable boundary used by tests to prove that the
/// transport layer passes the stored binding into the media source factory.
pub(in crate::daemon::plugins::remote_desktop) fn start_remote_app_media_source(
    factory: &dyn RemoteAppMediaSourceFactory,
    binding: &RemoteAppTargetBinding,
    request: MediaStartRequest<'_>,
) -> Result<RemoteAppMediaSource, RemoteAppTargetError> {
    factory.start_from_binding(binding, request)
}

/// Direct WebRTC media-source factory for the builtin remote desktop plugin.
#[derive(Debug, Default, Clone, Copy)]
pub(in crate::daemon::plugins::remote_desktop) struct DirectWebRtcMediaSourceFactory;

impl RemoteAppMediaSourceFactory for DirectWebRtcMediaSourceFactory {
    fn start_from_binding(
        &self,
        binding: &RemoteAppTargetBinding,
        request: MediaStartRequest<'_>,
    ) -> Result<RemoteAppMediaSource, RemoteAppTargetError> {
        binding.require_capture_proof(ABILITY_SET_DESCRIPTION)?;
        if request.config.backend.production_ready() {
            #[cfg(target_os = "macos")]
            {
                return Ok(RemoteAppMediaSource::NativeProduction);
            }
            #[cfg(not(target_os = "macos"))]
            {
                return Err(RemoteAppTargetError::new(
                    ABILITY_SET_DESCRIPTION,
                    TargetResolutionError::CaptureBackendUnavailable,
                    format!(
                        "direct WebRTC native media is required for a production-ready {} target binding on this platform",
                        binding.target_kind().as_str()
                    ),
                ));
            }
        }

        if binding.target_kind() == RemoteDesktopTargetKind::Display {
            return Ok(RemoteAppMediaSource::DisplayBaseline);
        }

        Err(RemoteAppTargetError::new(
            ABILITY_SET_DESCRIPTION,
            TargetResolutionError::DisplayFallbackForbidden,
            format!(
                "direct WebRTC baseline capture is display-only and cannot satisfy a {} target binding",
                binding.target_kind().as_str()
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::RefCell;

    use serde_json::json;

    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::remote_desktop::media::XCAP_OPENH264_WEBRTC_BACKEND;
    use crate::daemon::plugins::remote_desktop::target::{
        AppWindowSetProof, RemoteAppTargetResolver, ResolvedCaptureTargetProof,
        ResourceEntryTargetResolver, TargetResolutionError,
    };
    use crate::daemon::plugins::remote_desktop::test_support::live_remote_target_metadata;

    fn binding_for(kind: ResourceType, metadata: serde_json::Value) -> RemoteAppTargetBinding {
        let metadata = match kind {
            ResourceType::Window | ResourceType::Application => {
                live_remote_target_metadata(metadata)
            }
            _ => metadata,
        };
        ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: format!(
                        "easynet:///r/acme/resource/device.01DEV/streams/{}.test",
                        kind.as_str()
                    ),
                    owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
                    kind,
                    binding: ResourceBinding::LocalDevice,
                    hardware_id: format!("{}:test", kind.as_str()),
                    display_name: "Test Target".to_string(),
                    metadata,
                    first_seen_at: "2026-06-01T00:00:00Z".to_string(),
                },
                "view_only",
                1,
            )
            .expect("binding resolves")
    }

    fn commit_test_capture_proof(binding: &mut RemoteAppTargetBinding) {
        let locator = binding.native_locator();
        let proof = ResolvedCaptureTargetProof::new(
            locator.capture_backend(),
            binding.target_kind(),
            locator.display_id(),
            locator.window_id(),
            locator.pid(),
            locator.app_identity().map(ToOwned::to_owned),
            locator.bundle_id().map(ToOwned::to_owned),
            Some((1280, 720)),
        );
        let proof = if binding.target_kind() == RemoteDesktopTargetKind::Application {
            proof.with_app_window_set(AppWindowSetProof::new(
                locator.display_id().expect("application display id"),
                locator.bundle_id().map(ToOwned::to_owned),
                locator.pid(),
                vec![7],
            ))
        } else {
            proof
        };
        binding
            .commit_capture_proof("remote_desktop.create_session", proof)
            .expect("test capture proof commits");
    }

    fn display_baseline_config() -> BuiltinH264Config {
        BuiltinH264Config {
            backend: XCAP_OPENH264_WEBRTC_BACKEND,
            requested_fps: 30,
            fps: 30,
            bitrate_kbps: 2_500,
            max_frame_queue_depth: 4,
            keyframe_interval_frames: 30,
        }
    }

    #[test]
    fn display_source_may_use_baseline_when_native_backend_is_not_selected() {
        let mut binding = binding_for(
            ResourceType::Display,
            json!({
                "backend": "xcap",
                "display_id": 1,
            }),
        );
        commit_test_capture_proof(&mut binding);
        let config = display_baseline_config();

        let source = DirectWebRtcMediaSourceFactory
            .start_from_binding(&binding, MediaStartRequest { config: &config })
            .expect("display baseline is allowed");

        assert_eq!(source, RemoteAppMediaSource::DisplayBaseline);
    }

    #[test]
    fn direct_factory_rejects_uncommitted_target_binding_before_media_selection() {
        let binding = binding_for(
            ResourceType::Display,
            json!({
                "backend": "xcap",
                "display_id": 1,
            }),
        );
        let config = display_baseline_config();

        let err = DirectWebRtcMediaSourceFactory
            .start_from_binding(&binding, MediaStartRequest { config: &config })
            .expect_err("media source startup requires committed capture proof");

        assert_eq!(
            err.reason(),
            TargetResolutionError::TargetMetadataIncomplete
        );
        assert_eq!(err.reason().frontend_action().as_str(), "show_unsupported");
    }

    #[test]
    fn fake_factory_receives_session_owned_binding_without_resource_re_resolution() {
        struct RecordingFactory {
            seen_binding_id: RefCell<Option<String>>,
        }

        impl RemoteAppMediaSourceFactory for RecordingFactory {
            fn start_from_binding(
                &self,
                binding: &RemoteAppTargetBinding,
                _request: MediaStartRequest<'_>,
            ) -> Result<RemoteAppMediaSource, RemoteAppTargetError> {
                self.seen_binding_id
                    .replace(Some(binding.binding_id().to_string()));
                Ok(RemoteAppMediaSource::DisplayBaseline)
            }
        }

        let binding = binding_for(
            ResourceType::Display,
            json!({
                "backend": "xcap",
                "display_id": 1,
            }),
        );
        let expected_binding_id = binding.binding_id().to_string();
        let config = display_baseline_config();
        let factory = RecordingFactory {
            seen_binding_id: RefCell::new(None),
        };

        let source = start_remote_app_media_source(
            &factory,
            &binding,
            MediaStartRequest { config: &config },
        )
        .expect("fake factory selects source");

        assert_eq!(source, RemoteAppMediaSource::DisplayBaseline);
        assert_eq!(
            factory.seen_binding_id.into_inner(),
            Some(expected_binding_id),
            "transport media source selection must pass the committed session binding"
        );
    }

    #[test]
    fn non_native_window_and_application_sources_fail_closed_before_display_baseline() {
        for (kind, metadata) in [
            (
                ResourceType::Window,
                json!({
                    "window_id": 7,
                    "pid": 9001,
                    "bundle_id": "com.example.Editor",
                }),
            ),
            (
                ResourceType::Application,
                json!({
                    "display_id": 1,
                    "bundle_id": "com.example.Editor",
                    "app_identity": "com.example.Editor",
                    "primary_pid": 9001,
                    "resolved_window_ids": [7],
                    "window_set_epoch": 42,
                }),
            ),
        ] {
            let mut binding = binding_for(kind, metadata);
            commit_test_capture_proof(&mut binding);
            let config = display_baseline_config();

            let err = DirectWebRtcMediaSourceFactory
                .start_from_binding(&binding, MediaStartRequest { config: &config })
                .expect_err("app/window must not fall back to display baseline");

            assert_eq!(
                err.reason(),
                TargetResolutionError::DisplayFallbackForbidden
            );
            assert_eq!(err.reason().frontend_action().as_str(), "show_unsupported");
        }
    }
}

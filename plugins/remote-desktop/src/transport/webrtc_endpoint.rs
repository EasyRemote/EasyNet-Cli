// EasyNet CLI — direct WebRTC endpoint setup
// ===========================================
//
// File: plugins/remote-desktop/src/transport/webrtc_endpoint.rs
// Description: Device-side direct WebRTC endpoint construction and startup.
//
// Protocol Responsibility:
// - Answer a browser offer with one negotiated H.264 video sender and the
//   remote-desktop input data channel on a shared ICE/DTLS transport.
//
// Implementation Approach:
// - Apply the remote offer before attaching the local track so the sender is
//   bound to the browser-offered video transceiver, then validate the answer
//   before publishing the endpoint.
//
// Usage Contract:
// - Callers provide a validated session/resource and must stop the endpoint
//   when later session commit or authorization fails.
//
// Architectural Position:
// - Device plugin transport boundary; session policy remains in the lifecycle
//   service and Axon owns only invocation/receipt semantics.

use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use rtc::ice::mdns::MulticastDnsMode;
use rtc::interceptor::Registry;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::interceptor_registry::{
    configure_nack, configure_rtcp_reports, configure_simulcast_extension_headers,
    configure_twcc_sender_only,
};
use rtc::peer_connection::configuration::media_engine::{
    MediaEngine, MIME_TYPE_H264, MIME_TYPE_OPUS,
};
use rtc::peer_connection::configuration::setting_engine::SettingEngine;
use rtc::peer_connection::transport::RTCDtlsRole;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
    RtpCodecKind,
};
use rtc::rtp_transceiver::PayloadType;
use serde_json::{json, Value};
use tokio::sync::watch;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, RTCConfigurationBuilder, RTCIceServer,
    RTCSessionDescription,
};
use webrtc::runtime::{channel, default_runtime};

use crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenCaptureOptions;
use crate::daemon::plugins::remote_desktop::constants::{
    direct_webrtc_endpoint_ura, ABILITY_SET_DESCRIPTION, DIRECT_WEBRTC_H264_PREFERRED_PAYLOAD_TYPE,
    REASON_RESOURCE_TYPE_MISMATCH, TRANSPORT_WEBRTC,
};
use crate::daemon::plugins::remote_desktop::input::EffectiveRemoteDesktopInputPolicy;
use crate::daemon::plugins::remote_desktop::media::encode::build_direct_webrtc_h264_config_for_binding;
use crate::daemon::plugins::remote_desktop::media::host_audio_capability::HostAudioRuntimeSnapshot;
#[cfg(not(any(
    all(feature = "native-media", target_os = "windows"),
    all(feature = "native-media", target_os = "linux")
)))]
use crate::daemon::plugins::remote_desktop::media::host_audio_capability::HostAudioSourceClass;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows")
))]
use crate::daemon::plugins::remote_desktop::media::host_audio_capability::REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE;
use crate::daemon::plugins::remote_desktop::network::{
    direct_webrtc_route_candidate_evidence, ConfiguredDirectWebRtcRouteProvider,
    DirectWebRtcIceServerConfig, DirectWebRtcRouteCandidateProvider,
};
use crate::daemon::plugins::remote_desktop::relay_lease::RemoteDesktopRelayLease;
use crate::daemon::plugins::remote_desktop::sdp::{
    ensure_answer_sends_audio, ensure_answer_sends_video, normalize_browser_answer_sdp,
    normalize_remote_offer_sdp, remote_offer_accepts_audio, remote_offer_h264_receive_limits,
};
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::target::RemoteAppTargetBinding;
use crate::daemon::plugins::remote_desktop::transport::{
    apply_pending_remote_ice_candidates, close_peer_connection_bounded,
    run_direct_webrtc_media_loop, DirectWebRtcEndpoint, DirectWebRtcHandler,
    DirectWebRtcHandlerConfig, DirectWebRtcSession, RemoteDesktopTransportManager,
};

const DIRECT_WEBRTC_ICE_GATHER_TIMEOUT_MS: u64 = 2_500;
const DIRECT_WEBRTC_SETUP_DEADLINE: Duration = Duration::from_secs(10);
const DIRECT_WEBRTC_OPUS_PAYLOAD_TYPE: PayloadType = 111;

struct DirectWebRtcEndpointSetupFailure {
    source: anyhow::Error,
    unsettled_peer_connection: Option<Arc<dyn PeerConnection>>,
}

impl fmt::Debug for DirectWebRtcEndpointSetupFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectWebRtcEndpointSetupFailure")
            .field("source", &self.source)
            .field(
                "has_unsettled_peer_connection",
                &self.unsettled_peer_connection.is_some(),
            )
            .finish()
    }
}

impl fmt::Display for DirectWebRtcEndpointSetupFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.source, formatter)
    }
}

impl StdError for DirectWebRtcEndpointSetupFailure {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.source.as_ref())
    }
}

fn assert_direct_webrtc_endpoint_start_unlocked() {
    RemoteDesktopSessionStore::assert_current_thread_unlocked(
        "remote_desktop.webrtc.start_direct_webrtc_endpoint",
    );
}

#[cfg(all(
    test,
    not(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "windows")
    ))
))]
mod host_audio_admission_tests {
    use super::*;
    use crate::daemon::plugins::remote_desktop::media::host_audio_capability::{
        HostAudioRuntimeSnapshot, REASON_HOST_AUDIO_SNAPSHOT_EXPIRED,
        REASON_PIPEWIRE_RUNTIME_UNAVAILABLE,
    };
    use crate::daemon::plugins::remote_desktop::test_support::test_session_init;

    fn binding() -> RemoteAppTargetBinding {
        test_session_init(
            "rd-audio-admission",
            "easynet:///r/acme/resource/display.audio-admission",
            vec!["webrtc".to_string()],
        )
        .target_binding
    }

    #[test]
    fn expired_ready_snapshot_fails_audio_offer_admission_closed() {
        let mut runtime = HostAudioRuntimeSnapshot::for_test(true, true, true, true, None);
        runtime.expire_for_test();
        let error = admit_host_audio_offer(&binding(), &runtime)
            .expect_err("expired capability must not authorize an offer")
            .to_string();
        assert!(
            error.contains(REASON_HOST_AUDIO_SNAPSHOT_EXPIRED),
            "{error}"
        );
    }

    #[test]
    fn unreachable_runtime_fails_audio_offer_admission_closed() {
        let runtime = HostAudioRuntimeSnapshot::for_test(
            true,
            false,
            false,
            false,
            Some(REASON_PIPEWIRE_RUNTIME_UNAVAILABLE),
        );
        let error = admit_host_audio_offer(&binding(), &runtime)
            .expect_err("unreachable runtime must not authorize an offer")
            .to_string();
        assert!(
            error.contains(REASON_PIPEWIRE_RUNTIME_UNAVAILABLE),
            "{error}"
        );
    }
}

#[cfg(all(
    test,
    feature = "native-media",
    any(target_os = "linux", target_os = "windows")
))]
#[test]
fn hosted_media_rejects_audio_until_active_session_opus_exists() {
    use crate::daemon::plugins::remote_desktop::media::host_audio_capability::HostAudioRuntimeSnapshot;
    use crate::daemon::plugins::remote_desktop::test_support::test_session_init;

    let binding = test_session_init(
        "rd-linux-hosted-audio-admission",
        "easynet:///r/acme/resource/display.audio-admission",
        vec!["webrtc".to_string()],
    )
    .target_binding;
    let runtime = HostAudioRuntimeSnapshot::for_test(true, true, true, true, None);
    let error = admit_host_audio_offer(&binding, &runtime)
        .expect_err("hosted media cannot negotiate an audio track it cannot emit")
        .to_string();
    assert!(error.contains(REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE));
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows")
))]
fn admit_host_audio_offer(
    binding: &RemoteAppTargetBinding,
    runtime: &HostAudioRuntimeSnapshot,
) -> anyhow::Result<()> {
    let _ = (binding, runtime);
    anyhow::bail!(
        "RemoteApp audio-video negotiation rejected: {} media-host session audio is not implemented; reason={REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE}",
        std::env::consts::OS,
    )
}

#[cfg(not(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows")
)))]
fn admit_host_audio_offer(
    binding: &RemoteAppTargetBinding,
    runtime: &HostAudioRuntimeSnapshot,
) -> anyhow::Result<()> {
    let source = {
        let _ = binding;
        HostAudioSourceClass::SystemLoopback
    };

    if !runtime.compiled_supported() {
        anyhow::bail!(
            "RemoteApp audio-video negotiation rejected: host audio is not compiled; reason={}",
            runtime.runtime_blocker().unwrap_or("native_media_disabled")
        );
    }
    if !runtime.is_fresh() {
        anyhow::bail!(
            "RemoteApp audio-video negotiation rejected: host audio runtime snapshot expired; reason={}",
            runtime
                .admission_blocker(source)
                .expect("expired snapshots always have an admission blocker")
        );
    }
    if !runtime.runtime_reachable() {
        anyhow::bail!(
            "RemoteApp audio-video negotiation rejected: host audio runtime is unreachable; reason={}",
            runtime
                .runtime_blocker()
                .unwrap_or("host_audio_runtime_unavailable")
        );
    }
    let readiness = runtime.source(source);
    if !readiness.is_ready() {
        anyhow::bail!(
            "RemoteApp audio-video negotiation rejected: {} runtime source is not ready; reason={}",
            source.as_str(),
            readiness
                .blocker()
                .or_else(|| runtime.admission_blocker(source))
                .unwrap_or("host_audio_runtime_unavailable")
        );
    }
    Ok(())
}

pub(in crate::daemon::plugins::remote_desktop) struct StartDirectWebRtcEndpointRequest {
    pub(in crate::daemon::plugins::remote_desktop) sessions: Arc<RemoteDesktopSessionStore>,
    pub(in crate::daemon::plugins::remote_desktop) transports: Arc<RemoteDesktopTransportManager>,
    pub(in crate::daemon::plugins::remote_desktop) session_id: String,
    pub(in crate::daemon::plugins::remote_desktop) epoch: TransportEpoch,
    pub(in crate::daemon::plugins::remote_desktop) target_binding: RemoteAppTargetBinding,
    pub(in crate::daemon::plugins::remote_desktop) options: ScreenCaptureOptions,
    pub(in crate::daemon::plugins::remote_desktop) target_bitrate_kbps: u32,
    pub(in crate::daemon::plugins::remote_desktop) max_frame_queue_depth: usize,
    pub(in crate::daemon::plugins::remote_desktop) input_policy: EffectiveRemoteDesktopInputPolicy,
    pub(in crate::daemon::plugins::remote_desktop) offer_sdp: String,
    pub(in crate::daemon::plugins::remote_desktop) relay_lease: Option<RemoteDesktopRelayLease>,
    pub(in crate::daemon::plugins::remote_desktop) host_audio_runtime:
        crate::daemon::plugins::remote_desktop::media::host_audio_capability::HostAudioRuntimeSnapshot,
}

pub(in crate::daemon::plugins::remote_desktop) fn start_direct_webrtc_endpoint(
    request: StartDirectWebRtcEndpointRequest,
) -> anyhow::Result<Value> {
    assert_direct_webrtc_endpoint_start_unlocked();

    let StartDirectWebRtcEndpointRequest {
        sessions,
        transports,
        session_id,
        epoch,
        target_binding,
        options,
        target_bitrate_kbps,
        max_frame_queue_depth,
        input_policy,
        offer_sdp,
        relay_lease,
        host_audio_runtime,
    } = request;

    let reservation = transports.reserve_endpoint(session_id.clone(), epoch)?;
    let stop_rx = reservation.stop_receiver();
    let build = transports.block_on(create_direct_webrtc_endpoint(DirectWebRtcEndpointConfig {
        sessions: Arc::clone(&sessions),
        transports: Arc::clone(&transports),
        session_id: session_id.clone(),
        epoch,
        target_binding,
        options,
        target_bitrate_kbps,
        max_frame_queue_depth,
        input_policy,
        offer_sdp,
        relay_lease,
        host_audio_runtime,
        stop_rx,
    }))?;
    let (answer, peer_connection, completion) = match build {
        Ok(endpoint) => endpoint,
        Err(error) => {
            return match error.downcast::<DirectWebRtcEndpointSetupFailure>() {
                Ok(mut failure) => {
                    if let Some(peer_connection) = failure.unsettled_peer_connection.take() {
                        reservation.complete_with_endpoint_cleanup(peer_connection);
                    } else {
                        reservation.complete_without_endpoint();
                    }
                    Err(failure.source)
                }
                Err(error) => {
                    reservation.complete_without_endpoint();
                    Err(error)
                }
            };
        }
    };
    if !reservation.commit(
        DirectWebRtcEndpoint {
            epoch,
            peer_connection,
        },
        completion,
    ) {
        anyhow::bail!(
            "{ABILITY_SET_DESCRIPTION}: session {session_id:?} entered terminal lifecycle while direct WebRTC endpoint was starting"
        );
    }
    if let Err(err) =
        apply_pending_remote_ice_candidates(&sessions, &transports, &session_id, epoch)
    {
        transports.stop_endpoint_if_epoch(&session_id, epoch);
        return Err(err);
    }
    Ok(answer)
}

struct DirectWebRtcEndpointConfig {
    sessions: Arc<RemoteDesktopSessionStore>,
    transports: Arc<RemoteDesktopTransportManager>,
    session_id: String,
    epoch: TransportEpoch,
    target_binding: RemoteAppTargetBinding,
    options: ScreenCaptureOptions,
    target_bitrate_kbps: u32,
    max_frame_queue_depth: usize,
    input_policy: EffectiveRemoteDesktopInputPolicy,
    offer_sdp: String,
    relay_lease: Option<RemoteDesktopRelayLease>,
    host_audio_runtime:
        crate::daemon::plugins::remote_desktop::media::host_audio_capability::HostAudioRuntimeSnapshot,
    stop_rx: watch::Receiver<bool>,
}

async fn create_direct_webrtc_endpoint(
    endpoint_config: DirectWebRtcEndpointConfig,
) -> anyhow::Result<(Value, Arc<dyn PeerConnection>, std::thread::JoinHandle<()>)> {
    let setup_deadline = tokio::time::Instant::now() + DIRECT_WEBRTC_SETUP_DEADLINE;
    let mut setup_stop_rx = endpoint_config.stop_rx.clone();
    let offer_sdp = normalize_remote_offer_sdp(&endpoint_config.offer_sdp);
    let h264_receive_limits = remote_offer_h264_receive_limits(&offer_sdp).map_err(|error| {
        anyhow::anyhow!(
            "{ABILITY_SET_DESCRIPTION}: browser H.264 receive contract rejected: {error}; reason=webrtc_h264_receive_contract_invalid"
        )
    })?;
    let (media_options, negotiated_bitrate_kbps) = h264_receive_limits
        .constrain(&endpoint_config.options, endpoint_config.target_bitrate_kbps)
        .map_err(|error| {
            anyhow::anyhow!(
                "{ABILITY_SET_DESCRIPTION}: browser H.264 receive limits cannot admit the requested stream: {error}; reason=webrtc_h264_receive_contract_invalid"
            )
        })?;
    let mut media_config = build_direct_webrtc_h264_config_for_binding(
        &endpoint_config.target_binding,
        &media_options,
        negotiated_bitrate_kbps,
        endpoint_config.max_frame_queue_depth,
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_SET_DESCRIPTION}: no compatible direct WebRTC media backend for subject type {}; reason={REASON_RESOURCE_TYPE_MISMATCH}",
            endpoint_config.target_binding.target_kind().as_str()
        )
    })?;
    media_config.requested_fps = endpoint_config.options.fps;
    media_config.bitrate_kbps = negotiated_bitrate_kbps;
    media_config.h264_level = h264_receive_limits.level();
    let negotiated_resolution = media_options
        .resolution
        .expect("H264ReceiveLimits::constrain guarantees explicit resolution");
    let negotiated_h264_level = h264_receive_limits.level().as_str();
    let audio_offered = remote_offer_accepts_audio(&offer_sdp);
    if audio_offered {
        admit_host_audio_offer(
            &endpoint_config.target_binding,
            &endpoint_config.host_audio_runtime,
        )?;
    }

    let mut media_engine = MediaEngine::default();
    let video_codec = RTCRtpCodecParameters {
        rtp_codec: RTCRtpCodec {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                .to_owned(),
            rtcp_feedback: vec![],
        },
        payload_type: DIRECT_WEBRTC_H264_PREFERRED_PAYLOAD_TYPE,
    };
    media_engine.register_codec(video_codec.clone(), RtpCodecKind::Video)?;
    let audio_codec = RTCRtpCodecParameters {
        rtp_codec: RTCRtpCodec {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: 48_000,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
            rtcp_feedback: vec![],
        },
        payload_type: DIRECT_WEBRTC_OPUS_PAYLOAD_TYPE,
    };
    media_engine.register_codec(audio_codec.clone(), RtpCodecKind::Audio)?;
    // RemoteApp is a media sender, so TWCC sequence numbers and Browser
    // feedback must be configured in the sender direction. The crate default
    // is receiver-only and is suitable for consuming remote media, not for the
    // device-to-Browser video path owned by this endpoint.
    let registry = configure_nack(Registry::new(), &mut media_engine);
    let registry = configure_rtcp_reports(registry);
    configure_simulcast_extension_headers(&mut media_engine)?;
    let registry = configure_twcc_sender_only(registry, &mut media_engine)?;
    let route_candidate_provider = ConfiguredDirectWebRtcRouteProvider::from_env_with_relay_lease(
        endpoint_config.relay_lease.as_ref(),
    )?;
    let route_candidates = route_candidate_provider.route_candidates();
    let route_candidate_evidence =
        direct_webrtc_route_candidate_evidence(&route_candidate_provider, &route_candidates);
    let ice_servers = route_candidate_provider
        .ice_servers()
        .iter()
        .map(rtc_ice_server_from_route_config)
        .collect::<Vec<_>>();
    // Local direct WebRTC starts host-first and never depends on a public
    // STUN/TURN default. Deployment-owned STUN/TURN/EasyNet relay routes are
    // explicit provider configuration and are fed into RTC only as ICE servers.
    let rtc_config = RTCConfigurationBuilder::new()
        .with_ice_servers(ice_servers)
        .build();
    let mut setting_engine = SettingEngine::default();
    setting_engine.set_answering_dtls_role(RTCDtlsRole::Client)?;
    // QueryOnly accepts and resolves the browser's mDNS (.local) host
    // candidates. On localhost Chrome only emits .local host + srflx, so
    // dropping .local would leave the device with no usable remote host
    // candidate and ICE would stall in `checking`.
    setting_engine.set_multicast_dns_mode(MulticastDnsMode::QueryOnly);

    let (gather_complete_tx, mut gather_complete_rx) = channel::<()>(1);
    let (connected_tx, connected_rx) = channel::<()>(1);
    let (done_tx, done_rx) = channel::<()>(1);
    let runtime =
        default_runtime().ok_or_else(|| anyhow::anyhow!("no WebRTC async runtime available"))?;
    let handler = Arc::new(DirectWebRtcHandler::new(DirectWebRtcHandlerConfig {
        sessions: Arc::clone(&endpoint_config.sessions),
        transports: Arc::downgrade(&endpoint_config.transports),
        session_id: endpoint_config.session_id.clone(),
        epoch: endpoint_config.epoch,
        input_policy: endpoint_config.input_policy.clone(),
        gather_complete_tx,
        connected_tx,
        done_tx,
    }));
    let udp_addrs = route_candidates
        .iter()
        .filter_map(|candidate| candidate.local_bind_endpoint().map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    eprintln!(
        "[remote-desktop-webrtc] session={} route_provider={} provider_state={} local_bind_addrs={}",
        endpoint_config.session_id,
        route_candidate_provider.provider_id(),
        route_candidate_provider.provider_state(),
        udp_addrs.join(",")
    );
    let peer_connection: Arc<dyn PeerConnection> = await_direct_webrtc_setup_phase(
        &mut setup_stop_rx,
        setup_deadline,
        "peer construction",
        async move {
            let peer_connection = PeerConnectionBuilder::new()
                .with_configuration(rtc_config)
                .with_media_engine(media_engine)
                .with_interceptor_registry(registry)
                .with_setting_engine(setting_engine)
                .with_handler(handler)
                .with_runtime(runtime)
                .with_udp_addrs(udp_addrs)
                .build()
                .await?;
            Ok(Arc::new(peer_connection) as Arc<dyn PeerConnection>)
        },
    )
    .await?;
    let peer_connection_for_cleanup = Arc::clone(&peer_connection);
    let setup = async move {
        // Answerer ordering is load-bearing. Applying the offer first materializes
        // the browser's recvonly video transceiver; add_track can then reuse it.
        // Adding the track before set_remote_description creates an unassociated
        // sender in rtc rc.4: ICE/data channels connect and RTP is written, but the
        // browser never receives an ontrack event.
        let offer: RTCSessionDescription =
            serde_json::from_value(json!({ "type": "offer", "sdp": offer_sdp }))?;
        peer_connection.set_remote_description(offer).await?;

        let track = Arc::new(TrackLocalStaticSample::new(MediaStreamTrack::new(
            format!("easynet-rd-stream-{}", endpoint_config.session_id),
            format!("easynet-rd-video-{}", endpoint_config.session_id),
            "EasyNet Remote Desktop".to_string(),
            RtpCodecKind::Video,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(rand::random::<u32>()),
                    ..Default::default()
                },
                codec: video_codec.rtp_codec.clone(),
                ..Default::default()
            }],
        ))?);
        let rtp_sender = peer_connection
            .add_track(Arc::clone(&track) as Arc<dyn TrackLocal>)
            .await?;

        let audio_track_and_sender = if audio_offered {
            let audio_track = Arc::new(TrackLocalStaticSample::new(MediaStreamTrack::new(
                format!("easynet-rd-stream-{}", endpoint_config.session_id),
                format!("easynet-rd-audio-{}", endpoint_config.session_id),
                "EasyNet Remote Desktop Audio".to_string(),
                RtpCodecKind::Audio,
                vec![RTCRtpEncodingParameters {
                    rtp_coding_parameters: RTCRtpCodingParameters {
                        ssrc: Some(rand::random::<u32>()),
                        ..Default::default()
                    },
                    codec: audio_codec.rtp_codec.clone(),
                    ..Default::default()
                }],
            ))?);
            let sender = peer_connection
                .add_track(Arc::clone(&audio_track) as Arc<dyn TrackLocal>)
                .await?;
            Some((audio_track, sender))
        } else {
            None
        };

        let answer = peer_connection.create_answer(None).await?;
        ensure_answer_sends_video(&answer.sdp)?;
        if audio_offered {
            ensure_answer_sends_audio(&answer.sdp)?;
        }
        peer_connection.set_local_description(answer).await?;
        let _ = tokio::time::timeout(
            Duration::from_millis(DIRECT_WEBRTC_ICE_GATHER_TIMEOUT_MS),
            gather_complete_rx.recv(),
        )
        .await;

        let local = peer_connection
            .local_description()
            .await
            .ok_or_else(|| anyhow::anyhow!("direct WebRTC local description missing"))?;
        let payload_type: PayloadType = rtp_sender
            .get_parameters()
            .await?
            .rtp_parameters
            .codecs
            .first()
            .map(|codec| codec.payload_type)
            .ok_or_else(|| anyhow::anyhow!("direct WebRTC sender has no negotiated codec"))?;
        let (audio_track, audio_payload_type) = match audio_track_and_sender {
            Some((track, sender)) => {
                let payload_type = sender
                    .get_parameters()
                    .await?
                    .rtp_parameters
                    .codecs
                    .first()
                    .map(|codec| codec.payload_type)
                    .ok_or_else(|| {
                        anyhow::anyhow!("direct WebRTC audio sender has no negotiated codec")
                    })?;
                (Some(track), Some(payload_type))
            }
            None => (None, None),
        };
        let answer_sdp = normalize_browser_answer_sdp(&local.sdp);
        eprintln!(
        "[remote-desktop-webrtc] answer_candidate_lines={} negotiated_payload_type={payload_type}",
        answer_sdp
            .lines()
            .filter(|line| line.starts_with("a=candidate:"))
            .count()
    );
        let answer_value = json!({
            "type": "answer",
            "sdp": answer_sdp,
            "transport": TRANSPORT_WEBRTC,
            "endpoint_ura": direct_webrtc_endpoint_ura(&endpoint_config.session_id),
            "codec": "h264",
            "codec_profile": "baseline",
            "codec_level": negotiated_h264_level,
            "video_width": negotiated_resolution.width,
            "video_height": negotiated_resolution.height,
            "video_fps": media_config.fps,
            "video_bitrate_kbps": media_config.bitrate_kbps,
            "audio_codec": audio_offered.then_some("opus"),
            "media_scope": if audio_offered { "audio_video" } else { "video_only" },
            "carrier": "rtp_srtp",
            "route_candidate_evidence": route_candidate_evidence,
        });

        let peer_connection_for_endpoint = Arc::clone(&peer_connection);
        let session_id = endpoint_config.session_id;
        let epoch = endpoint_config.epoch;
        let sessions = Arc::clone(&endpoint_config.sessions);
        let transport_runtime = endpoint_config.transports.runtime_handle()?;
        let completion = std::thread::Builder::new()
            .name("easynet-remote-desktop-webrtc".into())
            .spawn(move || {
                transport_runtime.block_on(run_direct_webrtc_media_loop(
                    Arc::clone(&sessions),
                    DirectWebRtcSession {
                        session_id,
                        epoch,
                        peer_connection,
                        track,
                        video_sender: rtp_sender,
                        payload_type,
                        audio_track,
                        audio_payload_type,
                        target_binding: endpoint_config.target_binding,
                        options: media_options,
                        config: media_config,
                    },
                    connected_rx,
                    done_rx,
                    endpoint_config.stop_rx,
                ));
            })
            .map_err(|err| anyhow::anyhow!("spawn direct WebRTC media loop: {err}"))?;

        Ok((answer_value, peer_connection_for_endpoint, completion))
    };
    let setup_result = await_direct_webrtc_setup_phase(
        &mut setup_stop_rx,
        setup_deadline,
        "description and media setup",
        setup,
    )
    .await;
    match setup_result {
        Ok(endpoint) => Ok(endpoint),
        Err(source) => {
            let settled = close_peer_connection_bounded(&peer_connection_for_cleanup).await;
            Err(DirectWebRtcEndpointSetupFailure {
                source,
                unsettled_peer_connection: (!settled).then_some(peer_connection_for_cleanup),
            }
            .into())
        }
    }
}

async fn await_direct_webrtc_setup_phase<T, F>(
    stop_rx: &mut watch::Receiver<bool>,
    deadline: tokio::time::Instant,
    phase: &'static str,
    future: F,
) -> anyhow::Result<T>
where
    F: Future<Output = anyhow::Result<T>>,
{
    if *stop_rx.borrow() {
        anyhow::bail!(
            "{ABILITY_SET_DESCRIPTION}: direct WebRTC endpoint setup cancelled during {phase}: session transport admission was cancelled"
        );
    }
    tokio::pin!(future);
    loop {
        tokio::select! {
            biased;
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow_and_update() {
                    let detail = if changed.is_err() {
                        "reservation channel closed"
                    } else {
                        "session transport admission was cancelled"
                    };
                    anyhow::bail!(
                        "{ABILITY_SET_DESCRIPTION}: direct WebRTC endpoint setup cancelled during {phase}: {detail}"
                    );
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                anyhow::bail!(
                    "{ABILITY_SET_DESCRIPTION}: direct WebRTC endpoint setup exceeded {}ms during {phase}",
                    DIRECT_WEBRTC_SETUP_DEADLINE.as_millis()
                );
            }
            result = &mut future => return result,
        }
    }
}

fn rtc_ice_server_from_route_config(config: &DirectWebRtcIceServerConfig) -> RTCIceServer {
    RTCIceServer {
        urls: config.urls().to_vec(),
        username: config.username().to_string(),
        credential: config.credential().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "remote_desktop.webrtc.start_direct_webrtc_endpoint")]
    fn endpoint_start_boundary_refuses_to_run_while_session_store_lock_is_held() {
        let store = RemoteDesktopSessionStore::new();
        let _guard = store.lock();

        assert_direct_webrtc_endpoint_start_unlocked();
    }

    #[tokio::test]
    async fn endpoint_setup_phase_is_interrupted_by_terminal_admission_cancel() {
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let cancel = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            stop_tx
                .send(true)
                .expect("terminal transition publishes endpoint setup cancellation");
        });
        let started = tokio::time::Instant::now();
        let result = await_direct_webrtc_setup_phase(
            &mut stop_rx,
            started + Duration::from_secs(1),
            "test phase",
            std::future::pending::<anyhow::Result<()>>(),
        )
        .await;

        assert!(result
            .expect_err("terminal admission cancel interrupts endpoint setup")
            .to_string()
            .contains("session transport admission was cancelled"));
        assert!(started.elapsed() < Duration::from_millis(500));
        cancel.await.expect("cancellation publisher exits");
    }

    #[tokio::test]
    async fn endpoint_setup_phase_enforces_one_absolute_deadline() {
        let (_stop_tx, mut stop_rx) = watch::channel(false);
        let started = tokio::time::Instant::now();
        let result = await_direct_webrtc_setup_phase(
            &mut stop_rx,
            started + Duration::from_millis(20),
            "test phase",
            std::future::pending::<anyhow::Result<()>>(),
        )
        .await;

        assert!(result
            .expect_err("setup deadline interrupts a hung phase")
            .to_string()
            .contains("exceeded"));
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}

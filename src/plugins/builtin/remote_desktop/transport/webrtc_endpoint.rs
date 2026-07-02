// EasyNet CLI — direct WebRTC endpoint setup
// ===========================================
//
// File: src/plugins/builtin/remote_desktop/transport/webrtc_endpoint.rs
// Description: Device-side direct WebRTC endpoint construction and startup.

use std::sync::Arc;
use std::time::Duration;

use rtc::ice::mdns::MulticastDnsMode;
use rtc::interceptor::Registry;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::{MediaEngine, MIME_TYPE_H264};
use rtc::peer_connection::configuration::setting_engine::SettingEngine;
use rtc::peer_connection::transport::RTCDtlsRole;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
    RtpCodecKind,
};
use serde_json::{json, Value};
use tokio::sync::watch;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, RTCConfigurationBuilder, RTCSessionDescription,
};
use webrtc::runtime::{channel, default_runtime};

use crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenCaptureOptions;
use crate::persistence::resources::ResourceEntry;
use crate::plugins::remote_desktop::constants::{
    ABILITY_SET_DESCRIPTION, DIRECT_WEBRTC_ENDPOINT_PREFIX, REASON_RESOURCE_TYPE_MISMATCH,
    TRANSPORT_WEBRTC,
};
use crate::plugins::remote_desktop::media::encode::build_direct_webrtc_h264_config;
use crate::plugins::remote_desktop::network::direct_webrtc_udp_addrs;
use crate::plugins::remote_desktop::sdp::{
    normalize_browser_answer_sdp, normalize_remote_offer_sdp,
};
use crate::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::plugins::remote_desktop::transport::{
    apply_pending_remote_ice_candidates, run_direct_webrtc_media_loop, DirectWebRtcEndpoint,
    DirectWebRtcHandler, DirectWebRtcSession, RemoteDesktopTransportManager,
};

const DIRECT_WEBRTC_ICE_GATHER_TIMEOUT_MS: u64 = 2_500;

pub(in crate::plugins::builtin::remote_desktop) struct StartDirectWebRtcEndpointRequest {
    pub(in crate::plugins::builtin::remote_desktop) sessions: Arc<RemoteDesktopSessionStore>,
    pub(in crate::plugins::builtin::remote_desktop) transports: Arc<RemoteDesktopTransportManager>,
    pub(in crate::plugins::builtin::remote_desktop) session_id: String,
    pub(in crate::plugins::builtin::remote_desktop) entry: ResourceEntry,
    pub(in crate::plugins::builtin::remote_desktop) options: ScreenCaptureOptions,
    pub(in crate::plugins::builtin::remote_desktop) target_bitrate_kbps: u32,
    pub(in crate::plugins::builtin::remote_desktop) max_frame_queue_depth: usize,
    pub(in crate::plugins::builtin::remote_desktop) input_policy: Value,
    pub(in crate::plugins::builtin::remote_desktop) offer_sdp: String,
}

pub(in crate::plugins::builtin::remote_desktop) fn start_direct_webrtc_endpoint(
    request: StartDirectWebRtcEndpointRequest,
) -> anyhow::Result<Value> {
    let StartDirectWebRtcEndpointRequest {
        sessions,
        transports,
        session_id,
        entry,
        options,
        target_bitrate_kbps,
        max_frame_queue_depth,
        input_policy,
        offer_sdp,
    } = request;

    let (stop_tx, stop_rx) = watch::channel(false);
    let (answer, peer_connection) =
        transports.block_on(create_direct_webrtc_endpoint(DirectWebRtcEndpointConfig {
            sessions: Arc::clone(&sessions),
            transports: Arc::clone(&transports),
            session_id: session_id.clone(),
            entry,
            options,
            target_bitrate_kbps,
            max_frame_queue_depth,
            input_policy,
            offer_sdp,
            stop_rx,
        }))?;
    let old = transports.replace_endpoint(
        session_id.clone(),
        DirectWebRtcEndpoint {
            stop_tx,
            peer_connection,
        },
    );
    if let Some(old) = old {
        let _ = old.stop_tx.send(true);
    }
    if let Err(err) = apply_pending_remote_ice_candidates(&sessions, &transports, &session_id) {
        transports.stop_endpoint(&session_id);
        return Err(err);
    }
    Ok(answer)
}

struct DirectWebRtcEndpointConfig {
    sessions: Arc<RemoteDesktopSessionStore>,
    transports: Arc<RemoteDesktopTransportManager>,
    session_id: String,
    entry: ResourceEntry,
    options: ScreenCaptureOptions,
    target_bitrate_kbps: u32,
    max_frame_queue_depth: usize,
    input_policy: Value,
    offer_sdp: String,
    stop_rx: watch::Receiver<bool>,
}

async fn create_direct_webrtc_endpoint(
    endpoint_config: DirectWebRtcEndpointConfig,
) -> anyhow::Result<(Value, Arc<dyn PeerConnection>)> {
    let media_config = build_direct_webrtc_h264_config(
        &endpoint_config.entry,
        &endpoint_config.options,
        endpoint_config.target_bitrate_kbps,
        endpoint_config.max_frame_queue_depth,
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_SET_DESCRIPTION}: no compatible direct WebRTC media backend for subject type {}; reason={REASON_RESOURCE_TYPE_MISMATCH}",
            endpoint_config.entry.kind.as_str()
        )
    })?;

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
        payload_type: 102,
    };
    media_engine.register_codec(video_codec.clone(), RtpCodecKind::Video)?;
    let registry = register_default_interceptors(Registry::new(), &mut media_engine)?;
    // Local direct WebRTC starts host-first. Production deployments can add
    // deployment-owned TURN policy at the signaling layer; this runtime path
    // must not depend on a third-party STUN server to connect localhost/LAN.
    let rtc_config = RTCConfigurationBuilder::new().build();
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
    let handler = Arc::new(DirectWebRtcHandler::new(
        Arc::clone(&endpoint_config.sessions),
        endpoint_config.session_id.clone(),
        endpoint_config.input_policy.clone(),
        gather_complete_tx,
        connected_tx,
        done_tx,
    ));
    let udp_addrs = direct_webrtc_udp_addrs();
    eprintln!(
        "[remote-desktop-webrtc] session={} udp_addrs={}",
        endpoint_config.session_id,
        udp_addrs.join(",")
    );
    let peer_connection: Arc<dyn PeerConnection> = Arc::new(
        PeerConnectionBuilder::new()
            .with_configuration(rtc_config)
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .with_setting_engine(setting_engine)
            .with_handler(handler)
            .with_runtime(runtime)
            .with_udp_addrs(udp_addrs)
            .build()
            .await?,
    );

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
    peer_connection
        .add_track(Arc::clone(&track) as Arc<dyn TrackLocal>)
        .await?;

    let offer_sdp = normalize_remote_offer_sdp(&endpoint_config.offer_sdp);
    let offer: RTCSessionDescription =
        serde_json::from_value(json!({ "type": "offer", "sdp": offer_sdp }))?;
    peer_connection.set_remote_description(offer).await?;
    let answer = peer_connection.create_answer(None).await?;
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
    let answer_sdp = normalize_browser_answer_sdp(&local.sdp);
    eprintln!(
        "[remote-desktop-webrtc] answer_candidate_lines={}",
        answer_sdp
            .lines()
            .filter(|line| line.starts_with("a=candidate:"))
            .count()
    );
    let answer_value = json!({
        "type": "answer",
        "sdp": answer_sdp,
        "transport": TRANSPORT_WEBRTC,
        "endpoint_ura": format!("{DIRECT_WEBRTC_ENDPOINT_PREFIX}{}", endpoint_config.session_id),
        "codec": "h264",
        "carrier": "rtp_srtp",
    });

    let peer_connection_for_endpoint = Arc::clone(&peer_connection);
    let session_id = endpoint_config.session_id;
    let sessions = Arc::clone(&endpoint_config.sessions);
    let transports = Arc::clone(&endpoint_config.transports);
    std::thread::Builder::new()
        .name("easynet-remote-desktop-webrtc".into())
        .spawn(move || {
            transports.block_on(run_direct_webrtc_media_loop(
                Arc::clone(&sessions),
                DirectWebRtcSession {
                    session_id,
                    peer_connection,
                    track,
                    entry: endpoint_config.entry,
                    options: endpoint_config.options,
                    config: media_config,
                },
                connected_rx,
                done_rx,
                endpoint_config.stop_rx,
            ));
        })
        .map_err(|err| anyhow::anyhow!("spawn direct WebRTC media loop: {err}"))?;

    Ok((answer_value, peer_connection_for_endpoint))
}

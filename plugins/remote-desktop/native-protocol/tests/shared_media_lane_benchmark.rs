use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bytes::Bytes;
use easynet_remoteapp_native_protocol::media_session::{
    binary_media_frame_capacity, decode_binary_media_event_frame_compact, generation_nonce_bytes,
    read_event_frame, write_event_frame, BinaryMediaEvent, CaptureBackend, CaptureProof, Command,
    CommandBody, EventBody, EventMetadata, GenerationFence, MediaConversationValidator, MediaLane,
    NativeTargetPlan, StartContract, TargetKind, VideoCodec, VideoConfig, PROTOCOL, SCHEMA_VERSION,
};
use easynet_remoteapp_native_protocol::shared_media_lane::{
    DetachedMediaBufferPool, SharedMediaLaneConsumer, SharedMediaLaneFile, SharedMediaLaneLayout,
    SharedMediaLaneProducer, SharedPublishOutcome, SharedSlotNotification,
    SHARED_SLOT_NOTIFICATION_BYTES,
};
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, IntraFramePeriod,
    Level as OpenH264Level, Profile, RateControlMode, UsageType,
};
use openh264::formats::{RgbSliceU8, YUVBuffer};
use openh264::{OpenH264API, Timestamp};

struct CountingAllocator;

static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Delegates the exact allocation request to the process system allocator.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` and `layout` are the pair returned by this allocator.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: Delegates the valid original pair and requested size to System.
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        if !replacement.is_null() {
            ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        replacement
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Clone, Copy)]
struct AllocationSnapshot {
    calls: u64,
    bytes: u64,
}

fn reset_allocations() {
    ALLOCATION_CALLS.store(0, Ordering::SeqCst);
    ALLOCATED_BYTES.store(0, Ordering::SeqCst);
}

fn allocation_snapshot() -> AllocationSnapshot {
    AllocationSnapshot {
        calls: ALLOCATION_CALLS.load(Ordering::SeqCst),
        bytes: ALLOCATED_BYTES.load(Ordering::SeqCst),
    }
}

fn contract() -> StartContract {
    StartContract {
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
            fps: 60,
            bitrate_kbps: 8_000,
            keyframe_interval_frames: 60,
            max_pending_frames: 1,
            max_access_unit_bytes: 2 * 1024 * 1024,
            max_nal_unit_bytes: 2 * 1024 * 1024,
            h264_profile_idc: 66,
            h264_level_idc: 31,
        },
        audio: None,
    }
}

fn fence(contract: &StartContract) -> GenerationFence {
    GenerationFence {
        process_generation: 7,
        build_id: "33".repeat(32),
        session_nonce: "5a".repeat(16),
        transport_epoch: 3,
        media_source_epoch: 5,
        contract_digest: contract.digest().unwrap(),
    }
}

fn video_body(sequence: u64) -> EventBody {
    EventBody::VideoH264 {
        media_gate: 1,
        pts_90khz: sequence * 1_500,
        duration_90khz: 1_500,
        keyframe: false,
        sps_pps_present: false,
        discontinuity: false,
        codec_generation: 1,
        width: 640,
        height: 360,
        encode_submitted_at_ms: 1_000 + sequence,
        encoded_at_ms: 1_001 + sequence,
    }
}

fn command(sequence: u64, fence: &GenerationFence, body: CommandBody) -> Command {
    Command {
        schema_version: SCHEMA_VERSION,
        protocol: PROTOCOL.into(),
        fence: fence.clone(),
        sequence,
        body,
    }
}

fn event(sequence: u64, fence: &GenerationFence, body: EventBody) -> EventMetadata {
    EventMetadata {
        schema_version: SCHEMA_VERSION,
        protocol: PROTOCOL.into(),
        fence: fence.clone(),
        sequence,
        observed_at_ms: 100 + sequence,
        body,
    }
}

fn real_openh264_idr() -> Vec<u8> {
    let config = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .rate_control_mode(RateControlMode::Bitrate)
        .bitrate(BitRate::from_bps(2_500_000))
        .max_frame_rate(FrameRate::from_hz(30.0))
        .profile(Profile::Baseline)
        .level(OpenH264Level::Level_3_1)
        .complexity(Complexity::Low)
        .intra_frame_period(IntraFramePeriod::from_num_frames(30));
    let mut encoder = Encoder::with_api_config(OpenH264API::from_source(), config).unwrap();
    let rgb = vec![0_u8; 640 * 360 * 3];
    let yuv = YUVBuffer::from_rgb8_source(RgbSliceU8::new(&rgb, (640, 360)));
    encoder
        .encode_at(&yuv, Timestamp::from_millis(0))
        .unwrap()
        .to_vec()
}

fn activated_validator(
    contract: StartContract,
    fence: &GenerationFence,
) -> MediaConversationValidator {
    let mut validator = MediaConversationValidator::new(fence.clone()).unwrap();
    validator
        .register_command(&command(
            1,
            fence,
            CommandBody::StartPrepared {
                contract: contract.clone(),
            },
        ))
        .unwrap();
    validator
        .observe(
            MediaLane::Control,
            &event(
                1,
                fence,
                EventBody::Prepared {
                    command_sequence: 1,
                    capture_proof: CaptureProof {
                        backend: CaptureBackend::ScreenCaptureKit,
                        observed_target: contract.target,
                        native_width: 640,
                        native_height: 360,
                        verified_at_ms: 100,
                    },
                },
            ),
            &[],
        )
        .unwrap();
    validator
        .register_command(&command(2, fence, CommandBody::Activate))
        .unwrap();
    validator
        .observe(
            MediaLane::Control,
            &event(
                2,
                fence,
                EventBody::Activated {
                    command_sequence: 2,
                },
            ),
            &[],
        )
        .unwrap();
    validator
        .register_command(&command(
            3,
            fence,
            CommandBody::BeginMedia {
                activation_command_sequence: 2,
            },
        ))
        .unwrap();
    let idr = real_openh264_idr();
    validator
        .observe_binary_media(
            MediaLane::Video,
            &BinaryMediaEvent {
                sequence: 1,
                observed_at_ms: 1_101,
                body: EventBody::VideoH264 {
                    media_gate: 1,
                    pts_90khz: 1_500,
                    duration_90khz: 1_500,
                    keyframe: true,
                    sps_pps_present: true,
                    discontinuity: true,
                    codec_generation: 1,
                    width: 640,
                    height: 360,
                    encode_submitted_at_ms: 1_001,
                    encoded_at_ms: 1_002,
                },
            },
            &idr,
        )
        .unwrap();
    validator
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    let index = (sorted.len() - 1) * percentile / 100;
    sorted[index]
}

#[test]
fn shared_lane_benchmark_emits_comparative_evidence() {
    const FRAME_COUNT: usize = 128;
    const PAYLOAD_BYTES: usize = 256 * 1024;

    let contract = contract();
    let fence = fence(&contract);
    let generation_nonce = generation_nonce_bytes(&fence).unwrap();
    let mut payload = vec![0x41_u8; PAYLOAD_BYTES];
    payload[..5].copy_from_slice(&[0, 0, 0, 1, 0x41]);
    let frame_capacity = binary_media_frame_capacity(MediaLane::Video, payload.len()).unwrap();
    let layout =
        SharedMediaLaneLayout::new(MediaLane::Video, 1, frame_capacity as u32, generation_nonce)
            .unwrap();
    let lane = SharedMediaLaneFile::create(layout).unwrap();
    let mut producer = SharedMediaLaneProducer::open(
        &lane.try_clone_file().unwrap(),
        MediaLane::Video,
        generation_nonce,
    )
    .unwrap();
    let consumer = SharedMediaLaneConsumer::open(
        &lane.try_clone_file().unwrap(),
        MediaLane::Video,
        generation_nonce,
    )
    .unwrap();
    let mut notification = [0_u8; SHARED_SLOT_NOTIFICATION_BYTES];
    let mut shared_latency_ns = Vec::with_capacity(FRAME_COUNT);
    let mut pipe_latency_ns = Vec::with_capacity(FRAME_COUNT);
    let mut pipe_frame = Vec::with_capacity(frame_capacity);
    let mut retained_transport_bytes: Option<Bytes> = None;
    let transport_pool = DetachedMediaBufferPool::new(2, 2 * PAYLOAD_BYTES).unwrap();
    let mut shared_validator = activated_validator(contract.clone(), &fence);
    let mut pipe_validator = activated_validator(contract, &fence);

    // Fault in the mapped slot and warm both framing paths before recording
    // latency. The measured sequence starts at two because both canonical
    // validators already consumed the recovery IDR at sequence one.
    let warm_body = video_body(1);
    let SharedPublishOutcome::Published(warm_ticket) = producer
        .publish_media_event(1, 1_101, &warm_body, &payload)
        .unwrap()
    else {
        panic!("empty shared lane must accept warm-up frame");
    };
    let warm_lease = consumer.claim(warm_ticket).unwrap();
    let warm_frame = Bytes::from_owner(warm_lease);
    decode_binary_media_event_frame_compact(&warm_frame, MediaLane::Video, generation_nonce)
        .unwrap();
    drop(warm_frame);
    let warm_metadata = EventMetadata {
        schema_version: SCHEMA_VERSION,
        protocol: PROTOCOL.to_string(),
        fence: fence.clone(),
        sequence: 1,
        observed_at_ms: 1_101,
        body: warm_body,
    };
    write_event_frame(&mut pipe_frame, MediaLane::Video, &warm_metadata, &payload).unwrap();
    read_event_frame(
        &mut Cursor::new(&pipe_frame),
        MediaLane::Video,
        Some(&fence),
    )
    .unwrap()
    .unwrap();
    pipe_frame.clear();
    // Two transport buffers cover one retained payload plus the next detach.
    // Their backing allocations are warm-up cost, not per-frame hot-path cost.
    let warm_transport_a = Bytes::from_owner(transport_pool.copy_from_slice(&payload));
    let warm_transport_b = Bytes::from_owner(transport_pool.copy_from_slice(&payload));
    drop(warm_transport_a);
    drop(warm_transport_b);

    reset_allocations();
    let shared_started = Instant::now();
    for sequence in 2..=FRAME_COUNT as u64 + 1 {
        let frame_started = Instant::now();
        let observed_at_ms = 1_100 + sequence;
        let body = video_body(sequence);
        let outcome = producer
            .publish_media_event(sequence, observed_at_ms, &body, &payload)
            .unwrap();
        let SharedPublishOutcome::Published(_) = outcome else {
            panic!("single-slot benchmark released the preceding lease");
        };
        let mut writer = Cursor::new(&mut notification[..]);
        SharedSlotNotification::from(outcome)
            .write_to(&mut writer, MediaLane::Video)
            .unwrap();
        let decoded_notification = SharedSlotNotification::read_from(
            &mut Cursor::new(&notification[..]),
            MediaLane::Video,
        )
        .unwrap()
        .unwrap();
        let SharedSlotNotification::Published(ticket) = decoded_notification else {
            panic!("published shared frame must retain a ticket");
        };
        let lease = consumer.claim(ticket).unwrap();
        let mapped_pointer = lease.as_ref().as_ptr();
        let frame = Bytes::from_owner(lease);
        assert_eq!(frame.as_ptr(), mapped_pointer);
        let (decoded, payload_view) =
            decode_binary_media_event_frame_compact(&frame, MediaLane::Video, generation_nonce)
                .unwrap();
        assert_eq!(decoded.sequence, sequence);
        shared_validator
            .observe_binary_media(MediaLane::Video, &decoded, payload_view)
            .unwrap();
        let mapped_payload_pointer = payload_view.as_ptr();
        let webrtc_bytes = Bytes::from_owner(transport_pool.copy_from_slice(payload_view));
        assert_ne!(webrtc_bytes.as_ptr(), mapped_payload_pointer);
        assert_eq!(webrtc_bytes.len(), PAYLOAD_BYTES);
        drop(frame);
        // Keep transport bytes alive across the next publish. The shared slot
        // must already be reusable because RTP/NACK ownership is detached from
        // the mapping lease.
        retained_transport_bytes = Some(webrtc_bytes);
        shared_latency_ns.push(frame_started.elapsed().as_nanos() as u64);
    }
    drop(retained_transport_bytes);
    let shared_elapsed = shared_started.elapsed();
    let shared_allocations = allocation_snapshot();

    reset_allocations();
    let pipe_started = Instant::now();
    for sequence in 2..=FRAME_COUNT as u64 + 1 {
        let frame_started = Instant::now();
        let observed_at_ms = 1_100 + sequence;
        let body = video_body(sequence);
        let metadata = EventMetadata {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            fence: fence.clone(),
            sequence,
            observed_at_ms,
            body: body.clone(),
        };
        pipe_frame.clear();
        write_event_frame(&mut pipe_frame, MediaLane::Video, &metadata, &payload).unwrap();
        let (decoded, decoded_payload) = read_event_frame(
            &mut Cursor::new(&pipe_frame),
            MediaLane::Video,
            Some(&fence),
        )
        .unwrap()
        .unwrap();
        assert_eq!(decoded.sequence, sequence);
        assert_eq!(decoded_payload.len(), PAYLOAD_BYTES);
        pipe_validator
            .observe(MediaLane::Video, &decoded, &decoded_payload)
            .unwrap();
        pipe_latency_ns.push(frame_started.elapsed().as_nanos() as u64);
    }
    let pipe_elapsed = pipe_started.elapsed();
    let pipe_allocations = allocation_snapshot();

    shared_latency_ns.sort_unstable();
    pipe_latency_ns.sort_unstable();
    assert!(
        shared_allocations.calls <= (FRAME_COUNT * 2) as u64,
        "validated shared lane must need at most one lease owner and one detach allocation per frame"
    );
    assert!(
        shared_allocations.bytes <= (FRAME_COUNT * (PAYLOAD_BYTES + 256)) as u64,
        "shared lane allocation volume must remain one bounded payload plus lease ownership per frame"
    );

    let total_payload_bytes = (FRAME_COUNT * PAYLOAD_BYTES) as f64;
    let shared_mib_per_s = total_payload_bytes / (1024.0 * 1024.0) / shared_elapsed.as_secs_f64();
    let pipe_mib_per_s = total_payload_bytes / (1024.0 * 1024.0) / pipe_elapsed.as_secs_f64();
    println!(
        "REMOTEAPP_SHARED_LANE_BENCHMARK_JSON={}",
        serde_json::json!({
            "schema": "remoteapp_shared_media_lane_benchmark_v2",
            "hot_path": "shared_slot_decode_validate_pooled_transport_detach",
            "frame_count": FRAME_COUNT,
            "payload_bytes_per_frame": PAYLOAD_BYTES,
            "shared_v2": {
                "allocation_calls": shared_allocations.calls,
                "allocated_bytes": shared_allocations.bytes,
                "elapsed_ns": shared_elapsed.as_nanos(),
                "throughput_mib_per_s": shared_mib_per_s,
                "latency_ns": {
                    "p50": percentile(&shared_latency_ns, 50),
                    "p95": percentile(&shared_latency_ns, 95),
                    "p99": percentile(&shared_latency_ns, 99),
                }
            },
            "payload_pipe_v1": {
                "allocation_calls": pipe_allocations.calls,
                "allocated_bytes": pipe_allocations.bytes,
                "elapsed_ns": pipe_elapsed.as_nanos(),
                "throughput_mib_per_s": pipe_mib_per_s,
                "latency_ns": {
                    "p50": percentile(&pipe_latency_ns, 50),
                    "p95": percentile(&pipe_latency_ns, 95),
                    "p99": percentile(&pipe_latency_ns, 99),
                }
            }
        })
    );
}

// EasyNet CLI — mic.subscribe real handler (RFC-005 v3.2 A1)
// ============================================================
//
// File: src/daemon/ability/builtins/resources/media/mic_subscribe.rs
//
// Provider-backed server-stream slice. Binds the metadata-only
// `mic.subscribe` capability contract to a real envelope-aware handler that:
//
//   1. Reads `EnvelopeContext.subject` (per INV-SUBJECT-ENVELOPE)
//   2. Resolves subject → `ResourceEntry`, rejects mismatched
//      type / unknown URA per INV-RESOURCE-VALIDITY
//   3. Opens the cpal default-input stream on a dedicated worker
//      thread (cpal's `Stream` is `!Send` so it must own its
//      thread) and forwards PCM samples into a `tokio::sync::
//      broadcast` channel as `BinaryChunk`-shaped JSON frames
//   4. Returns `StreamSource::Live(rx)` immediately; the worker
//      thread runs until the receiver is dropped (subscriber
//      unsubscribed) or the stream errors
//
// Codec
// -----
// v1 emits S16LE PCM at the device's native sample rate. The
// `args.codec` schema permits `"opus"` per the original RFC, but
// libopus is a C dep we deliberately avoid in v1 — consumers
// either wrap an Opus encoder themselves or accept the raw
// `audio/L16; rate=<hz>; channels=<n>` content type. The frame
// envelope advertises the actual content_type so a consumer that
// asked for opus and got L16 sees the mismatch and can react.
//
// Per **INV-MAC-CHAIN-TRANSMITTED** (plan v3.2): on cpal xrun /
// underrun the worker drops the affected callback's samples and
// continues; we never pad or fabricate to fill PTS gaps. The
// per-direction MAC chain wraps only what was transmitted; gaps
// in PTS are visible to the consumer (jitter buffer concern).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

#[cfg(feature = "native-media")]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::daemon::ability::builtins::resources::media::resource_subject::{
    self, resolve_required_resource_subject, ResourceSubjectSpec,
};
use crate::daemon::ability::builtins::resources::media::{self, ABILITY_MIC_SUBSCRIBE};
use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, EnvelopeContext, StreamSource};
use crate::daemon::persistence::resources::{ResourceEntry, ResourceType};

pub const REASON_SUBJECT_REQUIRED: &str = resource_subject::REASON_SUBJECT_REQUIRED;
pub const REASON_SUBJECT_IN_ARGS: &str = resource_subject::REASON_SUBJECT_IN_ARGS;
pub const REASON_RESOURCE_NOT_FOUND: &str = resource_subject::REASON_RESOURCE_NOT_FOUND;
pub const REASON_RESOURCE_TABLE_UNAVAILABLE: &str =
    resource_subject::REASON_RESOURCE_TABLE_UNAVAILABLE;
pub const REASON_RESOURCE_TYPE_MISMATCH: &str = resource_subject::REASON_RESOURCE_TYPE_MISMATCH;
pub const REASON_RESOURCE_UNAVAILABLE: &str = "resource_unavailable";

/// Broadcast channel depth. 256 frames at ~20ms per frame ≈ 5s
/// of jitter buffer; matches `BIDI_CHANNEL_BOUND` so a single
/// slow consumer can't cause unbounded memory growth.
const BROADCAST_CAPACITY: usize = 256;

/// Backend trait. Real path = `CpalMicBackend`; tests use
/// `SyntheticMicBackend` which emits one synthetic frame and
/// terminates so the suite runs hardware-free.
pub trait MicBackend: Send + Sync {
    /// Open the mic and return a broadcast receiver of PCM frames.
    /// Each frame is a JSON object shaped `{ seq, content_type,
    /// sample_rate, channels, samples_b64 }`. The backend owns
    /// any worker thread it spawns; closing the broadcast tx
    /// (returned via the channel's lifetime) ends the worker.
    fn open(&self, entry: &ResourceEntry) -> anyhow::Result<broadcast::Receiver<Value>>;
}

// ── CpalMicBackend (real, PR3) ───────────────────────────────

/// Real cpal-backed input. Spawns a dedicated thread that owns
/// the cpal `Stream` (which is `!Send`), forwards every input
/// callback's PCM samples as a base64-encoded `BinaryChunk`-
/// shaped frame on the broadcast channel, and ends when the
/// channel's last receiver drops.
#[derive(Debug, Default)]
#[cfg(feature = "native-media")]
pub struct CpalMicBackend;

#[cfg(feature = "native-media")]
impl MicBackend for CpalMicBackend {
    fn open(&self, entry: &ResourceEntry) -> anyhow::Result<broadcast::Receiver<Value>> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = host.default_input_device().ok_or_else(|| {
            anyhow::anyhow!(
                "{ABILITY_MIC_SUBSCRIBE}: no default input device on host \
                 ({:?}); reason={REASON_RESOURCE_UNAVAILABLE}",
                host.id()
            )
        })?;
        let config = device.default_input_config().map_err(|e| {
            anyhow::anyhow!(
                "{ABILITY_MIC_SUBSCRIBE}: default_input_config failed: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let sample_format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();

        let (tx, rx) = broadcast::channel::<Value>(BROADCAST_CAPACITY);
        let hardware_id = entry.hardware_id.clone();

        // Per INV-MAC-CHAIN-TRANSMITTED: we never pad gaps. A cpal
        // xrun shows up as a missing frame, full stop. The seq
        // counter monotonically tags each transmitted frame so a
        // consumer can detect the gap from PTS arithmetic
        // (sample_rate × samples-per-frame).
        let seq = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        // cpal `Stream` is `!Send`; own it on a dedicated thread.
        // The worker is an explicit two-state loop:
        //
        //   Running
        //      -- no live broadcast receivers, or callback observes
        //         send failure -->
        //   Stopped: drop the Stream and close the channel
        //
        // This keeps the hardware stream lifetime tied to the
        // subscription. A permanent park would leave the stream open
        // after the recording consumer disconnects.
        std::thread::Builder::new()
            .name("easynet-mic-cpal".into())
            .spawn(move || {
                let params = MicInputStreamParams {
                    tx: tx.clone(),
                    seq: Arc::clone(&seq),
                    hardware_id: hardware_id.clone(),
                    sample_rate,
                    channels,
                    stop: Arc::clone(&stop),
                };
                let stream_result = match sample_format {
                    cpal::SampleFormat::F32 => {
                        build_input_stream::<f32>(&device, &stream_config, params)
                    }
                    cpal::SampleFormat::I16 => {
                        build_input_stream::<i16>(&device, &stream_config, params)
                    }
                    cpal::SampleFormat::U16 => {
                        build_input_stream::<u16>(&device, &stream_config, params)
                    }
                    other => {
                        eprintln!("{ABILITY_MIC_SUBSCRIBE}: unsupported sample format {other:?}");
                        return;
                    }
                };
                let stream = match stream_result {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("{ABILITY_MIC_SUBSCRIBE}: build_input_stream failed: {e}");
                        return;
                    }
                };
                if let Err(e) = stream.play() {
                    eprintln!("{ABILITY_MIC_SUBSCRIBE}: stream.play() failed: {e}");
                    return;
                }
                while tx.receiver_count() > 0 && !stop.load(Ordering::Relaxed) {
                    std::thread::park_timeout(Duration::from_millis(250));
                }
                drop(stream);
            })
            .map_err(|e| {
                anyhow::anyhow!(
                    "{ABILITY_MIC_SUBSCRIBE}: failed to spawn mic worker: {e}; \
                     reason={REASON_RESOURCE_UNAVAILABLE}"
                )
            })?;

        Ok(rx)
    }
}

#[cfg(feature = "native-media")]
struct MicInputStreamParams {
    tx: broadcast::Sender<Value>,
    seq: Arc<AtomicU64>,
    hardware_id: String,
    sample_rate: u32,
    channels: u16,
    stop: Arc<AtomicBool>,
}

/// Build a cpal input stream for sample type `T`. Each callback
/// converts the buffer to S16LE PCM and broadcasts a JSON frame.
/// `Send`-bound on `T` so the closure can be moved into cpal.
#[cfg(feature = "native-media")]
fn build_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    params: MicInputStreamParams,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + ToS16Pcm + 'static,
{
    use cpal::traits::DeviceTrait;
    let MicInputStreamParams {
        tx,
        seq,
        hardware_id,
        sample_rate,
        channels,
        stop,
    } = params;
    let err_cb = |e: cpal::StreamError| {
        eprintln!("{ABILITY_MIC_SUBSCRIBE}: cpal stream error: {e}");
    };
    device.build_input_stream(
        config,
        move |data: &[T], _info: &cpal::InputCallbackInfo| {
            // Convert to S16LE little-endian byte buffer.
            let mut bytes = Vec::with_capacity(data.len() * 2);
            for sample in data {
                let s = sample.to_s16();
                bytes.push((s & 0xff) as u8);
                bytes.push(((s >> 8) & 0xff) as u8);
            }
            let frame = build_frame(&seq, sample_rate, channels, &hardware_id, &bytes);
            // Broadcast send returns Err only when there are no
            // receivers. That is the stop transition for this
            // one-subscription worker.
            if tx.send(frame).is_err() {
                stop.store(true, Ordering::Relaxed);
            }
        },
        err_cb,
        None,
    )
}

fn build_frame(
    seq: &AtomicU64,
    sample_rate: u32,
    channels: u16,
    hardware_id: &str,
    pcm_bytes: &[u8],
) -> Value {
    let n = seq.fetch_add(1, Ordering::Relaxed);
    let samples_b64 = BASE64_STANDARD.encode(pcm_bytes);
    json!({
        "seq":          n,
        "content_type": format!("audio/L16; rate={sample_rate}; channels={channels}"),
        "sample_rate":  sample_rate,
        "channels":     channels,
        "byte_size":    pcm_bytes.len(),
        "samples_b64":  samples_b64,
        "hardware_id":  hardware_id,
    })
}

/// PCM sample → S16LE conversion helper. Implemented for the
/// three cpal default sample formats we route in
/// `CpalMicBackend::open`.
#[cfg(feature = "native-media")]
trait ToS16Pcm {
    fn to_s16(&self) -> i16;
}

#[cfg(feature = "native-media")]
impl ToS16Pcm for f32 {
    fn to_s16(&self) -> i16 {
        let v = (*self * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32);
        v as i16
    }
}

#[cfg(feature = "native-media")]
impl ToS16Pcm for i16 {
    fn to_s16(&self) -> i16 {
        *self
    }
}

#[cfg(feature = "native-media")]
impl ToS16Pcm for u16 {
    fn to_s16(&self) -> i16 {
        // Map [0, 65535] → [-32768, 32767]
        (*self as i32 - 32768) as i16
    }
}

// ── SyntheticMicBackend (test-only) ──────────────────────────

/// Hardware-free backend that emits exactly one frame of silent
/// PCM (480 zero-samples = 10ms at 48 kHz mono) and ends. Lets
/// tests assert the dispatch path without spawning cpal threads.
#[derive(Debug, Default)]
pub struct SyntheticMicBackend;

impl MicBackend for SyntheticMicBackend {
    fn open(&self, entry: &ResourceEntry) -> anyhow::Result<broadcast::Receiver<Value>> {
        let (tx, rx) = broadcast::channel::<Value>(8);
        let seq = Arc::new(AtomicU64::new(0));
        let bytes = vec![0u8; 960]; // 480 i16 samples
        let frame = build_frame(&seq, 48000, 1, &entry.hardware_id, &bytes);
        let _ = tx.send(frame);
        Ok(rx)
    }
}

// ── Registration ─────────────────────────────────────────────

pub fn register_with_backend(reg: &mut AxonAbilityCatalog, backend: Arc<dyn MicBackend>) {
    reg.register_stream_with_envelope_and_spec(
        ABILITY_MIC_SUBSCRIBE,
        OwnerKind::Device,
        media::registry_manifest(ABILITY_MIC_SUBSCRIBE),
        Arc::new(move |env: EnvelopeContext, args: Value| handler(&backend, env, args)),
    );
}

pub fn register(reg: &mut AxonAbilityCatalog) {
    #[cfg(feature = "native-media")]
    register_with_backend(reg, Arc::new(CpalMicBackend));
    #[cfg(all(not(feature = "native-media"), feature = "headless-media"))]
    register_with_backend(reg, Arc::new(SyntheticMicBackend));
}

// ── Handler core ─────────────────────────────────────────────

fn handler(
    backend: &Arc<dyn MicBackend>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<StreamSource> {
    let entry = resolve_required_resource_subject(
        &env,
        &args,
        ResourceSubjectSpec {
            ability: ABILITY_MIC_SUBSCRIBE,
            required_subject: "a mic",
            allowed_kinds: &[ResourceType::Mic],
            allowed_label: "mic",
        },
    )?;
    let rx = backend.open(&entry)?;
    Ok(StreamSource::Live(tee_recording(
        rx,
        env.callee().to_string(),
    )?))
}

// ── Context-surface recording tee ────────────────────────────
//
// The tee owns one relay receiver-facing stream and one upstream
// receiver from the backend. It polls upstream with a bounded sleep
// so a downstream disconnect finalizes the WAV even when the mic is
// quiet and no later audio callback arrives.

/// Stop accumulating (but keep relaying) past this many PCM bytes —
/// 10 minutes of 48 kHz mono S16LE. Bounds disk + memory for a
/// subscriber that listens forever.
const RECORDING_MAX_PCM_BYTES: usize = 48_000 * 2 * 60 * 10;

fn tee_recording(
    mut upstream: broadcast::Receiver<Value>,
    device_ura: String,
) -> anyhow::Result<broadcast::Receiver<Value>> {
    let (relay_tx, relay_rx) = broadcast::channel::<Value>(BROADCAST_CAPACITY);
    std::thread::Builder::new()
        .name("easynet-mic-tee".into())
        .spawn(move || {
            let mut recording = MicRecordingAccumulator::default();
            loop {
                if relay_tx.receiver_count() == 0 {
                    break;
                }
                match upstream.try_recv() {
                    Ok(frame) => {
                        recording.observe(&frame);
                        if relay_tx.send(frame).is_err() {
                            break; // consumer dropped the stream
                        }
                    }
                    Err(broadcast::error::TryRecvError::Empty) => {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                    Err(broadcast::error::TryRecvError::Closed) => break,
                }
            }
            recording.finish(&device_ura);
        })
        .map_err(|e| {
            anyhow::anyhow!(
                "{ABILITY_MIC_SUBSCRIBE}: failed to spawn recording tee: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
    Ok(relay_rx)
}

struct MicRecordingAccumulator {
    pcm: Vec<u8>,
    sample_rate: u32,
    channels: u16,
}

impl Default for MicRecordingAccumulator {
    fn default() -> Self {
        Self {
            pcm: Vec::new(),
            sample_rate: 48_000,
            channels: 1,
        }
    }
}

impl MicRecordingAccumulator {
    fn observe(&mut self, frame: &Value) {
        if let Some(rate) = frame.get("sample_rate").and_then(Value::as_u64) {
            self.sample_rate = rate as u32;
        }
        if let Some(ch) = frame.get("channels").and_then(Value::as_u64) {
            self.channels = ch as u16;
        }
        let remaining = RECORDING_MAX_PCM_BYTES.saturating_sub(self.pcm.len());
        if remaining == 0 {
            return;
        }
        let Some(b64) = frame.get("samples_b64").and_then(Value::as_str) else {
            return;
        };
        let Ok(bytes) = BASE64_STANDARD.decode(b64) else {
            return;
        };
        self.pcm
            .extend_from_slice(&bytes[..bytes.len().min(remaining)]);
    }

    fn finish(self, device_ura: &str) {
        finalize_recording(device_ura, &self.pcm, self.sample_rate, self.channels);
    }
}

fn finalize_recording(device_ura: &str, pcm: &[u8], sample_rate: u32, channels: u16) {
    if pcm.is_empty() {
        return;
    }
    let wav = wav_from_s16le(pcm, sample_rate, channels);
    let bytes_per_second = u64::from(sample_rate) * u64::from(channels) * 2;
    let duration_ms = (pcm.len() as u64).saturating_mul(1000) / bytes_per_second.max(1);
    if let Err(err) = crate::daemon::persistence::context_store::record_capture(
        crate::daemon::persistence::context_store::CaptureRecord {
            device: device_ura,
            ability: ABILITY_MIC_SUBSCRIBE,
            ext: "wav",
            bytes: &wav,
            content_type: "audio/wav",
            width: None,
            height: None,
            duration_ms: Some(duration_ms),
            preview: format!("Recording {:.1}s", duration_ms as f64 / 1000.0),
        },
    ) {
        crate::op_event!(
            component = context,
            kind = capture_persist_failed,
            level = "warn",
            ability = ABILITY_MIC_SUBSCRIBE,
            error = err,
        );
    }
}

/// Minimal RIFF/WAVE wrapper around raw S16LE PCM. 44-byte canonical
/// header; no dependency needed for fixed-format PCM.
fn wav_from_s16le(pcm: &[u8], sample_rate: u32, channels: u16) -> Vec<u8> {
    let byte_rate = sample_rate * u32::from(channels) * 2;
    let block_align = channels * 2;
    let data_len = pcm.len() as u32;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::invocation::routing::target::CallMode;
    use crate::daemon::persistence::resources::{
        self, upsert_resource, ResourceBinding, ResourceUpsert, ResourcesFile,
    };

    fn seed_mic(file: &mut ResourcesFile, hardware_id: &str) -> String {
        upsert_resource(
            file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/device/01DEV",
                kind: ResourceType::Mic,
                binding: ResourceBinding::LocalDevice,
                hardware_id,
                display_name: "Test Mic",
                metadata: json!({}),
            },
        )
        .expect("seed mic resource")
    }

    fn register_synthetic(reg: &mut AxonAbilityCatalog) {
        register_with_backend(reg, Arc::new(SyntheticMicBackend));
    }

    const TEST_DEVICE_URA: &str = "easynet:///r/test/device/mic-subscribe";

    fn metadata_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(TEST_DEVICE_URA)
    }

    fn runtime_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_runtime_for_device_authority(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            TEST_DEVICE_URA,
        )
    }

    #[test]
    fn registration_publishes_mic_descriptor_to_catalog_snapshot() {
        let mut reg = metadata_test_catalog();
        register_synthetic(&mut reg);
        let rows = reg.authority_ability_catalog_snapshot();
        let descriptor = rows
            .iter()
            .find(|row| row.name == ABILITY_MIC_SUBSCRIBE)
            .map(|row| &row.descriptor)
            .expect("mic.subscribe must publish canonical descriptor");

        assert_eq!(
            descriptor.description,
            media::description(ABILITY_MIC_SUBSCRIBE).expect("mic description")
        );
        assert_eq!(
            descriptor.input_schema(),
            &media::input_schema(ABILITY_MIC_SUBSCRIBE).expect("mic schema")
        );
    }

    #[test]
    fn handler_returns_live_stream_with_pcm_frame_when_subject_resolves() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_mic(&mut file, "h-mic-e2e");
        resources::save(&file).unwrap();
        let mut reg = runtime_test_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_MIC_SUBSCRIBE,
            json!({"sample_rate": 48000, "channels": 1, "codec": "opus"}),
            CallMode::Stream,
            ura,
        );
        let src = dispatcher.execute_stream(target).unwrap();
        let frame = match src {
            StreamSource::Snapshot(mut frames) => {
                assert_eq!(frames.len(), 1);
                frames.remove(0)
            }
            StreamSource::Live(mut rx) => rx
                .try_recv()
                .expect("synthetic backend must publish one frame"),
            StreamSource::SnapshotThenLive(mut frames, _) => {
                assert_eq!(frames.len(), 1);
                frames.remove(0)
            }
        };
        assert!(
            frame.get("samples_b64").is_some(),
            "frame missing samples_b64: {frame}"
        );
        assert_eq!(frame["channels"], 1);
        assert_eq!(frame["sample_rate"], 48000);
        assert!(frame["content_type"]
            .as_str()
            .unwrap()
            .contains("audio/L16"));
        assert_eq!(frame["hardware_id"], "h-mic-e2e");
    }

    #[test]
    fn handler_rejects_non_resource_subject() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let backend: Arc<dyn MicBackend> = Arc::new(SyntheticMicBackend);
        let err = handler(
            &backend,
            EnvelopeContext::for_test(
                "easynet:///r/acme/user/alice",
                "easynet:///r/acme/user/alice",
            ),
            json!({}),
        )
        .unwrap_err();
        assert!(err.to_string().contains(REASON_SUBJECT_REQUIRED));
    }

    #[test]
    fn handler_rejects_camera_subject_with_resource_type_mismatch() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let cam_ura = upsert_resource(
            &mut file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/device/01DEV",
                kind: ResourceType::Camera, // wrong type
                binding: ResourceBinding::LocalDevice,
                hardware_id: "h-cam-not-mic",
                display_name: "Not A Mic",
                metadata: json!({}),
            },
        )
        .expect("seed wrong-type camera resource");
        resources::save(&file).unwrap();
        let mut reg = runtime_test_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_MIC_SUBSCRIBE,
            json!({}),
            CallMode::Stream,
            cam_ura,
        );
        let err = dispatcher.execute_stream(target).unwrap_err();
        assert!(err.to_string().contains(REASON_RESOURCE_TYPE_MISMATCH));
    }

    #[test]
    fn handler_rejects_subject_in_args() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = runtime_test_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_MIC_SUBSCRIBE,
            json!({"subject": "easynet:///r/x/resource/y"}),
            CallMode::Stream,
            "easynet:///r/acme/resource/01MIC",
        );
        let err = dispatcher.execute_stream(target).unwrap_err();
        assert!(err.to_string().contains(REASON_SUBJECT_IN_ARGS));
    }

    #[test]
    fn handler_reports_corrupt_resources_table_as_table_unavailable() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let path = resources::path();
        std::fs::create_dir_all(path.parent().expect("resources path has parent"))
            .expect("create state dir");
        std::fs::write(&path, b"{not-json").expect("write corrupt resources table");

        let mut reg = runtime_test_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_MIC_SUBSCRIBE,
            json!({}),
            CallMode::Stream,
            "easynet:///r/acme/resource/01MIC",
        );

        let err = dispatcher.execute_stream(target).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains(REASON_RESOURCE_TABLE_UNAVAILABLE),
            "expected reason={REASON_RESOURCE_TABLE_UNAVAILABLE}; got: {message}"
        );
        assert!(
            !message.contains(REASON_RESOURCE_NOT_FOUND),
            "corrupt table must not be misreported as a missing resource: {message}"
        );
    }

    #[test]
    fn synthetic_backend_emits_one_frame_per_open() {
        let mut file = ResourcesFile::default();
        seed_mic(&mut file, "h-mic-syn");
        let entry = file.resources[0].clone();
        let mut rx = SyntheticMicBackend.open(&entry).unwrap();
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame["seq"], 0);
        assert_eq!(frame["byte_size"], 960);
    }

    #[test]
    fn recording_tee_finalizes_when_consumer_drops_without_next_frame() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let (tx, upstream_rx) = broadcast::channel::<Value>(8);
        let seq = AtomicU64::new(0);
        let frame = build_frame(&seq, 48_000, 1, "h-mic-recording", &[0u8; 960]);
        tx.send(frame).unwrap();

        let mut relay =
            tee_recording(upstream_rx, "easynet:///r/acme/device/01DEV".to_string()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            match relay.try_recv() {
                Ok(frame) => {
                    assert_eq!(frame["hardware_id"], "h-mic-recording");
                    break;
                }
                Err(broadcast::error::TryRecvError::Empty)
                    if std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("recording tee did not relay first frame: {err}"),
            }
        }
        drop(relay);

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let captures = crate::daemon::persistence::context_store::list_captures(
                Some(ABILITY_MIC_SUBSCRIBE),
                10,
            )
            .unwrap();
            if let Some(capture) = captures.first() {
                assert_eq!(capture.content_type, "audio/wav");
                assert_eq!(capture.duration_ms, Some(10));
                return;
            }
            if std::time::Instant::now() >= deadline {
                panic!("recording tee did not finalize after consumer drop");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(feature = "native-media")]
    #[test]
    fn s16_conversions_are_correct() {
        assert_eq!(0.0f32.to_s16(), 0);
        assert_eq!(1.0f32.to_s16(), i16::MAX);
        // Symmetric scaling around i16::MAX gives -32767 (one above
        // i16::MIN), the standard "audio samples don't lie about
        // peak DC" choice — losing 1 LSB of headroom on the
        // negative rail beats clipping a full-scale negative
        // sample to a different value than a full-scale positive
        // would scale to.
        assert_eq!((-1.0f32).to_s16(), -i16::MAX);
        assert_eq!(0i16.to_s16(), 0);
        assert_eq!(32768u16.to_s16(), 0);
        assert_eq!(0u16.to_s16(), -32768);
    }
}

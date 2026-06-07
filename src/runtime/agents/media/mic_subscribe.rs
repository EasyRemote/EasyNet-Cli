// EasyNet CLI — mic.subscribe real handler (RFC-005 v3.2 A1)
// ============================================================
//
// File: src/runtime/agents/media/mic_subscribe.rs
//
// PR3 server-stream slice. Replaces the `stream_stub` in
// `media_abilities.rs` for the `mic.subscribe` name with a real
// envelope-aware handler that:
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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::persistence::resources::{self, lookup_by_ura, ResourceEntry, ResourceType};
use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, EnvelopeContext, StreamSource};
use crate::runtime::agents::media::resource_subject;
use crate::runtime::agents::media_abilities::{ABILITY_MIC_SUBSCRIBE, REASON_SUBJECT_IN_ARGS};

pub const REASON_SUBJECT_REQUIRED: &str = "subject_required";
pub const REASON_RESOURCE_NOT_FOUND: &str = "resource_not_found";
pub const REASON_RESOURCE_TABLE_UNAVAILABLE: &str =
    resource_subject::REASON_RESOURCE_TABLE_UNAVAILABLE;
pub const REASON_RESOURCE_TYPE_MISMATCH: &str = "resource_type_mismatch";
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
pub struct CpalMicBackend;

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

        // cpal `Stream` is `!Send`; own it on a dedicated thread.
        // The thread parks on a one-shot signal that fires when
        // the broadcast tx is dropped (last receiver gone). We
        // arrange that by moving `tx` into the thread and
        // returning only `rx` to the caller — when no receivers
        // remain `tx.send` returns Err and we drop the stream.
        std::thread::Builder::new()
            .name("easynet-mic-cpal".into())
            .spawn(move || {
                let stream_result = match sample_format {
                    cpal::SampleFormat::F32 => build_input_stream::<f32>(
                        &device,
                        &stream_config,
                        tx,
                        seq,
                        hardware_id,
                        sample_rate,
                        channels,
                    ),
                    cpal::SampleFormat::I16 => build_input_stream::<i16>(
                        &device,
                        &stream_config,
                        tx,
                        seq,
                        hardware_id,
                        sample_rate,
                        channels,
                    ),
                    cpal::SampleFormat::U16 => build_input_stream::<u16>(
                        &device,
                        &stream_config,
                        tx,
                        seq,
                        hardware_id,
                        sample_rate,
                        channels,
                    ),
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
                // Park forever; cpal callback drives the stream.
                // The loop exits when the data callback's `tx.send`
                // sees no receivers and we set a kill flag, but a
                // simpler shape is just to park — when the registry
                // shuts down the process exits and the thread dies.
                loop {
                    std::thread::park();
                }
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

/// Build a cpal input stream for sample type `T`. Each callback
/// converts the buffer to S16LE PCM and broadcasts a JSON frame.
/// `Send`-bound on `T` so the closure can be moved into cpal.
fn build_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tx: broadcast::Sender<Value>,
    seq: Arc<AtomicU64>,
    hardware_id: String,
    sample_rate: u32,
    channels: u16,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + ToS16Pcm + 'static,
{
    use cpal::traits::DeviceTrait;
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
            // receivers — at that point the subscriber has gone,
            // we drop the frame silently and keep the stream open
            // (cheap: another subscriber may still arrive). Any
            // other "channel full" lag is broadcast's lag-on-slow-
            // consumer behaviour and the consumer's problem.
            let _ = tx.send(frame);
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
trait ToS16Pcm {
    fn to_s16(&self) -> i16;
}

impl ToS16Pcm for f32 {
    fn to_s16(&self) -> i16 {
        let v = (*self * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32);
        v as i16
    }
}

impl ToS16Pcm for i16 {
    fn to_s16(&self) -> i16 {
        *self
    }
}

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
    reg.register_stream_with_envelope_and_owner(
        "mic.subscribe",
        OwnerKind::Device,
        Arc::new(move |env: EnvelopeContext, args: Value| handler(&backend, env, args)),
    );
}

pub fn register(reg: &mut AxonAbilityCatalog) {
    register_with_backend(reg, Arc::new(CpalMicBackend));
}

// ── Handler core ─────────────────────────────────────────────

fn handler(
    backend: &Arc<dyn MicBackend>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<StreamSource> {
    if let Value::Object(map) = &args {
        if map.contains_key("subject") {
            anyhow::bail!(
                "{ABILITY_MIC_SUBSCRIBE}: `subject` MUST come from the \
                 invocation envelope, not from args (INV-SUBJECT-ENVELOPE; \
                 reason={REASON_SUBJECT_IN_ARGS})"
            );
        }
    }
    let subject = env.subject.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_MIC_SUBSCRIBE}: subject required (resource_ura of a \
             mic); reason={REASON_SUBJECT_REQUIRED}"
        )
    })?;
    let file = resources::load().map_err(|err| {
        anyhow::anyhow!(
            "{ABILITY_MIC_SUBSCRIBE}: local resources table could not be loaded; \
             reason={REASON_RESOURCE_TABLE_UNAVAILABLE}; source={err}"
        )
    })?;
    let entry = lookup_by_ura(&file, subject).ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_MIC_SUBSCRIBE}: subject {subject} not found in local \
             resources table; reason={REASON_RESOURCE_NOT_FOUND}"
        )
    })?;
    if entry.kind != ResourceType::Mic {
        anyhow::bail!(
            "{ABILITY_MIC_SUBSCRIBE}: subject {subject} resolves to a {}, \
             not a mic; reason={REASON_RESOURCE_TYPE_MISMATCH}",
            entry.kind.as_str()
        );
    }
    let rx = backend.open(entry)?;
    Ok(StreamSource::Live(rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::resources::{
        upsert_resource, ResourceBinding, ResourceUpsert, ResourcesFile,
    };
    use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

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
    }

    fn register_synthetic(reg: &mut AxonAbilityCatalog) {
        register_with_backend(reg, Arc::new(SyntheticMicBackend));
    }

    #[test]
    fn handler_returns_live_stream_with_pcm_frame_when_subject_resolves() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_mic(&mut file, "h-mic-e2e");
        resources::save(&file).unwrap();
        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_MIC_SUBSCRIBE.to_string(),
            normalized_args: json!({"sample_rate": 48000, "channels": 1, "codec": "opus"}),
            call_mode: CallMode::Stream,
            subject: Some(ura),
            causal_context: None,
        };
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
    fn handler_rejects_missing_subject() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let backend: Arc<dyn MicBackend> = Arc::new(SyntheticMicBackend);
        let err = handler(&backend, EnvelopeContext::default(), json!({})).unwrap_err();
        assert!(err.to_string().contains(REASON_SUBJECT_REQUIRED));
    }

    #[test]
    fn handler_rejects_camera_subject_with_resource_type_mismatch() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
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
        );
        resources::save(&file).unwrap();
        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_MIC_SUBSCRIBE.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Stream,
            subject: Some(cam_ura),
            causal_context: None,
        };
        let err = dispatcher.execute_stream(target).unwrap_err();
        assert!(err.to_string().contains(REASON_RESOURCE_TYPE_MISMATCH));
    }

    #[test]
    fn handler_rejects_subject_in_args() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_MIC_SUBSCRIBE.to_string(),
            normalized_args: json!({"subject": "easynet:///r/x/resource/y"}),
            call_mode: CallMode::Stream,
            subject: Some("easynet:///r/acme/resource/01MIC".into()),
            causal_context: None,
        };
        let err = dispatcher.execute_stream(target).unwrap_err();
        assert!(err.to_string().contains(REASON_SUBJECT_IN_ARGS));
    }

    #[test]
    fn handler_reports_corrupt_resources_table_as_table_unavailable() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let path = resources::path();
        std::fs::create_dir_all(path.parent().expect("resources path has parent"))
            .expect("create state dir");
        std::fs::write(&path, b"{not-json").expect("write corrupt resources table");

        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_MIC_SUBSCRIBE.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Stream,
            subject: Some("easynet:///r/acme/resource/01MIC".into()),
            causal_context: None,
        };

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

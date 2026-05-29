// EasyNet CLI — camera.snapshot real handler (RFC-005 v3.2 A3)
// ==============================================================
//
// File: src/runtime/agents/media/camera_snapshot.rs
//
// PR3a vertical slice. Replaces the `query_stub` in
// `media_abilities.rs` for the `camera.snapshot` name with a real
// envelope-aware handler that:
//
//   1. Reads `EnvelopeContext.subject` (per **INV-SUBJECT-ENVELOPE**:
//      handler MUST get subject from the envelope, NOT from args).
//   2. Resolves subject → `ResourceEntry` via
//      `persistence::resources::lookup_by_uri`.
//   3. Branches per **INV-RESOURCE-VALIDITY**:
//        * subject absent     → InvalidArgument with reason="subject_required"
//        * URI not in table   → terminal failure with reason="resource_not_found"
//        * type ≠ camera      → terminal failure with reason="resource_type_mismatch"
//        * (PR3a synthetic backend skips the "binding alive" check
//          — every present entry is "available"; the real
//          cpal/nokhwa swap adds the device-still-plugged-in test
//          and returns reason="resource_unavailable" when the
//          camera was unplugged after the table was last scanned.)
//   4. Captures one frame via the configured backend
//      (`SyntheticBackend` in PR3a — produces a deterministic
//       64×48 JPEG so the wire path is testable without hardware
//       or macOS permission prompts).
//   5. base64-encodes the JPEG bytes (per the design discussion —
//      base64 inline is the right tradeoff for snapshot-shaped
//      receipts up to a small-blob threshold; large-image overflow
//      to PayloadStore is the next consumer's PR).
//   6. Returns the receipt body shape declared in
//      `media_abilities::ABILITY_CAMERA_SNAPSHOT`'s description:
//      `{ image_bytes_b64, captured_at, content_type, width,
//         height, hardware_id }`.
//
// What's NOT in PR3a
// ------------------
// * Real camera capture (cpal/nokhwa). The `SnapshotBackend` trait
//   is the seam: a future PR drops in a `NokhwaBackend` impl
//   without touching the dispatch / receipt code.
// * PayloadStore overflow path for >256 KiB images. Returns a
//   clear "image too large for inline; payloadstore path not yet
//   wired" error so a future operator hitting the limit gets a
//   pointer instead of a silent truncation.
// * Args parsing for `format` / `region`. PR3a accepts whatever
//   args are passed (only `subject` matters) and always emits
//   JPEG. Wiring the format selector is a follow-up.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::persistence::resources::{self, lookup_by_uri, ResourceEntry, ResourceType};
use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, EnvelopeContext};
use crate::runtime::agents::media_abilities::{ABILITY_CAMERA_SNAPSHOT, REASON_SUBJECT_IN_ARGS};

/// Maximum inline image size, in encoded JPEG bytes (NOT the base64
/// expansion). Above this the handler refuses with an explicit
/// "use payloadstore" error rather than embedding a multi-MB body
/// in the receipt. Picked at 256 KiB because:
///   * a 1920×1080 JPEG at quality 80 is ~200 KB — comfortably
///     under the cap, the common case ships inline
///   * a 4K (3840×2160) JPEG is typically 1-2 MB — over the cap,
///     forces the right path
///   * base64 expansion is ~33%, so the receipt body stays
///     under 350 KB worst-case — well within the axon 4 MiB
///     IPC frame limit
const MAX_INLINE_BYTES: usize = 256 * 1024;

/// Reason strings the handler emits on terminal failures. Pinned
/// as constants so the integration tests + PR2 sibling-handler
/// guards reference the exact same strings.
pub const REASON_SUBJECT_REQUIRED: &str = "subject_required";
pub const REASON_RESOURCE_NOT_FOUND: &str = "resource_not_found";
pub const REASON_RESOURCE_TYPE_MISMATCH: &str = "resource_type_mismatch";
/// Camera was in the resources table at scan time but is now
/// busy / unplugged / permission-denied. Distinct from
/// `resource_not_found` per INV-RESOURCE-VALIDITY: the URA
/// resolves to an entry, but the underlying device cannot serve
/// a frame right now.
pub const REASON_RESOURCE_UNAVAILABLE: &str = "resource_unavailable";
pub const REASON_IMAGE_TOO_LARGE: &str = "image_too_large_for_inline";

// ── Backend trait ────────────────────────────────────────────

/// One-shot camera capture. The trait is the seam between this
/// PR (which provides `SyntheticBackend`) and a future PR (which
/// drops in `NokhwaBackend` or similar without touching the
/// dispatch / receipt-shaping code above).
pub trait SnapshotBackend: Send + Sync {
    /// Capture one frame from the resource described by `entry`.
    /// Returns the encoded JPEG bytes plus the actual dimensions
    /// (so the receipt can record what was captured even when the
    /// requested resolution couldn't be honoured).
    fn capture_jpeg(&self, entry: &ResourceEntry) -> anyhow::Result<EncodedFrame>;
}

#[derive(Debug, Clone)]
pub struct EncodedFrame {
    pub jpeg_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

// ── Nokhwa backend (real, PR3) ───────────────────────────────

/// Real camera backend backed by `nokhwa`. Opens the platform's
/// default backend (AVFoundation on macOS, V4L2 on Linux), grabs
/// one frame at the camera's chosen "absolute highest resolution"
/// format, decodes to RGB, and re-encodes as JPEG to match the
/// receipt body shape camera.snapshot promises.
///
/// `hardware_id` ↔ camera index mapping
/// ------------------------------------
/// v1 ignores `entry.hardware_id` and grabs the default camera
/// (CameraIndex::Index(0)). The mint-an-index-per-camera scan
/// happens in the daemon's first-boot resource scan, but the
/// stable mapping back to a `CameraIndex` is platform-specific
/// and not yet plumbed through `ResourceEntry.metadata`. As soon
/// as the scan records the index, this backend reads it from
/// `entry.metadata["camera_index"]`.
///
/// Failure mapping
/// ---------------
/// Every `NokhwaError` becomes `reason="resource_unavailable"` —
/// the camera was in the resources table at scan time but is now
/// busy / unplugged / permission-denied. This matches
/// INV-RESOURCE-VALIDITY's split between
/// `resource_not_found` (handled by the dispatch layer above)
/// and `resource_unavailable` (this branch).
#[derive(Debug, Default)]
pub struct NokhwaBackend;

impl SnapshotBackend for NokhwaBackend {
    fn capture_jpeg(&self, entry: &ResourceEntry) -> anyhow::Result<EncodedFrame> {
        use nokhwa::pixel_format::RgbFormat;
        use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
        use nokhwa::Camera;

        let index = entry
            .metadata
            .get("camera_index")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(0);

        let req = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestResolution);
        let mut cam = Camera::new(CameraIndex::Index(index), req).map_err(|e| {
            anyhow::anyhow!(
                "{ABILITY_CAMERA_SNAPSHOT}: nokhwa Camera::new(index={index}) failed: \
                 {e}; reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
        cam.open_stream().map_err(|e| {
            anyhow::anyhow!(
                "{ABILITY_CAMERA_SNAPSHOT}: nokhwa open_stream failed: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
        let buf = cam.frame().map_err(|e| {
            anyhow::anyhow!(
                "{ABILITY_CAMERA_SNAPSHOT}: nokhwa frame() failed: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
        let rgb_image = buf.decode_image::<RgbFormat>().map_err(|e| {
            anyhow::anyhow!(
                "{ABILITY_CAMERA_SNAPSHOT}: nokhwa decode_image failed: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
        let width = rgb_image.width();
        let height = rgb_image.height();
        let rgb = rgb_image.into_raw();
        let mut jpeg = Vec::new();
        let encoder = jpeg_encoder::Encoder::new(&mut jpeg, 80);
        encoder
            .encode(
                &rgb,
                width as u16,
                height as u16,
                jpeg_encoder::ColorType::Rgb,
            )
            .map_err(|e| anyhow::anyhow!("jpeg encode failed: {e}"))?;
        Ok(EncodedFrame {
            jpeg_bytes: jpeg,
            width,
            height,
        })
    }
}

// ── Synthetic backend (PR3a, kept for tests) ─────────────────

/// Deterministic synthetic-frame backend. Produces a 64×48
/// solid-colour JPEG seeded from the `hardware_id` so two
/// invocations against the same resource produce byte-identical
/// frames (testable). Two distinct hardware_ids produce different
/// colours — an integration test seeing the same bytes for two
/// resources would catch a wrong-resource bug.
///
/// The colour scheme: pull three bytes from the FNV-1a hash of
/// `hardware_id` to pick (R, G, B). Cheap, deterministic, and
/// produces visibly distinct frames per resource for any human
/// debugging.
#[derive(Debug, Default)]
pub struct SyntheticBackend;

impl SnapshotBackend for SyntheticBackend {
    fn capture_jpeg(&self, entry: &ResourceEntry) -> anyhow::Result<EncodedFrame> {
        const W: u32 = 64;
        const H: u32 = 48;

        // FNV-1a 24-bit-equivalent over hardware_id → (R, G, B).
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in entry.hardware_id.bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let r = (hash & 0xff) as u8;
        let g = ((hash >> 8) & 0xff) as u8;
        let b = ((hash >> 16) & 0xff) as u8;

        // Solid-colour RGB buffer, then JPEG-encode at quality 80.
        // Quality 80 is the standard "good enough" balance — the
        // synthetic content is uniform so even quality 50 would
        // compress identically; 80 keeps us in line with what a
        // real camera frame would use.
        let mut rgb = Vec::with_capacity((W * H * 3) as usize);
        for _ in 0..(W * H) {
            rgb.push(r);
            rgb.push(g);
            rgb.push(b);
        }

        let mut jpeg = Vec::new();
        let encoder = jpeg_encoder::Encoder::new(&mut jpeg, 80);
        encoder
            .encode(&rgb, W as u16, H as u16, jpeg_encoder::ColorType::Rgb)
            .map_err(|e| anyhow::anyhow!("jpeg encode failed: {e}"))?;

        Ok(EncodedFrame {
            jpeg_bytes: jpeg,
            width: W,
            height: H,
        })
    }
}

// ── Registration ─────────────────────────────────────────────

/// Register `camera.snapshot` with a real envelope-aware handler
/// backed by `backend`. The default constructor uses the synthetic
/// backend; a future bin may swap in a real one by calling this
/// with a different `Arc<dyn SnapshotBackend>`.
///
/// **Important**: this MUST be called AFTER `media_abilities::register`
/// because the registry is replace-on-write — registering the
/// envelope-aware variant after the args-only stub means the
/// dispatcher's "envelope-first" lookup picks this one up and the
/// stub becomes unreachable. Reversing the order silently leaves
/// the stub in place.
pub fn register_with_backend(reg: &mut AxonAbilityCatalog, backend: Arc<dyn SnapshotBackend>) {
    reg.register_rpc_with_envelope_and_owner(
        "device.camera.snapshot",
        OwnerKind::Device,
        Arc::new(move |env: EnvelopeContext, args: Value| handler(&backend, env, args)),
    );
}

/// Register with the real `NokhwaBackend` (PR3 default). The
/// daemon boot path calls this after `media_abilities::register`
/// so the envelope-aware variant supersedes the args-only stub.
/// Tests that need a hardware-free path call
/// `register_with_backend(reg, Arc::new(SyntheticBackend))`
/// instead — the trait keeps the dispatch / receipt code
/// backend-agnostic.
pub fn register(reg: &mut AxonAbilityCatalog) {
    register_with_backend(reg, Arc::new(NokhwaBackend));
}

// ── Handler core ─────────────────────────────────────────────

fn handler(
    backend: &Arc<dyn SnapshotBackend>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    // INV-SUBJECT-ENVELOPE negative half: defend the rule even
    // here (the envelope-aware path SHOULDN'T see args.subject
    // because callers know better, but a buggy caller passing it
    // both ways would be silently OK if we didn't check).
    if let Value::Object(map) = &args {
        if map.contains_key("subject") {
            anyhow::bail!(
                "{ABILITY_CAMERA_SNAPSHOT}: `subject` MUST come from the \
                 invocation envelope, not from args (INV-SUBJECT-ENVELOPE; \
                 reason={REASON_SUBJECT_IN_ARGS})"
            );
        }
    }

    // INV-SUBJECT-ENVELOPE positive half: read subject from envelope.
    let subject = env.subject.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: subject required (resource_ura \
             of a camera); reason={REASON_SUBJECT_REQUIRED}"
        )
    })?;

    // INV-RESOURCE-VALIDITY: distinguish "URI unknown" from "URI
    // known but resource currently unavailable". PR3a's synthetic
    // backend cannot detect "unplugged after scan" — every entry
    // in the table is treated as alive. The
    // resource_unavailable branch lands when the real backend can
    // probe the device (cpal::Device::default_input_config() / etc).
    let file = resources::load().unwrap_or_default();
    let entry = lookup_by_uri(&file, subject).ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: subject {subject} not found in \
             local resources table; reason={REASON_RESOURCE_NOT_FOUND}"
        )
    })?;

    // Type check — `camera.snapshot` over a `mic` resource is a
    // caller bug worth catching loudly. Same shape as a future
    // `policy.evaluate` rule but at the handler edge so we don't
    // spin the camera backend up for nothing.
    if entry.kind != ResourceType::Camera {
        anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: subject {subject} resolves to a \
             {}, not a camera; reason={REASON_RESOURCE_TYPE_MISMATCH}",
            entry.kind.as_str()
        );
    }

    // Capture + encode.
    let EncodedFrame {
        jpeg_bytes,
        width,
        height,
    } = backend.capture_jpeg(entry)?;

    if jpeg_bytes.len() > MAX_INLINE_BYTES {
        anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: encoded image {} bytes exceeds \
             inline cap {MAX_INLINE_BYTES}; payloadstore path not yet \
             wired in PR3a; reason={REASON_IMAGE_TOO_LARGE}",
            jpeg_bytes.len()
        );
    }

    let image_bytes_b64 = BASE64_STANDARD.encode(&jpeg_bytes);
    let captured_at = chrono::Utc::now().to_rfc3339();

    Ok(json!({
        "image_bytes_b64": image_bytes_b64,
        "content_type":    "image/jpeg",
        "width":           width,
        "height":          height,
        "byte_size":       jpeg_bytes.len(),
        "captured_at":     captured_at,
        // hardware_id surfaces here (NOT in meta.list_resources's
        // wire shape, which keeps it audit-only) because a snapshot
        // receipt is the natural place to record "which physical
        // device produced this byte stream" for the auditor.
        "hardware_id":     entry.hardware_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::resources::{
        upsert_resource, ResourceBinding, ResourceUpsert, ResourcesFile,
    };
    use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

    /// Build a one-resource ResourcesFile and return its URA. The
    /// caller passes the file to `lookup_by_uri` via the on-disk
    /// path; tests that need to round-trip through the real handler
    /// must use HomeGuard so `resources::load` reads the right path.
    fn seed_camera(file: &mut ResourcesFile, hardware_id: &str) -> String {
        upsert_resource(
            file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/device/01DEV",
                kind: ResourceType::Camera,
                binding: ResourceBinding::LocalDevice,
                hardware_id,
                display_name: "Test Camera",
                metadata: json!({}),
            },
        )
    }

    /// Test register helper. Tests use SyntheticBackend so the
    /// suite runs hardware-free (CI / Linux-without-camera). The
    /// daemon's `register(reg)` defaults to `NokhwaBackend` which
    /// only works against a real `/dev/video*` or AVFoundation
    /// device.
    fn register_synthetic(reg: &mut AxonAbilityCatalog) {
        register_with_backend(reg, Arc::new(SyntheticBackend));
    }

    /// Synthetic-backend smoke: capture against a hand-built
    /// ResourceEntry, assert the bytes look like a JPEG (magic
    /// number 0xff 0xd8) and the dimensions match.
    #[test]
    fn synthetic_backend_emits_valid_jpeg_with_expected_dimensions() {
        let mut file = ResourcesFile::default();
        seed_camera(&mut file, "h-cam-test-1");
        let entry = file.resources[0].clone();
        let backend = SyntheticBackend;
        let frame = backend.capture_jpeg(&entry).unwrap();
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 48);
        assert!(frame.jpeg_bytes.len() > 200, "tiny JPEG implausibly short");
        // SOI marker (Start of Image) — every JPEG starts with these
        // two bytes. If the encoder was misconfigured (e.g. raw RGB
        // instead of JPEG) this would fail.
        assert_eq!(&frame.jpeg_bytes[..2], &[0xff, 0xd8]);
        // EOI marker (End of Image) — last two bytes of every JPEG.
        assert_eq!(
            &frame.jpeg_bytes[frame.jpeg_bytes.len() - 2..],
            &[0xff, 0xd9]
        );
    }

    /// Two distinct hardware_ids produce distinct JPEG byte
    /// streams. Pin so a future "optimisation" that reused a
    /// cached buffer across resources would surface here as a
    /// wrong-camera attribution bug.
    #[test]
    fn synthetic_backend_emits_distinct_bytes_per_hardware_id() {
        let mut file = ResourcesFile::default();
        seed_camera(&mut file, "h-cam-A");
        seed_camera(&mut file, "h-cam-B");
        let backend = SyntheticBackend;
        let frame_a = backend.capture_jpeg(&file.resources[0]).unwrap();
        let frame_b = backend.capture_jpeg(&file.resources[1]).unwrap();
        assert_ne!(
            frame_a.jpeg_bytes, frame_b.jpeg_bytes,
            "different hardware_ids must produce different bytes"
        );
    }

    /// Same hardware_id → byte-identical output across calls.
    /// Determinism is the test fixture's load-bearing property.
    #[test]
    fn synthetic_backend_is_deterministic_per_hardware_id() {
        let mut file = ResourcesFile::default();
        seed_camera(&mut file, "h-cam-deterministic");
        let entry = file.resources[0].clone();
        let backend = SyntheticBackend;
        let one = backend.capture_jpeg(&entry).unwrap();
        let two = backend.capture_jpeg(&entry).unwrap();
        assert_eq!(one.jpeg_bytes, two.jpeg_bytes);
    }

    /// End-to-end: register with synthetic backend, invoke through
    /// the real dispatcher, assert receipt body shape + base64
    /// decodes back to the same JPEG the backend produced.
    #[test]
    fn handler_returns_receipt_with_base64_jpeg_when_subject_resolves() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        // Seed the on-disk resources.json so the handler's
        // `resources::load` finds it.
        let mut file = ResourcesFile::default();
        let uri = seed_camera(&mut file, "h-cam-e2e");
        resources::save(&file).unwrap();

        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some(uri),
        };
        let resp = dispatcher.execute_rpc(target).unwrap();

        // Required receipt fields (matches the docstring contract).
        for field in [
            "image_bytes_b64",
            "content_type",
            "width",
            "height",
            "byte_size",
            "captured_at",
            "hardware_id",
        ] {
            assert!(
                resp.get(field).is_some(),
                "receipt body missing `{field}`: {resp}"
            );
        }
        assert_eq!(resp["content_type"], "image/jpeg");
        assert_eq!(resp["width"], 64);
        assert_eq!(resp["height"], 48);
        assert_eq!(resp["hardware_id"], "h-cam-e2e");

        // base64 round-trip back to the same bytes the synthetic
        // backend would have produced. Catches a malformed encoder
        // or a wrong-buffer bug.
        let b64 = resp["image_bytes_b64"].as_str().unwrap();
        let decoded = BASE64_STANDARD.decode(b64).unwrap();
        assert_eq!(decoded[..2], [0xff, 0xd8]); // JPEG SOI
        assert_eq!(decoded[decoded.len() - 2..], [0xff, 0xd9]); // EOI
        assert_eq!(decoded.len(), resp["byte_size"].as_u64().unwrap() as usize);
    }

    /// INV-SUBJECT-ENVELOPE positive half: invocation with no
    /// envelope subject (`subject: None`) MUST fail with
    /// reason="subject_required". Without this the handler would
    /// either crash or silently capture from "the first camera",
    /// either of which makes auditing a lie.
    #[test]
    fn handler_rejects_missing_subject_with_subject_required_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None, // no envelope subject
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(
            err.to_string().contains(REASON_SUBJECT_REQUIRED),
            "expected reason={REASON_SUBJECT_REQUIRED}; got: {err}"
        );
    }

    /// INV-RESOURCE-VALIDITY: subject points at a URI not in the
    /// local table → reason="resource_not_found". Distinct from
    /// "URI present but device unplugged" (resource_unavailable —
    /// not testable in PR3a's synthetic backend; covered when the
    /// real backend lands).
    #[test]
    fn handler_rejects_unknown_subject_with_resource_not_found_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        // Save an empty resources.json so load() returns Default
        // rather than picking up some prior test's state.
        resources::save(&ResourcesFile::default()).unwrap();
        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some("easynet:///r/acme/resource/01NEVER-EXISTED".into()),
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(
            err.to_string().contains(REASON_RESOURCE_NOT_FOUND),
            "expected reason={REASON_RESOURCE_NOT_FOUND}; got: {err}"
        );
    }

    /// Caller asks `camera.snapshot` against a `mic` resource_ura
    /// → reason="resource_type_mismatch". A real-world UX bug that
    /// would otherwise produce a confusing later error from the
    /// camera backend; catching at the handler edge is much clearer.
    #[test]
    fn handler_rejects_wrong_type_subject_with_resource_type_mismatch_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let mic_uri = upsert_resource(
            &mut file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/device/01DEV",
                kind: ResourceType::Mic, // not a camera
                binding: ResourceBinding::LocalDevice,
                hardware_id: "h-mic-not-camera",
                display_name: "Not A Camera",
                metadata: json!({}),
            },
        );
        resources::save(&file).unwrap();

        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some(mic_uri),
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(
            err.to_string().contains(REASON_RESOURCE_TYPE_MISMATCH),
            "expected reason={REASON_RESOURCE_TYPE_MISMATCH}; got: {err}"
        );
    }

    /// INV-SUBJECT-ENVELOPE negative half (defence in depth): if
    /// args carry a `subject` key the handler MUST reject before
    /// any other parsing, even on the envelope-aware path. The
    /// `media_abilities` stubs already enforce this for the
    /// args-only path; this test pins the env-aware sibling.
    #[test]
    fn handler_rejects_subject_in_args_even_on_envelope_path() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SNAPSHOT.to_string(),
            normalized_args: json!({"subject": "easynet:///r/x/resource/y"}),
            call_mode: CallMode::Rpc,
            subject: Some("easynet:///r/acme/resource/01CAM".into()),
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(
            err.to_string().contains(REASON_SUBJECT_IN_ARGS),
            "expected reason={REASON_SUBJECT_IN_ARGS}; got: {err}"
        );
    }
}

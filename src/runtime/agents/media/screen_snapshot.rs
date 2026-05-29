// EasyNet CLI — screen.snapshot real handler (RFC-005 v3.2 A8)
// ==============================================================
//
// File: src/runtime/agents/media/screen_snapshot.rs
//
// PR3 vertical slice. Replaces the `query_stub` in
// `media_abilities.rs` for the `screen.snapshot` name with a
// real envelope-aware handler whose flow mirrors
// `camera_snapshot` exactly:
//
//   1. envelope.subject required (INV-SUBJECT-ENVELOPE)
//   2. resolve subject → ResourceEntry, reject when type is not
//      one of {display, application, window} per RFC-005 v3.2
//      §A4 (the screen-target type taxonomy is shared between
//      subscribe and snapshot)
//   3. capture one RGBA frame via the configured backend, encode
//      to JPEG, base64 the bytes if under MAX_INLINE_BYTES
//   4. return { image_bytes_b64, content_type, width, height,
//               byte_size, captured_at, hardware_id }
//
// The backend trait keeps stub / synthetic / real switchable per
// the AXON-RFC-005-device-backend-selection note. PR3 ships an
// `XcapBackend` that captures the primary monitor (scope is
// trimmed to single-display in v1; multi-monitor / window /
// application targeting lands when meta.list_resources actually
// scans them — until then resources.json holds at most one
// `display` entry minted by the daemon's first-boot scan).
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
use crate::runtime::agents::media_abilities::{ABILITY_SCREEN_SNAPSHOT, REASON_SUBJECT_IN_ARGS};

/// 256 KiB inline cap — same shape as camera.snapshot. A 1080p
/// JPEG at quality 80 is ~200 KB; 4K is multi-MB and forces the
/// "image too large" reject (payloadstore overflow lands later).
const MAX_INLINE_BYTES: usize = 256 * 1024;

pub const REASON_SUBJECT_REQUIRED: &str = "subject_required";
pub const REASON_RESOURCE_NOT_FOUND: &str = "resource_not_found";
pub const REASON_RESOURCE_TYPE_MISMATCH: &str = "resource_type_mismatch";
pub const REASON_RESOURCE_UNAVAILABLE: &str = "resource_unavailable";
pub const REASON_IMAGE_TOO_LARGE: &str = "image_too_large_for_inline";

#[derive(Debug, Clone)]
pub struct EncodedFrame {
    pub jpeg_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Backend seam. Same shape as camera.snapshot's
/// `SnapshotBackend`; an `XcapBackend` ships in this PR, a future
/// Wayland-specific or per-window backend can drop in without
/// touching the dispatch / receipt code below.
pub trait ScreenSnapshotBackend: Send + Sync {
    /// Capture one frame from the resource described by `entry`.
    /// Returns the encoded JPEG bytes plus the actual dimensions
    /// (which may differ from any requested resolution if the
    /// backend can't satisfy it exactly).
    fn capture_jpeg(&self, entry: &ResourceEntry) -> anyhow::Result<EncodedFrame>;
}

// ── XcapBackend (real) ───────────────────────────────────────

/// xcap-backed real backend. v1 captures the primary monitor
/// regardless of which `display` resource_ura was passed — the
/// daemon's first-boot scan mints one `display` entry per
/// physical monitor and the dispatcher routes by URA, but the
/// platform layer doesn't yet plumb monitor IDs through (xcap
/// returns a `Monitor` per call rather than a stable handle).
/// Multi-monitor selection by URA is a follow-up; for now the
/// invariant is "primary display always reachable".
#[derive(Debug, Default)]
pub struct XcapBackend;

impl ScreenSnapshotBackend for XcapBackend {
    fn capture_jpeg(&self, _entry: &ResourceEntry) -> anyhow::Result<EncodedFrame> {
        let monitors = xcap::Monitor::all().map_err(|e| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: xcap Monitor::all failed: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
        let monitor = monitors
            .into_iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{ABILITY_SCREEN_SNAPSHOT}: no primary monitor reported by \
                     xcap; reason={REASON_RESOURCE_UNAVAILABLE}"
                )
            })?;
        let rgba = monitor.capture_image().map_err(|e| {
            // The macOS Screen Recording permission denial path
            // surfaces here (xcap returns an error rather than a
            // blank frame). Map both "permission denied" and
            // "device unplugged after scan" to
            // resource_unavailable per INV-RESOURCE-VALIDITY.
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: xcap capture_image failed (likely \
                 macOS Screen Recording permission not granted): {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
        let width = rgba.width();
        let height = rgba.height();
        let rgb = rgba_to_rgb(rgba.into_raw());
        let jpeg = encode_jpeg(&rgb, width as u16, height as u16)?;
        Ok(EncodedFrame {
            jpeg_bytes: jpeg,
            width,
            height,
        })
    }
}

/// Drop the alpha channel from xcap's RGBA8 buffer because
/// jpeg-encoder takes RGB. Walks 4 bytes at a time and copies
/// the first 3.
fn rgba_to_rgb(rgba: Vec<u8>) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((rgba.len() / 4) * 3);
    for chunk in rgba.chunks_exact(4) {
        rgb.push(chunk[0]);
        rgb.push(chunk[1]);
        rgb.push(chunk[2]);
    }
    rgb
}

fn encode_jpeg(rgb: &[u8], width: u16, height: u16) -> anyhow::Result<Vec<u8>> {
    let mut jpeg = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut jpeg, 80);
    encoder
        .encode(rgb, width, height, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| anyhow::anyhow!("jpeg encode failed: {e}"))?;
    Ok(jpeg)
}

// ── SyntheticBackend (test-only) ─────────────────────────────

/// Same FNV-1a colour-from-hardware-id pattern camera.snapshot
/// uses. Lets tests round-trip the full handler without touching
/// the desktop, and lets two distinct screen resources produce
/// distinguishable bytes.
#[derive(Debug, Default)]
pub struct SyntheticScreenBackend;

impl ScreenSnapshotBackend for SyntheticScreenBackend {
    fn capture_jpeg(&self, entry: &ResourceEntry) -> anyhow::Result<EncodedFrame> {
        const W: u32 = 64;
        const H: u32 = 48;
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in entry.hardware_id.bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let r = (hash & 0xff) as u8;
        let g = ((hash >> 8) & 0xff) as u8;
        let b = ((hash >> 16) & 0xff) as u8;
        let mut rgb = Vec::with_capacity((W * H * 3) as usize);
        for _ in 0..(W * H) {
            rgb.push(r);
            rgb.push(g);
            rgb.push(b);
        }
        let jpeg = encode_jpeg(&rgb, W as u16, H as u16)?;
        Ok(EncodedFrame {
            jpeg_bytes: jpeg,
            width: W,
            height: H,
        })
    }
}

// ── Registration ─────────────────────────────────────────────

pub fn register_with_backend(
    reg: &mut AxonAbilityCatalog,
    backend: Arc<dyn ScreenSnapshotBackend>,
) {
    reg.register_rpc_with_envelope_and_owner(
        "device.screen.snapshot",
        OwnerKind::Device,
        Arc::new(move |env: EnvelopeContext, args: Value| handler(&backend, env, args)),
    );
}

/// Default registration uses the real `XcapBackend`. The daemon
/// boot path calls this after `media_abilities::register` so the
/// envelope-aware variant takes precedence over the args-only
/// stub. Tests that need a hardware-free path call
/// `register_with_backend(reg, Arc::new(SyntheticScreenBackend))`
/// instead.
pub fn register(reg: &mut AxonAbilityCatalog) {
    register_with_backend(reg, Arc::new(XcapBackend));
}

// ── Handler core ─────────────────────────────────────────────

fn handler(
    backend: &Arc<dyn ScreenSnapshotBackend>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    if let Value::Object(map) = &args {
        if map.contains_key("subject") {
            anyhow::bail!(
                "{ABILITY_SCREEN_SNAPSHOT}: `subject` MUST come from the \
                 invocation envelope, not from args (INV-SUBJECT-ENVELOPE; \
                 reason={REASON_SUBJECT_IN_ARGS})"
            );
        }
    }
    let subject = env.subject.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: subject required (resource_ura of \
             a display/application/window); reason={REASON_SUBJECT_REQUIRED}"
        )
    })?;

    let file = resources::load().unwrap_or_default();
    let entry = lookup_by_uri(&file, subject).ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: subject {subject} not found in \
             local resources table; reason={REASON_RESOURCE_NOT_FOUND}"
        )
    })?;

    // Per RFC-005 v3.2 §A4, screen targets are one of three
    // resource types. Reject mics/cameras/etc with the same shape
    // camera.snapshot uses for the inverse rejection.
    let kind_ok = matches!(
        entry.kind,
        ResourceType::Display | ResourceType::Application | ResourceType::Window
    );
    if !kind_ok {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: subject {subject} resolves to a {}, \
             not a display/application/window; \
             reason={REASON_RESOURCE_TYPE_MISMATCH}",
            entry.kind.as_str()
        );
    }

    let EncodedFrame {
        jpeg_bytes,
        width,
        height,
    } = backend.capture_jpeg(entry)?;

    if jpeg_bytes.len() > MAX_INLINE_BYTES {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: encoded image {} bytes exceeds inline \
             cap {MAX_INLINE_BYTES}; payloadstore path not yet wired; \
             reason={REASON_IMAGE_TOO_LARGE}",
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

    fn seed_display(file: &mut ResourcesFile, hardware_id: &str) -> String {
        upsert_resource(
            file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/device/01DEV",
                kind: ResourceType::Display,
                binding: ResourceBinding::LocalDevice,
                hardware_id,
                display_name: "Test Display",
                metadata: json!({}),
            },
        )
    }

    fn register_with_synthetic(reg: &mut AxonAbilityCatalog) {
        register_with_backend(reg, Arc::new(SyntheticScreenBackend));
    }

    #[test]
    fn synthetic_backend_emits_valid_jpeg() {
        let mut file = ResourcesFile::default();
        seed_display(&mut file, "h-display-test");
        let entry = file.resources[0].clone();
        let backend = SyntheticScreenBackend;
        let frame = backend.capture_jpeg(&entry).unwrap();
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 48);
        assert_eq!(&frame.jpeg_bytes[..2], &[0xff, 0xd8]); // SOI
        assert_eq!(
            &frame.jpeg_bytes[frame.jpeg_bytes.len() - 2..],
            &[0xff, 0xd9]
        ); // EOI
    }

    #[test]
    fn handler_returns_receipt_with_base64_jpeg_when_subject_resolves() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let uri = seed_display(&mut file, "h-display-e2e");
        resources::save(&file).unwrap();
        let mut reg = AxonAbilityCatalog::new();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_SCREEN_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some(uri),
        };
        let resp = dispatcher.execute_rpc(target).unwrap();
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
        assert_eq!(resp["hardware_id"], "h-display-e2e");
    }

    #[test]
    fn handler_rejects_missing_subject_with_subject_required_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_SCREEN_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None,
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_SUBJECT_REQUIRED));
    }

    #[test]
    fn handler_rejects_camera_subject_with_resource_type_mismatch_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let cam_uri = upsert_resource(
            &mut file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/device/01DEV",
                kind: ResourceType::Camera, // wrong type for screen.snapshot
                binding: ResourceBinding::LocalDevice,
                hardware_id: "h-cam-not-screen",
                display_name: "Not A Screen",
                metadata: json!({}),
            },
        );
        resources::save(&file).unwrap();
        let mut reg = AxonAbilityCatalog::new();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_SCREEN_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some(cam_uri),
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_RESOURCE_TYPE_MISMATCH));
    }

    #[test]
    fn handler_rejects_unknown_subject_with_resource_not_found_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        resources::save(&ResourcesFile::default()).unwrap();
        let mut reg = AxonAbilityCatalog::new();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_SCREEN_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some("easynet:///r/acme/resource/01NEVER".into()),
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_RESOURCE_NOT_FOUND));
    }

    #[test]
    fn handler_rejects_subject_in_args() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_SCREEN_SNAPSHOT.to_string(),
            normalized_args: json!({"subject": "easynet:///r/x/resource/y"}),
            call_mode: CallMode::Rpc,
            subject: Some("easynet:///r/acme/resource/01SCR".into()),
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_SUBJECT_IN_ARGS));
    }
}

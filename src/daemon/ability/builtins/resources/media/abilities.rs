// EasyNet CLI — Media abilities (RFC-005 v3.2 A1-A10)
// =====================================================
//
// File: src/daemon/ability/builtins/resources/media/abilities.rs
//
// Physical-channel abilities, all `subject = resource_ura`.
// Per the binding invariants
// in plan v3.2:
//
//   A1 mic.subscribe        Stream  operational       device
//   A2 camera.subscribe     Stream  operational       device
//   A3 camera.snapshot      Rpc     operational       device
//   A4 screen.subscribe     Stream  operational       device
//   A5 speaker.publish      Bidi    operational       device
//   A6 voice.subscribe      Stream  provider seam     realm Authority
//   A7 voice.transcribe     Bidi    provider seam     realm Authority
//   A8 screen.snapshot      Rpc     operational       device
//   A9 camera.record_start  Rpc     state-transition  device
//   A10 camera.record_stop  Rpc     state-transition  device
//
// Single source of truth: a const `ABILITIES` table holds every
// ability's name + description + input_schema + transport mode +
// receipt semantics, in declaration order. The `register` fn iterates
// it; descriptor projections query the same rows. Adding a media
// ability requires touching exactly one place.
//
// PR2 scope (this file)
// ---------------------
// This module owns the metadata for all eight handlers and the
// temporary stubs for abilities that still have no real module in
// `media/`. What ships in PR2:
//
//   - metadata for all eight names, so `meta.list_abilities` and
//     `gen-ability-tomls` see them
//   - registration of only the still-unwired stubs; real media
//     modules register their own envelope-aware handlers and must
//     not share the same dispatch slot with an args-only stub
//   - description / input_schema / receipt metadata so each descriptor
//     materialises with orthogonal transport and state semantics
//   - validation skeleton enforcing **INV-SUBJECT-ENVELOPE**: the
//     handler MUST reject `args` containing a key named `subject`
//     before any other arg parsing, even when the body is stubbed.
//     This pins the rule from day one so a future contributor
//     cannot accidentally land a real handler that accepts
//     `args.subject`.
//
// Real media backend scope (NOT in this file yet)
// ------------------------------------------------
// Real device IO (cpal mic capture, nokhwa camera capture, screen
// capture), plus realm Authority TTS/ASR providers. Authority voice rows remain descriptor
// metadata only until such a provider is explicitly assembled; this module
// does not publish an unavailable handler. Physical media stubs remain
// Device-owned and never act as a voice proxy or fallback.
//
// INV-RESOURCE-VALIDITY
// ---------------------
// `resource_not_found` vs `resource_unavailable` — split error
// codes per the binding invariants. PR2 stubs only return
// `unimplemented!()` for the device IO branch; PR3 wires the
// distinction (look up `subject` in `resources.rs` → if absent,
// `resource_not_found`; if present but binding dead,
// `resource_unavailable`).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::descriptors::{CallMode, ReceiptSemantics, TransitionClass};
use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, BidiSource, StreamSource};

// ── Ability names (exported so registration + descriptor sites
//    pull from one place) ──────────────────────────────────────

pub const ABILITY_MIC_SUBSCRIBE: &str =
    crate::daemon::ability::names::resources::MEDIA_MIC_SUBSCRIBE;
pub const ABILITY_CAMERA_SUBSCRIBE: &str =
    crate::daemon::ability::names::resources::MEDIA_CAMERA_SUBSCRIBE;
pub const ABILITY_CAMERA_SNAPSHOT: &str =
    crate::daemon::ability::names::resources::MEDIA_CAMERA_SNAPSHOT;
pub const ABILITY_CAMERA_RECORD_START: &str =
    crate::daemon::ability::names::resources::MEDIA_CAMERA_RECORD_START;
pub const ABILITY_CAMERA_RECORD_STOP: &str =
    crate::daemon::ability::names::resources::MEDIA_CAMERA_RECORD_STOP;
pub const ABILITY_SCREEN_SUBSCRIBE: &str =
    crate::daemon::ability::names::resources::MEDIA_SCREEN_SUBSCRIBE;
pub const ABILITY_SCREEN_SNAPSHOT: &str =
    crate::daemon::ability::names::resources::MEDIA_SCREEN_SNAPSHOT;
pub const ABILITY_SPEAKER_PUBLISH: &str =
    crate::daemon::ability::names::resources::MEDIA_SPEAKER_PUBLISH;
pub const ABILITY_VOICE_SUBSCRIBE: &str = crate::daemon::ability::names::resources::VOICE_SUBSCRIBE;
/// Wire name `voice.transcribe` (not bare `transcribe`) because
/// `AbilityDescriptor::new` requires `<namespace>.<verb>` and the
/// transcribe resource is the inverse of voice synthesis (audio →
/// text vs text → audio); both live on the llm-profile.
pub const ABILITY_VOICE_TRANSCRIBE: &str =
    crate::daemon::ability::names::resources::VOICE_TRANSCRIBE;

/// String literal used inside `reject_subject_in_args` errors and
/// matched by the dispatcher's terminal-receipt path. Pinned as a
/// const so a rename trips a compile error rather than silently
/// drifting from the consumer-side string match.
pub const REASON_SUBJECT_IN_ARGS: &str = "subject_in_args";

// ── Dispatch shape + metadata table ──────────────────────────

/// Per-ability static record. Single source of truth for every
/// piece of metadata mod.rs needs (description, input_schema,
/// receipt semantics) plus the call mode `register` consumes.
struct AbilityRow {
    name: &'static str,
    description: &'static str,
    /// JSON Schema builder. Returns a fresh `Value` per call so
    /// callers can mutate without aliasing concerns.
    input_schema: fn() -> Value,
    call_mode: CallMode,
    receipt_semantics: fn() -> ReceiptSemantics,
}

/// All media abilities in declaration order. Adding another
/// media ability means appending one row here — `register` and
/// `metadata(name)` both pick it up automatically; mod.rs's three
/// description/schema/receipt-semantics tables already delegate via
/// `metadata`, so no extra mod.rs edit is needed.
const ABILITIES: &[AbilityRow] = &[
    AbilityRow {
        name: ABILITY_MIC_SUBSCRIBE,
        description: "Subscribe to a microphone resource. Returns a server-pushed \
                      stream of audio BinaryChunk frames in the requested codec. \
                      Subject MUST be a mic resource_ura (use meta.list_resources \
                      to discover).",
        input_schema: capture_audio_args,
        call_mode: CallMode::Stream,
        receipt_semantics: operational_receipt,
    },
    AbilityRow {
        name: ABILITY_CAMERA_SUBSCRIBE,
        description: "Subscribe to a camera resource. Returns a server-pushed \
                      stream of video BinaryChunk frames at the requested fps / \
                      resolution / codec. Subject MUST be a camera resource_ura.",
        input_schema: video_subscribe_args_no_region,
        call_mode: CallMode::Stream,
        receipt_semantics: operational_receipt,
    },
    AbilityRow {
        name: ABILITY_CAMERA_SNAPSHOT,
        description: "Capture one still image from a camera resource. Subject MUST \
                      be a camera resource_ura. Returns { image_bytes_b64 OR \
                      payloadstore_ura, captured_at } in the receipt body.",
        input_schema: snapshot_args_no_region,
        call_mode: CallMode::Rpc,
        receipt_semantics: operational_receipt,
    },
    AbilityRow {
        name: ABILITY_CAMERA_RECORD_START,
        description: "Start a bounded recording session for a camera resource. \
                      Subject MUST be a camera resource_ura. Returns a \
                      recording_session_id that must be passed to \
                      camera.record_stop.",
        input_schema: camera_record_start_args,
        call_mode: CallMode::Rpc,
        receipt_semantics: record_start_receipt,
    },
    AbilityRow {
        name: ABILITY_CAMERA_RECORD_STOP,
        description: "Stop a camera recording session and persist the captured \
                      device-camera artifact. Subject MUST be the same camera \
                      resource_ura used for camera.record_start.",
        input_schema: camera_record_stop_args,
        call_mode: CallMode::Rpc,
        receipt_semantics: record_stop_receipt,
    },
    AbilityRow {
        name: ABILITY_SCREEN_SUBSCRIBE,
        description: "Subscribe to a screen target. Subject MUST be a screen \
                      resource_ura whose type is `display`, `application`, or \
                      `window` (use meta.list_resources to discover). Optional \
                      `region` arg is valid ONLY when subject's type is `display` \
                      (window/application bounds are self-defining).",
        input_schema: video_subscribe_args_with_region,
        call_mode: CallMode::Stream,
        receipt_semantics: operational_receipt,
    },
    AbilityRow {
        name: ABILITY_SCREEN_SNAPSHOT,
        description: "Capture one still image of a screen target. Subject MUST be \
                      a screen resource_ura whose type is `display`, `application`, \
                      or `window`. Optional `region` arg is valid ONLY when \
                      subject's type is `display`.",
        input_schema: snapshot_args_with_region,
        call_mode: CallMode::Rpc,
        receipt_semantics: operational_receipt,
    },
    AbilityRow {
        name: ABILITY_SPEAKER_PUBLISH,
        description: "Push audio frames to a speaker resource. Caller streams \
                      BinaryChunk frames UP; downstream channel exists per axon \
                      bidi shape but emits no frames. Subject MUST be a speaker \
                      resource_ura.",
        input_schema: playback_audio_args,
        call_mode: CallMode::Bidi,
        receipt_semantics: operational_receipt,
    },
    AbilityRow {
        name: ABILITY_VOICE_SUBSCRIBE,
        description:
            "Subscribe to a realm Authority voice-synthesis resource. Returns a server-pushed \
                      stream of TTS audio BinaryChunk frames. Subject MUST be a \
                      voice resource_ura (one realm Authority may expose multiple voice \
                      profiles).",
        input_schema: tts_output_args,
        call_mode: CallMode::Stream,
        receipt_semantics: operational_receipt,
    },
    AbilityRow {
        name: ABILITY_VOICE_TRANSCRIBE,
        description: "Stream audio in, receive transcription text out. True bidi: \
                      caller pushes audio BinaryChunk UP, callee returns text \
                      BinaryChunk (or structured JSON) DOWN. Subject MUST be an \
                      ASR-model resource_ura governed by the realm Authority.",
        input_schema: transcribe_args,
        call_mode: CallMode::Bidi,
        receipt_semantics: operational_receipt,
    },
];

fn operational_receipt() -> ReceiptSemantics {
    ReceiptSemantics::Operational
}

fn record_start_receipt() -> ReceiptSemantics {
    recording_transition(ABILITY_CAMERA_RECORD_START)
}

fn record_stop_receipt() -> ReceiptSemantics {
    recording_transition(ABILITY_CAMERA_RECORD_STOP)
}

fn recording_transition(ability: &str) -> ReceiptSemantics {
    ReceiptSemantics::state_transition(format!("{ability}@v1"), TransitionClass::Operational)
        .expect("static media transition IDs satisfy RFC-006")
}

// ── Public projections ───────────────────────────────────────
//
// Three single-field projections backed by the `ABILITIES` table.
// Each call site needs exactly one of (description, schema, receipt semantics),
// so a bundled struct would force every caller to allocate the
// other two. Keeping the projections separate lets `mod.rs::
// description_for` route 8 names with zero schema allocations,
// and `receipt_semantics_for` routes names with zero schema allocations
// either.
//
// `gen-ability-tomls` is the one site that wants all three; it
// pays for them by calling all three projections (which walk the
// table once each). At ~10 abilities and ~1 regen per
// developer-push, the duplicate walks are noise.

/// Description for a media ability name, or `None` if not a media
/// ability. `&'static str` is `Copy`; the call is cheap.
pub fn description(name: &str) -> Option<&'static str> {
    row(name).map(|r| r.description)
}

/// JSON Schema for a media ability's input args, or `None` if not
/// a media ability. Allocates one `Value` per call (the schema is
/// built fresh; see the schema-fn comment below for why).
pub fn input_schema(name: &str) -> Option<Value> {
    row(name).map(|r| (r.input_schema)())
}

/// Registry manifest for a media ability.
///
/// The `ABILITIES` table is the authoritative media contract; live handler
/// registration must project through this helper instead of registering a
/// handler-only control-plane row and letting `meta.list_abilities` degrade to
/// a schema-less descriptor.
pub(crate) fn registry_manifest(
    name: &'static str,
) -> crate::daemon::ability::manifest::AbilityManifest {
    let row = row(name).unwrap_or_else(|| panic!("{name} must be a registered media ability"));
    crate::daemon::ability::catalog::system_manifest::registry_manifest(
        row.name,
        row.description,
        (row.input_schema)(),
    )
}

/// Receipt/state-machine semantics for a media ability. Transport remains a
/// separate [`CallMode`] in the same authoritative row.
pub fn receipt_semantics(name: &str) -> Option<ReceiptSemantics> {
    row(name).map(|row| (row.receipt_semantics)())
}

/// Canonical invocation transport for a media ability.
pub fn call_mode(name: &str) -> Option<CallMode> {
    row(name).map(|row| row.call_mode)
}

/// Internal table lookup. Centralises the linear scan so each
/// projection above stays one line. Linear scan over 8 entries
/// is faster than any hash-map setup cost; reorder freely.
///
/// Post-M5 of the system-namespace migration: `row.name` values
/// are canonical (`mic.subscribe` etc.) and the lookup
/// matches the catalogue exactly — no prefix gymnastics.
fn row(name: &str) -> Option<&'static AbilityRow> {
    ABILITIES.iter().find(|row| row.name == name)
}

// ── Registration ─────────────────────────────────────────────

/// Register only media abilities that still do not have a real
/// envelope-aware handler module. The full eight-ability metadata
/// remains in `ABILITIES`; handler registration is intentionally
/// narrower so the registry never relies on "real handler overrides
/// stub" precedence.
///
/// Each closure captures `row: &'static AbilityRow` by value
/// (the reference is `Copy + 'static`); no rebinding to `let
/// name = row.name;` is needed.
pub fn register(reg: &mut AxonAbilityCatalog) {
    for row in ABILITIES {
        if has_real_media_handler(row.name) || is_unprovided_realm_authority_voice(row.name) {
            continue;
        }
        let owner = OwnerKind::Device;
        // Post-M3 of the system-namespace migration: `row.name` is
        // already canonical (`device.<segment>.<verb>`). Earlier
        // revisions stored the legacy form in the table and
        // prepended `device.` at registration; the M5 cleanup
        // promoted the table itself, so the registration site
        // passes `row.name` verbatim.
        match row.call_mode {
            CallMode::Rpc => {
                reg.register_rpc_with_spec_and_semantics(
                    row.name,
                    owner.clone(),
                    registry_manifest(row.name),
                    (row.receipt_semantics)(),
                    Arc::new(|args| query_stub(row.name, args)),
                );
            }
            CallMode::Stream => {
                reg.register_stream_with_spec_and_semantics(
                    row.name,
                    owner,
                    registry_manifest(row.name),
                    (row.receipt_semantics)(),
                    Arc::new(|args| stream_stub(row.name, args)),
                );
            }
            CallMode::Bidi => {
                reg.register_bidi_with_spec_and_semantics(
                    row.name,
                    owner,
                    registry_manifest(row.name),
                    (row.receipt_semantics)(),
                    Arc::new(|args| bidi_stub(row.name, args)),
                );
            }
        }
    }
}

fn is_unprovided_realm_authority_voice(name: &str) -> bool {
    matches!(name, ABILITY_VOICE_SUBSCRIBE | ABILITY_VOICE_TRANSCRIBE)
}

fn has_real_media_handler(name: &str) -> bool {
    has_native_media_handler(name)
}

#[cfg(feature = "native-media")]
fn has_native_media_handler(name: &str) -> bool {
    matches!(
        name,
        ABILITY_MIC_SUBSCRIBE
            | ABILITY_CAMERA_SUBSCRIBE
            | ABILITY_CAMERA_SNAPSHOT
            | ABILITY_CAMERA_RECORD_START
            | ABILITY_CAMERA_RECORD_STOP
            | ABILITY_SCREEN_SUBSCRIBE
            | ABILITY_SCREEN_SNAPSHOT
    )
}

#[cfg(not(feature = "native-media"))]
fn has_native_media_handler(_name: &str) -> bool {
    false
}

// ── INV-SUBJECT-ENVELOPE enforcement ─────────────────────────

/// Reject any args object that carries a `subject` key. The
/// invocation `subject` MUST come from the envelope, not from
/// args — see plan v3.2 INV-SUBJECT-ENVELOPE. This guard runs
/// first, before any other arg parsing, so a misuse fails fast
/// with a clear error code rather than silently being accepted.
///
/// Returns `Ok(())` when args do not contain `subject`. Returns
/// an `anyhow::Error` carrying `REASON_SUBJECT_IN_ARGS` so the
/// dispatcher's terminal-receipt path can match on the reason
/// string via the same const.
fn reject_subject_in_args(ability: &str, args: &Value) -> anyhow::Result<()> {
    if let Value::Object(map) = args {
        if map.contains_key("subject") {
            anyhow::bail!(
                "{ability}: `subject` MUST come from the invocation envelope, \
                 not from args (INV-SUBJECT-ENVELOPE; reason={REASON_SUBJECT_IN_ARGS})"
            );
        }
    }
    Ok(())
}

// ── Stub bodies ──────────────────────────────────────────────

fn stream_stub(ability: &str, args: Value) -> anyhow::Result<StreamSource> {
    reject_subject_in_args(ability, &args)?;
    // PR3: resolve envelope.subject → resources.json entry, open
    // the device, encode frames, return SnapshotThenLive(..., rx).
    anyhow::bail!("{ability}: device backend not yet wired (PR3 lands cpal/nokhwa/screen)")
}

fn query_stub(ability: &str, args: Value) -> anyhow::Result<Value> {
    reject_subject_in_args(ability, &args)?;
    // PR3: resolve envelope.subject → device → capture single
    // frame → encode → return { image_bytes_b64 OR
    //                           payloadstore_ura, captured_at }.
    anyhow::bail!("{ability}: device backend not yet wired (PR3 lands snapshot capture)")
}

fn bidi_stub(ability: &str, args: Value) -> anyhow::Result<BidiSource> {
    reject_subject_in_args(ability, &args)?;
    // PR3: resolve envelope.subject → audio device, consume speaker.publish
    // up-frames, decode them, and write them to the cpal output stream.
    anyhow::bail!("{ability}: device backend not yet wired (PR3 lands bidi audio)")
}

// ── Schema fragments ─────────────────────────────────────────
//
// One schema fn per distinct constraint surface. Capture vs
// playback vs TTS-output are separate fns because their codec /
// sample-rate constraints genuinely differ; collapsing them into
// one parameterised helper would silently widen the contract.

/// Audio capture (mic): 16 kHz / 24 kHz / 48 kHz, mono or stereo.
/// 16 kHz is the ASR sweet spot; 48 kHz is the cpal default.
fn capture_audio_args() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "codec":       { "type": "string", "enum": ["opus"] },
            "sample_rate": { "type": "integer", "enum": [16000, 24000, 48000] },
            "channels":    { "type": "integer", "enum": [1, 2] }
        }
    })
}

/// Audio playback (speaker): 24 kHz / 48 kHz, mono or stereo.
/// No 16 kHz because nothing produces TTS at that rate today and
/// allowing it would surprise downstream resamplers.
fn playback_audio_args() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "codec":       { "type": "string", "enum": ["opus"] },
            "sample_rate": { "type": "integer", "enum": [24000, 48000] },
            "channels":    { "type": "integer", "enum": [1, 2] }
        }
    })
}

/// TTS output (voice.subscribe): the realm Authority service selects the voice via
/// subject (resource_ura); the caller picks how it wants the
/// audio framed. No `channels` (TTS is mono); no codec list (the
/// Authority voice resource declares its codec capabilities, the
/// caller asks for one).
fn tts_output_args() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "format":      { "type": "string", "enum": ["opus"] },
            "sample_rate": { "type": "integer", "enum": [24000, 48000] }
        }
    })
}

/// Video subscribe — region disallowed (camera).
fn video_subscribe_args_no_region() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "fps":        { "type": "integer", "minimum": 1, "maximum": 60 },
            "resolution": { "type": "string" },
            "codec":      { "type": "string", "enum": ["h264", "raw", "vp9"] }
        }
    })
}

/// Video subscribe — region permitted (screen, but only when
/// subject's type is `display`; the handler enforces that at
/// PR3, the schema permits and the validation rejects later).
fn video_subscribe_args_with_region() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "fps":        { "type": "integer", "minimum": 1, "maximum": 60 },
            "resolution": { "type": "string" },
            "codec":      { "type": "string", "enum": ["h264", "raw", "vp9"] },
            "region":     region_object()
        }
    })
}

/// Snapshot — region disallowed (camera).
fn snapshot_args_no_region() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "format": { "type": "string", "enum": ["jpeg", "png"] }
        }
    })
}

fn camera_record_start_args() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "fps":             { "type": "integer", "minimum": 1, "maximum": 60 },
            "resolution":      { "type": "string" },
            "codec":           { "type": "string", "enum": ["mjpeg"] },
            "max_duration_ms": { "type": "integer", "minimum": 1000, "maximum": 1800000 },
            "max_bytes":       { "type": "integer", "minimum": 1048576, "maximum": 268435456 }
        }
    })
}

fn camera_record_stop_args() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["recording_session_id"],
        "properties": {
            "recording_session_id": { "type": "string", "minLength": 1 }
        }
    })
}

/// Snapshot — region permitted (screen, with the same display-
/// only rule as `video_subscribe_args_with_region`).
fn snapshot_args_with_region() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "format": { "type": "string", "enum": ["jpeg", "png"] },
            "region": region_object()
        }
    })
}

/// Shared rectangle schema for `region` args. One source of truth
/// so the four screen-capable schemas never drift on x/y/w/h
/// constraints.
fn region_object() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "x": { "type": "integer", "minimum": 0 },
            "y": { "type": "integer", "minimum": 0 },
            "w": { "type": "integer", "minimum": 1 },
            "h": { "type": "integer", "minimum": 1 }
        }
    })
}

/// Transcribe — true bidi audio→text. The `format` field controls
/// the up-direction encoding the caller will push.
fn transcribe_args() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "language":   { "type": "string", "description": "ISO 639-1 code or `auto`." },
            "format":     { "type": "string", "enum": ["opus"] },
            "model_hint": { "type": "string" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DEVICE_URA: &str = "easynet:///r/test/device/media-abilities";

    fn metadata_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(TEST_DEVICE_URA)
    }

    #[test]
    fn registration_dispatches_unwired_stubs_to_the_shape_they_declare() {
        // Two-way pin between the `ABILITIES` table and stub
        // registration: rows without real modules must resolve to
        // a registered handler of their declared dispatch type;
        // rows with real modules must remain unregistered here so
        // one dispatch slot never has both an args-only stub and an
        // envelope-aware handler.
        let mut reg = metadata_test_catalog();
        register(&mut reg);
        for row in ABILITIES {
            if has_real_media_handler(row.name) || is_unprovided_realm_authority_voice(row.name) {
                assert!(
                    reg.get_rpc(row.name).is_none()
                        && reg.get_stream(row.name).is_none()
                        && reg.get_bidi(row.name).is_none(),
                    "{} must not be stub-registered",
                    row.name
                );
                continue;
            }
            match row.call_mode {
                CallMode::Rpc => assert!(
                    reg.get_rpc(row.name).is_some(),
                    "{} declared call_mode=Rpc but not registered as RPC",
                    row.name
                ),
                CallMode::Stream => assert!(
                    reg.has_stream(row.name),
                    "{} declared call_mode=Stream but not registered as Stream",
                    row.name
                ),
                CallMode::Bidi => assert!(
                    reg.has_bidi(row.name),
                    "{} declared call_mode=Bidi but not registered as Bidi",
                    row.name
                ),
            }
        }

        assert!(!reg.has_stream(ABILITY_VOICE_SUBSCRIBE));
        assert!(!reg.has_bidi(ABILITY_VOICE_TRANSCRIBE));
        assert!(reg.get_bidi(ABILITY_SPEAKER_PUBLISH).is_some());
        assert!(reg.resolve_bidi_with_env(ABILITY_SPEAKER_PUBLISH).is_none());
    }

    #[test]
    fn stub_registration_publishes_media_descriptors() {
        let mut reg = metadata_test_catalog();
        register(&mut reg);
        let rows = reg.authority_ability_catalog_snapshot();

        for row in ABILITIES {
            if has_real_media_handler(row.name) || is_unprovided_realm_authority_voice(row.name) {
                continue;
            }
            let descriptor = rows
                .iter()
                .find(|catalog_row| catalog_row.name == row.name)
                .map(|catalog_row| &catalog_row.descriptor)
                .unwrap_or_else(|| panic!("{} must publish a media descriptor", row.name));
            assert_eq!(descriptor.description, row.description);
            assert_eq!(descriptor.input_schema(), &(row.input_schema)());
        }
    }

    #[test]
    fn projections_resolve_every_row_in_the_table() {
        // Single-source pin: every name in `ABILITIES` must
        // resolve through all three projections. Catches a
        // future PR that adds a row but accidentally diverges
        // the table from a projection (e.g. by adding a row
        // outside the iteration range, or by short-circuiting
        // `row()`).
        for row in ABILITIES {
            assert_eq!(description(row.name), Some(row.description));
            assert_eq!(call_mode(row.name), Some(row.call_mode));
            assert_eq!(receipt_semantics(row.name), Some((row.receipt_semantics)()));
            let manifest = registry_manifest(row.name);
            assert_eq!(manifest.description(), row.description);
            assert!(
                input_schema(row.name).is_some(),
                "input_schema for {} returned None",
                row.name
            );
        }
    }

    #[test]
    fn projections_return_none_for_non_media_names() {
        for non_media in ["observe.health", "agent.list", "totally.unknown"] {
            assert!(description(non_media).is_none());
            assert!(input_schema(non_media).is_none());
            assert!(call_mode(non_media).is_none());
            assert!(receipt_semantics(non_media).is_none());
        }
    }

    #[test]
    fn handlers_with_subject_in_args_are_rejected_per_inv_subject_envelope() {
        // INV-SUBJECT-ENVELOPE: every media handler MUST reject
        // args.subject before any other parsing. Tested for every
        // dispatch shape (rpc, stream, bidi) so a future stub
        // copy-paste cannot accidentally drop the guard.
        let bad = json!({"subject": "easynet:///r/x/resource/y"});

        for ability in [
            ABILITY_CAMERA_SNAPSHOT,
            ABILITY_CAMERA_RECORD_START,
            ABILITY_CAMERA_RECORD_STOP,
            ABILITY_SCREEN_SNAPSHOT,
        ] {
            let err = query_stub(ability, bad.clone()).unwrap_err().to_string();
            assert!(
                err.contains(REASON_SUBJECT_IN_ARGS),
                "{ability} did not enforce INV-SUBJECT-ENVELOPE: {err}"
            );
        }
        for ability in [
            ABILITY_MIC_SUBSCRIBE,
            ABILITY_CAMERA_SUBSCRIBE,
            ABILITY_SCREEN_SUBSCRIBE,
        ] {
            let err = stream_stub(ability, bad.clone()).unwrap_err().to_string();
            assert!(
                err.contains(REASON_SUBJECT_IN_ARGS),
                "{ability} did not enforce INV-SUBJECT-ENVELOPE: {err}"
            );
        }
        for ability in [ABILITY_SPEAKER_PUBLISH] {
            let err = bidi_stub(ability, bad.clone()).unwrap_err().to_string();
            assert!(
                err.contains(REASON_SUBJECT_IN_ARGS),
                "{ability} did not enforce INV-SUBJECT-ENVELOPE: {err}"
            );
        }

        let _ = bad;
    }

    #[test]
    fn unprovided_realm_authority_voice_geometries_are_not_registered_or_published() {
        let mut reg = metadata_test_catalog();
        register(&mut reg);
        let rows = reg.authority_ability_catalog_snapshot();
        for voice in [ABILITY_VOICE_SUBSCRIBE, ABILITY_VOICE_TRANSCRIBE] {
            assert!(reg.control_plane_owner(voice).is_none());
            assert!(!rows.iter().any(|row| row.name == voice));
        }
        assert_eq!(call_mode(ABILITY_VOICE_SUBSCRIBE), Some(CallMode::Stream));
        assert_eq!(call_mode(ABILITY_VOICE_TRANSCRIBE), Some(CallMode::Bidi));
    }

    #[test]
    fn handlers_without_subject_in_args_reach_unimplemented_branch() {
        // Without the subject-in-args poison, the still-unwired
        // stubs fall through to the "not yet wired" error. Real
        // camera handlers are skipped by the stub registrar.
        let err = query_stub(ABILITY_SPEAKER_PUBLISH, json!({}))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("device backend not yet wired"),
            "expected stub fall-through, got: {err}"
        );
    }

    #[test]
    fn recording_abilities_are_rpc_operational_transitions() {
        for ability in [ABILITY_CAMERA_RECORD_START, ABILITY_CAMERA_RECORD_STOP] {
            assert_eq!(call_mode(ability), Some(CallMode::Rpc));
            let semantics = receipt_semantics(ability).expect("media semantics");
            let transition = semantics.transition().expect("state transition");
            assert_eq!(
                transition.transition_id(),
                format!("{ability}@v1"),
                "transition identity must not be inferred from Rpc transport"
            );
            assert_eq!(
                transition.transition_class(),
                TransitionClass::Operational,
                "recording session changes runtime state, not canonical state"
            );
        }
    }

    #[test]
    fn non_recording_media_abilities_do_not_claim_state_transitions() {
        for row in ABILITIES {
            if matches!(
                row.name,
                ABILITY_CAMERA_RECORD_START | ABILITY_CAMERA_RECORD_STOP
            ) {
                continue;
            }
            assert_eq!((row.receipt_semantics)(), ReceiptSemantics::Operational);
        }
    }

    #[test]
    fn bidirectional_media_abilities_are_not_collapsed_into_stream_mode() {
        assert_eq!(call_mode(ABILITY_SPEAKER_PUBLISH), Some(CallMode::Bidi));
        assert_eq!(call_mode(ABILITY_VOICE_TRANSCRIBE), Some(CallMode::Bidi));
    }

    #[test]
    fn screen_abilities_permit_region_arg_camera_abilities_do_not() {
        // Schema-side enforcement of the
        // "region only when subject is a display" rule: the
        // screen schemas declare a `region` property, the camera
        // ones do not. (The runtime check that subject's type IS
        // `display` lands in PR3.)
        //
        // Look up by name, not by table position — a future PR
        // that reorders `ABILITIES` shouldn't silently break this
        // test by swapping which schema each index points to.
        let schema = |name| input_schema(name).expect("media schema");
        assert!(schema(ABILITY_CAMERA_SUBSCRIBE)["properties"]
            .get("region")
            .is_none());
        assert!(schema(ABILITY_CAMERA_SNAPSHOT)["properties"]
            .get("region")
            .is_none());
        assert!(schema(ABILITY_SCREEN_SUBSCRIBE)["properties"]["region"].is_object());
        assert!(schema(ABILITY_SCREEN_SNAPSHOT)["properties"]["region"].is_object());
    }
}

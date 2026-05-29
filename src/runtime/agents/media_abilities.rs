// EasyNet CLI — Media abilities (RFC-005 v3.2 A1-A8)
// ====================================================
//
// File: src/runtime/agents/media_abilities.rs
//
// Eight physical-channel abilities, all `subject = resource_ura`,
// all RFC-006 class ∈ {Stream, Query}. Per the binding invariants
// in plan v3.2:
//
//   A1 mic.subscribe        Stream  (server-stream)  device
//   A2 camera.subscribe     Stream  (server-stream)  device
//   A3 camera.snapshot      Query   (rpc)            device
//   A4 screen.subscribe     Stream  (server-stream)  device
//   A5 speaker.publish      Stream  (bidi, down=∅)   device
//   A6 voice.subscribe      Stream  (server-stream)  llm
//   A7 voice.transcribe     Stream  (true bidi)      llm
//   A8 screen.snapshot      Query   (rpc)            device
//
// Single source of truth: a const `ABILITIES` table holds every
// ability's name + description + input_schema + RFC-006 class +
// dispatch shape, in declaration order. The `register` fn iterates
// it; `mod.rs` queries it through `metadata(name)` for its
// description/schema/rfc006 lookup tables. Adding a 9th media
// ability requires touching exactly one place.
//
// PR2 scope (this file)
// ---------------------
// All eight handlers are stubbed (`unimplemented!()` for the
// device IO portion). What ships in PR2:
//
//   - registration of all eight names into the local registry, so
//     `meta.list_abilities` and `gen-ability-tomls` see them
//   - description / input_schema / rfc006 metadata so each TOML
//     materialises with the correct RFC-006 class
//   - validation skeleton enforcing **INV-SUBJECT-ENVELOPE**: the
//     handler MUST reject `args` containing a key named `subject`
//     before any other arg parsing, even when the body is stubbed.
//     This pins the rule from day one so a future contributor
//     cannot accidentally land a real handler that accepts
//     `args.subject`.
//
// PR3 scope (NOT in this file yet)
// --------------------------------
// Real device IO (cpal mic capture, nokhwa camera capture, screen
// capture). The signature change to read `subject` from the
// invocation envelope (rather than args) also lands in PR3 — the
// current `Fn(Value) -> anyhow::Result<Value>` LocalRpcHandler
// signature has no envelope hook, so PR2 stubs document the
// invariant via `reject_subject_in_args` but cannot satisfy the
// positive half of INV-SUBJECT-ENVELOPE without dispatcher
// plumbing. The stubs return InvalidArgument before ever reaching
// the device IO branch, so the rule's negative half is enforceable
// today.
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

use crate::runtime::ability_descriptor::AbilityClass;
use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, BidiSource, StreamSource};
use crate::runtime::agents::ability_toml::Rfc006Metadata;

// ── Ability names (exported so registration + descriptor sites
//    pull from one place) ──────────────────────────────────────

pub const ABILITY_MIC_SUBSCRIBE: &str = "device.mic.subscribe";
pub const ABILITY_CAMERA_SUBSCRIBE: &str = "device.camera.subscribe";
pub const ABILITY_CAMERA_SNAPSHOT: &str = "device.camera.snapshot";
pub const ABILITY_SCREEN_SUBSCRIBE: &str = "device.screen.subscribe";
pub const ABILITY_SCREEN_SNAPSHOT: &str = "device.screen.snapshot";
pub const ABILITY_SPEAKER_PUBLISH: &str = "device.speaker.publish";
pub const ABILITY_VOICE_SUBSCRIBE: &str = "device.voice.subscribe";
/// Wire name `voice.transcribe` (not bare `transcribe`) because
/// `AbilityDescriptor::new` requires `<namespace>.<verb>` and the
/// transcribe resource is the inverse of voice synthesis (audio →
/// text vs text → audio); both live on the llm-profile.
pub const ABILITY_VOICE_TRANSCRIBE: &str = "device.voice.transcribe";

/// String literal used inside `reject_subject_in_args` errors and
/// matched by the dispatcher's terminal-receipt path. Pinned as a
/// const so a rename trips a compile error rather than silently
/// drifting from the consumer-side string match.
pub const REASON_SUBJECT_IN_ARGS: &str = "subject_in_args";

// ── Dispatch shape + metadata table ──────────────────────────

/// Three dispatch shapes the registry distinguishes. Names are
/// the same RFC-006 class names where the mapping is clear (Query
/// for RPC, Stream for stream/bidi); `Bidi` carries the explicit
/// "downstream may be empty" semantics for `speaker.publish`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchShape {
    /// `register_rpc` — RFC-006 Query. Single response.
    Rpc,
    /// `register_stream` — RFC-006 Stream, server-push only.
    Stream,
    /// `register_bidi` — RFC-006 Stream, bidirectional frames
    /// (true bidi for transcribe; downstream-empty for
    /// speaker.publish).
    Bidi,
}

/// Per-ability static record. Single source of truth for every
/// piece of metadata mod.rs needs (description, input_schema,
/// rfc006) plus the dispatch shape `register` consumes.
struct AbilityRow {
    name: &'static str,
    description: &'static str,
    /// JSON Schema builder. Returns a fresh `Value` per call so
    /// callers can mutate without aliasing concerns.
    input_schema: fn() -> Value,
    class: AbilityClass,
    shape: DispatchShape,
}

/// All eight media abilities in declaration order. Adding a 9th
/// media ability means appending one row here — `register` and
/// `metadata(name)` both pick it up automatically; mod.rs's three
/// description/schema/rfc006 tables already delegate via
/// `metadata`, so no extra mod.rs edit is needed.
const ABILITIES: &[AbilityRow] = &[
    AbilityRow {
        name: ABILITY_MIC_SUBSCRIBE,
        description: "Subscribe to a microphone resource. Returns a server-pushed \
                      stream of audio BinaryChunk frames in the requested codec. \
                      Subject MUST be a mic resource_ura (use meta.list_resources \
                      to discover).",
        input_schema: capture_audio_args,
        class: AbilityClass::Stream,
        shape: DispatchShape::Stream,
    },
    AbilityRow {
        name: ABILITY_CAMERA_SUBSCRIBE,
        description: "Subscribe to a camera resource. Returns a server-pushed \
                      stream of video BinaryChunk frames at the requested fps / \
                      resolution / codec. Subject MUST be a camera resource_ura.",
        input_schema: video_subscribe_args_no_region,
        class: AbilityClass::Stream,
        shape: DispatchShape::Stream,
    },
    AbilityRow {
        name: ABILITY_CAMERA_SNAPSHOT,
        description: "Capture one still image from a camera resource. Subject MUST \
                      be a camera resource_ura. Returns { image_bytes_b64 OR \
                      payloadstore_ura, captured_at } in the receipt body.",
        input_schema: snapshot_args_no_region,
        class: AbilityClass::Query,
        shape: DispatchShape::Rpc,
    },
    AbilityRow {
        name: ABILITY_SCREEN_SUBSCRIBE,
        description: "Subscribe to a screen target. Subject MUST be a screen \
                      resource_ura whose type is `display`, `application`, or \
                      `window` (use meta.list_resources to discover). Optional \
                      `region` arg is valid ONLY when subject's type is `display` \
                      (window/application bounds are self-defining).",
        input_schema: video_subscribe_args_with_region,
        class: AbilityClass::Stream,
        shape: DispatchShape::Stream,
    },
    AbilityRow {
        name: ABILITY_SCREEN_SNAPSHOT,
        description: "Capture one still image of a screen target. Subject MUST be \
                      a screen resource_ura whose type is `display`, `application`, \
                      or `window`. Optional `region` arg is valid ONLY when \
                      subject's type is `display`.",
        input_schema: snapshot_args_with_region,
        class: AbilityClass::Query,
        shape: DispatchShape::Rpc,
    },
    AbilityRow {
        name: ABILITY_SPEAKER_PUBLISH,
        description: "Push audio frames to a speaker resource. Caller streams \
                      BinaryChunk frames UP; downstream channel exists per axon \
                      bidi shape but emits no frames. Subject MUST be a speaker \
                      resource_ura.",
        input_schema: playback_audio_args,
        class: AbilityClass::Stream,
        shape: DispatchShape::Bidi,
    },
    AbilityRow {
        name: ABILITY_VOICE_SUBSCRIBE,
        description: "Subscribe to an LLM voice profile. Returns a server-pushed \
                      stream of TTS audio BinaryChunk frames. Subject MUST be a \
                      voice resource_ura (one llm may expose multiple voice \
                      profiles).",
        input_schema: tts_output_args,
        class: AbilityClass::Stream,
        shape: DispatchShape::Stream,
    },
    AbilityRow {
        name: ABILITY_VOICE_TRANSCRIBE,
        description: "Stream audio in, receive transcription text out. True bidi: \
                      caller pushes audio BinaryChunk UP, callee returns text \
                      BinaryChunk (or structured JSON) DOWN. Subject MUST be an \
                      ASR-model resource_ura.",
        input_schema: transcribe_args,
        class: AbilityClass::Stream,
        shape: DispatchShape::Bidi,
    },
];

// ── Public projections ───────────────────────────────────────
//
// Three single-field projections backed by the `ABILITIES` table.
// Each call site needs exactly one of (description, schema, class),
// so a bundled struct would force every caller to allocate the
// other two. Keeping the projections separate lets `mod.rs::
// description_for` route 8 names with zero schema allocations,
// and `rfc006_for` route 8 names with zero schema allocations
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

/// RFC-006 metadata for a media ability, or `None` if not a media
/// ability. No `Value` allocation; cheapest of the three.
pub fn rfc006(name: &str) -> Option<Rfc006Metadata> {
    row(name).map(|r| Rfc006Metadata {
        class: Some(r.class),
        ..Default::default()
    })
}

/// Internal table lookup. Centralises the linear scan so each
/// projection above stays one line. Linear scan over 8 entries
/// is faster than any hash-map setup cost; reorder freely.
///
/// Post-M5 of the system-namespace migration: `row.name` values
/// are canonical (`device.mic.subscribe` etc.) and the lookup
/// matches the catalogue exactly — no prefix gymnastics.
fn row(name: &str) -> Option<&'static AbilityRow> {
    ABILITIES.iter().find(|row| row.name == name)
}

// ── Registration ─────────────────────────────────────────────

/// Register all eight media abilities on the local registry by
/// iterating `ABILITIES`. PR3 replaces the per-shape stubs with
/// real cpal/nokhwa/screen-crate backends.
///
/// Each closure captures `row: &'static AbilityRow` by value
/// (the reference is `Copy + 'static`); no rebinding to `let
/// name = row.name;` is needed.
pub fn register(reg: &mut AxonAbilityCatalog) {
    for row in ABILITIES {
        // Post-M3 of the system-namespace migration: `row.name` is
        // already canonical (`device.<segment>.<verb>`). Earlier
        // revisions stored the legacy form in the table and
        // prepended `device.` at registration; the M5 cleanup
        // promoted the table itself, so the registration site
        // passes `row.name` verbatim.
        match row.shape {
            DispatchShape::Rpc => {
                reg.register_rpc_with_owner(
                    row.name,
                    OwnerKind::Device,
                    Arc::new(|args| query_stub(row.name, args)),
                );
            }
            DispatchShape::Stream => {
                reg.register_stream_with_owner(
                    row.name,
                    OwnerKind::Device,
                    Arc::new(|args| stream_stub(row.name, args)),
                );
            }
            DispatchShape::Bidi => {
                reg.register_bidi_with_owner(
                    row.name,
                    OwnerKind::Device,
                    Arc::new(|args| bidi_stub(row.name, args)),
                );
            }
        }
    }
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
    // PR3: resolve envelope.subject → audio device →
    //   (speaker.publish): consume up-frames, decode, write to
    //                      cpal output stream.
    //   (voice.transcribe): consume up-frames, decode, feed ASR,
    //                       emit text frames down.
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

/// TTS output (voice.subscribe): the LLM picks the voice via
/// subject (resource_ura); the caller picks how it wants the
/// audio framed. No `channels` (TTS is mono); no codec list (the
/// llm's voice resource declares its codec capabilities, the
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

    #[test]
    fn registration_dispatches_every_row_to_the_shape_it_declares() {
        // Two-way pin between the `ABILITIES` table and the
        // registered handlers: every row's `shape` field must
        // resolve to a registered handler of that exact dispatch
        // type. Catches both "row added but not registered" and
        // "row's shape field changed but register() not updated".
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg);
        for row in ABILITIES {
            match row.shape {
                DispatchShape::Rpc => assert!(
                    reg.get_rpc(row.name).is_some(),
                    "{} declared shape=Rpc but not registered as RPC",
                    row.name
                ),
                DispatchShape::Stream => assert!(
                    reg.get_stream(row.name).is_some(),
                    "{} declared shape=Stream but not registered as Stream",
                    row.name
                ),
                DispatchShape::Bidi => assert!(
                    reg.get_bidi(row.name).is_some(),
                    "{} declared shape=Bidi but not registered as Bidi",
                    row.name
                ),
            }
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
            assert_eq!(rfc006(row.name).and_then(|m| m.class), Some(row.class));
            assert!(
                input_schema(row.name).is_some(),
                "input_schema for {} returned None",
                row.name
            );
        }
    }

    #[test]
    fn projections_return_none_for_non_media_names() {
        for non_media in [
            "device.observe.health",
            "device.agent.list",
            "totally.unknown",
        ] {
            assert!(description(non_media).is_none());
            assert!(input_schema(non_media).is_none());
            assert!(rfc006(non_media).is_none());
        }
    }

    #[test]
    fn handlers_with_subject_in_args_are_rejected_per_inv_subject_envelope() {
        // INV-SUBJECT-ENVELOPE: every media handler MUST reject
        // args.subject before any other parsing. Tested for every
        // dispatch shape (rpc, stream, bidi) so a future stub
        // copy-paste cannot accidentally drop the guard.
        let bad = json!({"subject": "easynet:///r/x/resource/y"});

        for ability in [ABILITY_CAMERA_SNAPSHOT, ABILITY_SCREEN_SNAPSHOT] {
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
            ABILITY_VOICE_SUBSCRIBE,
        ] {
            let err = stream_stub(ability, bad.clone()).unwrap_err().to_string();
            assert!(
                err.contains(REASON_SUBJECT_IN_ARGS),
                "{ability} did not enforce INV-SUBJECT-ENVELOPE: {err}"
            );
        }
        for ability in [ABILITY_SPEAKER_PUBLISH, ABILITY_VOICE_TRANSCRIBE] {
            let err = bidi_stub(ability, bad.clone()).unwrap_err().to_string();
            assert!(
                err.contains(REASON_SUBJECT_IN_ARGS),
                "{ability} did not enforce INV-SUBJECT-ENVELOPE: {err}"
            );
        }
    }

    #[test]
    fn handlers_without_subject_in_args_reach_unimplemented_branch() {
        // Without the subject-in-args poison, the stubs fall
        // through to the "not yet wired" error. PR3 will replace
        // that branch with real device IO.
        let err = query_stub(ABILITY_CAMERA_SNAPSHOT, json!({}))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("device backend not yet wired"),
            "expected stub fall-through, got: {err}"
        );
    }

    #[test]
    fn all_eight_abilities_classify_as_query_or_stream() {
        // Sanity pin: no media ability is Transition. A future
        // change reclassifying one (e.g. a stateful media.session)
        // fires this and forces an explicit decision.
        for row in ABILITIES {
            assert!(
                matches!(row.class, AbilityClass::Query | AbilityClass::Stream),
                "{} classified as Transition; would need transition_id, state_type, etc.",
                row.name
            );
        }
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

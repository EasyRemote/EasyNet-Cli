// EasyNet CLI — `<self>.register_device_pubkey` ability handler
// ==============================================================
//
// File: src/services/invocation_transport/register_device_pubkey.rs
// Description: PR-7 commit 5/N. Daemon-side handler for the
//              `<self>.register_device_pubkey` ability that the
//              EasyNet backend invokes from
//              `verifyCredentialLogic` (commit 6/N) once a
//              device's pairing flow has produced a verified
//              public key. Atomically appends the entry to
//              `~/.easynet/realm-trust.toml` (or the path the
//              daemon was booted against) and republishes the
//              shared trust-anchor cell so subsequent admissions
//              see the new key.
//
// Rationale (DEC-010 Option A, mechanism vs policy split)
// -------------------------------------------------------
// Trust-set authorship sits behind a stable ability surface: the
// backend invokes `<self>.register_device_pubkey` over the gRPC
// transport plane like any other ability, with a signed envelope
// admitted by the strict §5.2 pipeline (Backend role).
//
// Inputs
// ------
//   {
//     "agent_ura":      "easynet:///r/{realm}/device/{device_id}" | "easynet:///r/{realm}/hub",
//     "public_key_b64": "<base64 standard, 32-byte ed25519 vk>",
//     "role":           "device" | "backend" | "hub"
//   }
//
// Realm cross-boundary rule
// -------------------------
// `role = "device"` is allowed to register an out-of-realm
// `agent_ura`. Production pairing stamps device URAs under the
// owning user's realm (`tenant_id = user_id`) while the hosting
// daemon may run a platform realm (for example
// `easynet-platform`). The trust anchor is keyed by full URA, not
// by daemon-local realm, so rejecting those device entries would
// make the pairing flow fundamentally incompatible with the
// production topology.
//
// `role = "backend"` and `role = "hub"` remain daemon-local only.
// Backend self-identity must match the hosting daemon's realm, and
// peer-hub entries are operator-curated rather than authored via
// this write surface.
//
// Output
// ------
// `{ "ok": true }` JSON on success. Errors are surfaced through
// `tonic::Status` so the gRPC layer carries the canonical reason
// without forcing a JSON envelope error model.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::prelude::*;
use ed25519_dalek::VerifyingKey;
use serde::Deserialize;
use tonic::Status;

use crate::services::realm_trust_anchor::{
    RealmTrustAnchor, RealmTrustError, TrustedAgent, TrustedAgentRole,
};
use crate::services::trust_anchor_cell::SharedTrustAnchor;

/// Ability name the daemon registers under and the backend invokes
/// against. Stable wire surface — DEC-010 calls this out by name.
///
/// **Wire-pinned** — held on the legacy
/// `<self>.register_device_pubkey` literal until the production
/// hub / backend (EasyNet/backend +  EasyNet-Axon) ship matching
/// dual-name acceptance. EasyNet/backend's
/// `abilityRegisterDevicePubkey` const tracks this string verbatim.
/// M4 of the system-namespace migration is staged for RFC-001
/// v4.1.6's wire-break carrier; see
/// `docs/open-questions/deprecate-self-alias-in-ability-names.md`
/// Stage 2 for the cross-repo coordination plan.
// TODO(RFC-001-v4.1.6 stage-2): rename to
// `device.register_device_pubkey` once the hub ships dual-name
// acceptance. Single grep anchor for all wire-pinned `<self>.*` constants.
pub const ABILITY_SELF_REGISTER_DEVICE_PUBKEY: &str = "<self>.register_device_pubkey";

/// JSON-shaped argument tuple. `role` is a free string here (rather
/// than `TrustedAgentRole` directly) so that an unknown role from a
/// future protocol version surfaces as a clean `invalid_argument`
/// rather than a serde decoder error.
#[derive(Debug, Deserialize)]
struct RegisterArgs {
    agent_ura: String,
    public_key_b64: String,
    role: String,
}

/// Outputs returned to the caller. Currently a `{ok: true}` ack —
/// PR-7 deliberately keeps this minimal so the caller's contract
/// is "ability did or did not succeed". Richer telemetry (e.g.
/// number of trust entries after) can ride on a future bump.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RegisterResponse {
    pub ok: bool,
}

/// Handle one `<self>.register_device_pubkey` invocation.
///
/// Flow:
///
///   1. Decode `arguments` JSON into `RegisterArgs`. Any decoder
///      failure → `Status::invalid_argument`.
///   2. Validate `agent_ura` matches the trust-writer policy:
///      device entries may target any realm, backend/hub entries
///      must stay daemon-local. Policy mismatch →
///      `Status::permission_denied`.
///   3. Snapshot the current trust anchor, build a new
///      `RealmTrustAnchor` with the new entry appended, persist
///      it atomically (`save` does tmpfile + fsync + rename), and
///      `replace` the shared cell.
///   4. Return `{ok: true}` JSON.
///
/// `daemon_realm` is the realm string the daemon was booted with
/// (`DaemonConfig::realm`); it is `&str` to make the call sites
/// trivially testable. `trust_anchor_path` is the file the cell
/// persists to (`/etc/easynet/realm-trust.toml` in production;
/// `tempdir/realm-trust.toml` in tests).
pub fn handle(
    arguments: &[u8],
    daemon_realm: &str,
    trust_anchor_path: &Path,
    cell: &SharedTrustAnchor,
) -> Result<Vec<u8>, Status> {
    let args: RegisterArgs = serde_json::from_slice(arguments).map_err(|err| {
        Status::invalid_argument(format!(
            "<self>.register_device_pubkey: arguments JSON decode failed: {err}"
        ))
    })?;

    if args.agent_ura.is_empty() {
        return Err(Status::invalid_argument(
            "<self>.register_device_pubkey: agent_ura is required",
        ));
    }
    if args.public_key_b64.is_empty() {
        return Err(Status::invalid_argument(
            "<self>.register_device_pubkey: public_key_b64 is required",
        ));
    }
    validate_public_key_b64(&args.public_key_b64)?;

    let role = parse_role(&args.role)?;

    let parsed_realm = parse_realm_from_ura(&args.agent_ura).ok_or_else(|| {
        Status::invalid_argument(format!(
            "<self>.register_device_pubkey: agent_ura `{}` does not match the URA \
             supported canonical/legacy trust-entry shape",
            args.agent_ura,
        ))
    })?;
    // Device roams across realms by design (a laptop paired to
    // realm A keeps its URA when re-anchoring at realm B). Every
    // other role — including User — must register in its home
    // realm. User cross-realm roaming (DEC-EU §multi-realm) does
    // NOT happen via remote registration; instead the visiting
    // realm's admission gate resolves the user pubkey through
    // `federation.resolve_key` against the user's home realm with
    // `presented_pubkey_b64` pinned. That keeps the home realm as
    // the single source of truth for user signing credentials —
    // no peer can fabricate a "user X registered here" row.
    if parsed_realm != daemon_realm && !matches!(role, TrustedAgentRole::Device) {
        return Err(Status::permission_denied(format!(
            "<self>.register_device_pubkey: role `{}` requires agent_ura realm `{parsed_realm}` \
             to match daemon realm `{daemon_realm}` — cross-realm user pubkey resolution \
             happens via federation.resolve_key, not via remote registration",
            args.role,
        )));
    }

    let entry = TrustedAgent {
        agent_ura: args.agent_ura.clone(),
        public_key_b64: args.public_key_b64.clone(),
        role,
        added_at_unix_ms: now_unix_ms(),
        // Cross-hub federation fields (PR-N1 schema-B) only apply
        // to operator-curated `[[trusted_agent]] role = "hub"` entries
        // authored by hand at peer-pairing time. The
        // `<self>.register_device_pubkey` flow registers Backend /
        // Device entries from the device-pairing path; those never
        // dial cross-hub, so the federation fields are always
        // `None` here.
        origin_tenant_id: None,
        hub_endpoint: None,
        tls_ca_pem_path: None,
    };

    // Build the next anchor by snapshotting current entries and
    // applying the role-specific write policy. Device pairing keeps
    // strict duplicate rejection; user registration keeps DEC-EU
    // multi-pubkey semantics; backend / hub registration is an
    // upsert so boot-time identity repair replaces a stale key
    // instead of leaving delegation verification pinned to an old
    // trust row.
    let snapshot = cell.snapshot();
    let mut next_entries: Vec<TrustedAgent> = snapshot.entries_sorted();
    let mut next_anchor =
        RealmTrustAnchor::from_entries(next_entries.split_off(0)).map_err(realm_error_to_status)?;
    match role {
        TrustedAgentRole::Backend | TrustedAgentRole::Hub => next_anchor
            .upsert_singleton_agent(entry.clone())
            .map_err(realm_error_to_status)?,
        TrustedAgentRole::Device | TrustedAgentRole::User => next_anchor
            .append_agent(entry.clone())
            .map_err(realm_error_to_status)?,
    }

    next_anchor
        .save(trust_anchor_path)
        .map_err(realm_error_to_status)?;

    // Publish the new anchor before returning; the next admission
    // call sees the appended entry.
    cell.replace(Arc::new(next_anchor));

    serde_json::to_vec(&RegisterResponse { ok: true }).map_err(|err| {
        Status::internal(format!(
            "<self>.register_device_pubkey: response JSON encode failed: {err}"
        ))
    })
}

/// Parse a `TrustedAgentRole` from a wire-string. The wire shape
/// uses lowercase strings (`device` / `backend` / `hub`) per
/// `TrustedAgentRole`'s `serde(rename_all = "lowercase")`. We do
/// not just `serde_json::from_value` here because the surrounding
/// argument struct is already deserialised — a per-field hand
/// match keeps error messages aimed at the caller.
fn parse_role(raw: &str) -> Result<TrustedAgentRole, Status> {
    match raw {
        "device" => Ok(TrustedAgentRole::Device),
        "backend" => Ok(TrustedAgentRole::Backend),
        "hub" => Ok(TrustedAgentRole::Hub),
        "user" => Ok(TrustedAgentRole::User),
        other => Err(Status::invalid_argument(format!(
            "<self>.register_device_pubkey: role `{other}` is not one of \
             device|backend|hub|user",
        ))),
    }
}

fn validate_public_key_b64(raw: &str) -> Result<(), Status> {
    let decoded = BASE64_STANDARD.decode(raw).map_err(|err| {
        Status::invalid_argument(format!(
            "<self>.register_device_pubkey: public_key_b64 is not valid base64: {err}"
        ))
    })?;
    let bytes: [u8; 32] = decoded.as_slice().try_into().map_err(|_| {
        Status::invalid_argument(format!(
            "<self>.register_device_pubkey: public_key_b64 must decode to exactly 32 bytes, got {}",
            decoded.len()
        ))
    })?;
    VerifyingKey::from_bytes(&bytes).map_err(|err| {
        Status::invalid_argument(format!(
            "<self>.register_device_pubkey: public_key_b64 is not a valid Ed25519 verifying key: {err}"
        ))
    })?;
    Ok(())
}

/// Extract the realm component from a URA `easynet:///r/{realm}/...`
/// URA. Returns `None` if the URA does not match the URA shape;
/// the caller surfaces that as `invalid_argument`. We match on
/// the canonical `easynet:///r/` prefix per RFC 001 §3.1; non-canon
/// prefixes (`easynet://...`, query-stringed, etc.) fall through
/// to `None` so a malformed URA reaches the user not the disk.
pub(crate) fn parse_realm_from_ura(ura: &str) -> Option<String> {
    crate::ura::parse_ura(ura).ok().map(|parsed| parsed.realm)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Map a `RealmTrustError` to an appropriate `tonic::Status`.
/// `DuplicateUra` is `already_exists` (idempotent retry contract:
/// re-registering the same URA is a caller bug, not a transient
/// condition); IO errors are `internal` so callers know to look
/// at daemon logs.
fn realm_error_to_status(err: RealmTrustError) -> Status {
    match err {
        RealmTrustError::DuplicateUra { agent_ura } => Status::already_exists(format!(
            "<self>.register_device_pubkey: agent_ura `{agent_ura}` already in trust set",
        )),
        RealmTrustError::DuplicateUserPubkey { agent_ura } => Status::already_exists(format!(
            "<self>.register_device_pubkey: user `{agent_ura}` already has this exact \
             public key registered (different pubkeys per device are expected; same pubkey \
             twice is a no-op pairing retry)",
        )),
        RealmTrustError::ReadFailed { path, source } => Status::internal(format!(
            "<self>.register_device_pubkey: read {path:?}: {source}"
        )),
        RealmTrustError::ParseFailed { path, source } => Status::internal(format!(
            "<self>.register_device_pubkey: parse {path:?}: {source}"
        )),
        RealmTrustError::WriteFailed { path, source } => Status::internal(format!(
            "<self>.register_device_pubkey: write {path:?}: {source}"
        )),
        RealmTrustError::SerializeFailed { path, source } => Status::internal(format!(
            "<self>.register_device_pubkey: serialize {path:?}: {source}"
        )),
        RealmTrustError::InvalidUraForRole {
            agent_ura,
            role,
            detail,
        } => Status::invalid_argument(format!(
            "<self>.register_device_pubkey: trusted {role} URA `{agent_ura}` is invalid: {detail}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use serde_json::json;
    use tempfile::tempdir;

    fn args_bytes(ura: &str, key: &str, role: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "agent_ura": ura,
            "public_key_b64": key,
            "role": role
        }))
        .expect("encode")
    }

    fn empty_cell() -> SharedTrustAnchor {
        SharedTrustAnchor::default()
    }

    fn fresh_path() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");
        (dir, path)
    }

    fn test_pub_b64() -> String {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        BASE64_STANDARD.encode(signing.verifying_key().to_bytes())
    }

    fn test_pub_b64_with_seed(seed: u8) -> String {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        BASE64_STANDARD.encode(signing.verifying_key().to_bytes())
    }

    #[test]
    fn happy_path_appends_and_persists() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let test_pub_b64 = test_pub_b64();
        let args = args_bytes("easynet:///r/r1/device/alpha", &test_pub_b64, "device");

        let result = handle(&args, "r1", &path, &cell).expect("ok");
        let response: RegisterResponse = serde_json::from_slice(&result).expect("decode");
        assert!(response.ok);

        // Cell observes the new entry.
        let snap = cell.snapshot();
        let entry = snap
            .lookup("easynet:///r/r1/device/alpha")
            .expect("present");
        assert_eq!(entry.public_key_b64, test_pub_b64);
        assert!(matches!(entry.role, TrustedAgentRole::Device));

        // File on disk reflects the entry.
        let from_disk = RealmTrustAnchor::try_load_strict(&path).expect("disk load");
        assert!(from_disk.lookup("easynet:///r/r1/device/alpha").is_some());
    }

    #[test]
    fn cross_realm_device_ura_is_allowed() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes("easynet:///r/r2/device/intruder", &test_pub_b64(), "device");

        let result = handle(&args, "r1", &path, &cell).expect("cross-realm device ok");
        let response: RegisterResponse = serde_json::from_slice(&result).expect("decode");
        assert!(response.ok);
        assert!(cell
            .snapshot()
            .lookup("easynet:///r/r2/device/intruder")
            .is_some());
        assert!(RealmTrustAnchor::try_load_strict(&path)
            .expect("disk load")
            .lookup("easynet:///r/r2/device/intruder")
            .is_some());
    }

    #[test]
    fn cross_realm_backend_uri_rejected_with_permission_denied() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes("easynet:///r/r2/hub", &test_pub_b64(), "backend");

        let err = handle(&args, "r1", &path, &cell).expect_err("must reject cross-realm backend");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("role `backend` requires"));
        assert!(cell.snapshot().is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn malformed_ura_rejected_with_invalid_argument() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes("not-a-ura-ura", &test_pub_b64(), "device");

        let err = handle(&args, "r1", &path, &cell).expect_err("must reject malformed URA");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("URA"));
    }

    #[test]
    fn unknown_role_rejected_with_invalid_argument() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes("easynet:///r/r1/device/x", &test_pub_b64(), "supervisor");
        let err = handle(&args, "r1", &path, &cell).expect_err("must reject unknown role");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("supervisor"));
    }

    #[test]
    fn duplicate_ura_rejected_with_already_exists() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes("easynet:///r/r1/device/dup", &test_pub_b64(), "device");
        handle(&args, "r1", &path, &cell).expect("first ok");
        let err = handle(&args, "r1", &path, &cell).expect_err("second must reject as duplicate");
        assert_eq!(err.code(), tonic::Code::AlreadyExists);
    }

    #[test]
    fn backend_same_ura_new_key_replaces_stale_trust_entry() {
        let (_dir, path) = fresh_path();
        let old_key = test_pub_b64_with_seed(7);
        let new_key = test_pub_b64_with_seed(8);
        let cell = SharedTrustAnchor::new(Arc::new(
            RealmTrustAnchor::from_entries(vec![TrustedAgent {
                agent_ura: "easynet:///r/r1/hub".to_string(),
                public_key_b64: old_key,
                role: TrustedAgentRole::Hub,
                added_at_unix_ms: 1,
                origin_tenant_id: Some("r1".to_string()),
                hub_endpoint: Some("https://127.0.0.1:50443".to_string()),
                tls_ca_pem_path: Some(std::path::PathBuf::from("/tmp/r1.ca.pem")),
            }])
            .expect("stale hub anchor"),
        ));
        handle(
            &args_bytes("easynet:///r/r1/hub", &new_key, "backend"),
            "r1",
            &path,
            &cell,
        )
        .expect("backend key rotation ok");

        let snap = cell.snapshot();
        let entry = snap.lookup("easynet:///r/r1/hub").expect("backend present");
        assert_eq!(entry.public_key_b64, new_key);
        assert_eq!(entry.role, TrustedAgentRole::Hub);
        assert_eq!(
            entry.hub_endpoint.as_deref(),
            Some("https://127.0.0.1:50443")
        );
        assert_eq!(
            entry.tls_ca_pem_path.as_deref(),
            Some(Path::new("/tmp/r1.ca.pem"))
        );
        assert_eq!(snap.len(), 1);

        let from_disk = RealmTrustAnchor::try_load_strict(&path).expect("disk load");
        let disk_entry = from_disk
            .lookup("easynet:///r/r1/hub")
            .expect("backend present on disk");
        assert_eq!(disk_entry.public_key_b64, new_key);
        assert_eq!(disk_entry.role, TrustedAgentRole::Hub);
        assert_eq!(
            disk_entry.hub_endpoint.as_deref(),
            Some("https://127.0.0.1:50443")
        );
        assert_eq!(
            disk_entry.tls_ca_pem_path.as_deref(),
            Some(Path::new("/tmp/r1.ca.pem"))
        );
        assert_eq!(from_disk.len(), 1);
    }

    #[test]
    fn empty_agent_ura_rejected_with_invalid_argument() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes("", &test_pub_b64(), "device");
        let err = handle(&args, "r1", &path, &cell).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("agent_ura is required"));
    }

    #[test]
    fn empty_public_key_rejected_with_invalid_argument() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes("easynet:///r/r1/device/z", "", "device");
        let err = handle(&args, "r1", &path, &cell).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("public_key_b64 is required"));
    }

    #[test]
    fn malformed_public_key_b64_rejected_with_invalid_argument() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes("easynet:///r/r1/device/z", "@@@not-base64@@@", "device");
        let err = handle(&args, "r1", &path, &cell).expect_err("must reject malformed base64");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("not valid base64"));
    }

    #[test]
    fn wrong_length_public_key_rejected_with_invalid_argument() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let short_key = BASE64_STANDARD.encode([0u8; 31]);
        let args = args_bytes("easynet:///r/r1/device/z", &short_key, "device");
        let err = handle(&args, "r1", &path, &cell).expect_err("must reject short key");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("exactly 32 bytes"));
    }

    #[test]
    fn invalid_curve_public_key_rejected_with_invalid_argument() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let invalid_key = "xxdqcD1MnE8te47Y0dRcLz8rn+fYxqSy8eDZyLem9f8=";
        let args = args_bytes("easynet:///r/r1/device/z", invalid_key, "device");
        let err = handle(&args, "r1", &path, &cell).expect_err("must reject invalid curve point");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("valid Ed25519 verifying key"));
    }

    #[test]
    fn malformed_json_rejected_with_invalid_argument() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let err =
            handle(b"{not json}", "r1", &path, &cell).expect_err("must reject malformed JSON");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("JSON decode failed"));
    }

    #[test]
    fn append_preserves_existing_entries() {
        // First register a backend, then a device. Both must be in
        // the cell + on disk after the second call.
        let (_dir, path) = fresh_path();
        let cell = empty_cell();

        handle(
            &args_bytes("easynet:///r/r1/hub", &test_pub_b64(), "backend"),
            "r1",
            &path,
            &cell,
        )
        .expect("backend ok");
        handle(
            &args_bytes("easynet:///r/r1/device/device-A", &test_pub_b64(), "device"),
            "r1",
            &path,
            &cell,
        )
        .expect("device ok");

        let snap = cell.snapshot();
        assert_eq!(snap.len(), 2);
        let from_disk = RealmTrustAnchor::try_load_strict(&path).expect("disk load");
        assert_eq!(from_disk.len(), 2);
    }

    #[test]
    fn parse_realm_from_ura_handles_canonical_shape() {
        assert_eq!(
            parse_realm_from_ura("easynet:///r/realm-x/device/n1"),
            Some("realm-x".to_string())
        );
        assert_eq!(
            parse_realm_from_ura("easynet:///r/abc/hub"),
            Some("abc".to_string())
        );
        assert_eq!(parse_realm_from_ura("easynet:///r//device/n1"), None);
        assert_eq!(parse_realm_from_ura("https://example.com"), None);
        assert_eq!(parse_realm_from_ura("easynet://r/x/device/n1"), None); // missing third slash
    }
}

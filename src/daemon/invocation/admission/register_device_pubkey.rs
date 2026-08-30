// EasyNet CLI — `identity.register_pubkey` ability handler
// =========================================================
//
// File: src/daemon/invocation/register_device_pubkey.rs
// Description: PR-7 commit 5/N. Daemon-side handler for the
//              `identity.register_pubkey` ability that the
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
// backend invokes `identity.register_pubkey` over the gRPC
// transport plane like any other ability, with a signed envelope
// admitted by the strict §5.2 pipeline (Backend role).
//
// Inputs
// ------
//   {
//     "principal_ura":  "easynet:///r/{realm}/device/{device_id}"
//                       | "easynet:///r/{realm}/authority",
//     "public_key_b64": "<base64 standard, 32-byte ed25519 vk>",
//     "role":           "device" | "backend" | "hub"
//   }
//
// Realm cross-boundary rule
// -------------------------
// `role = "device"` is allowed to register an out-of-realm
// `principal_ura`. Production pairing stamps device URAs under the
// owning user's realm (`realm = user_id`) while the hosting
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

use axon_sdk::pb::axon::v1::Envelope;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::Deserialize;
use tonic::Status;

use crate::daemon::invocation::admission::runtime_trust::RuntimeTrust;
use crate::daemon::trust::anchor::{TrustAnchorRole, TrustedPrincipalOwner};
use crate::daemon::trust::cell::SharedTrustAnchor;

/// Canonical daemon identity/trust ability name.
pub const ABILITY_IDENTITY_REGISTER_PUBKEY: &str =
    crate::daemon::ability::names::federation::IDENTITY_REGISTER_PUBKEY;

/// JSON-shaped argument tuple. `role` is a free string here (rather
/// than `TrustAnchorRole` directly) so that an unknown trust-anchor role from a
/// future protocol version surfaces as a clean `invalid_argument`
/// rather than a serde decoder error.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterArgs {
    principal_ura: String,
    public_key_b64: String,
    role: String,
    #[serde(default)]
    principal_owner_ura: Option<String>,
}

/// Canonical request DTO for `identity.register_pubkey`.
///
/// Producers use this type instead of hand-writing JSON so the retired
/// `agent_ura` field cannot leak back into trust-sync or prelude paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisterPubkeyRequest {
    principal_ura: String,
    public_key_b64: String,
    role: TrustAnchorRole,
    principal_owner_ura: Option<String>,
}

impl RegisterPubkeyRequest {
    pub(crate) fn new(
        principal_ura: impl Into<String>,
        public_key_b64: impl Into<String>,
        role: TrustAnchorRole,
    ) -> Self {
        Self {
            principal_ura: principal_ura.into(),
            public_key_b64: public_key_b64.into(),
            role,
            principal_owner_ura: None,
        }
    }

    pub(crate) fn with_principal_owner(mut self, owner_ura: impl Into<String>) -> Self {
        self.principal_owner_ura = Some(owner_ura.into());
        self
    }

    pub(crate) fn to_arguments_bytes(&self) -> serde_json::Result<Vec<u8>> {
        #[derive(serde::Serialize)]
        struct Wire<'a> {
            principal_ura: &'a str,
            public_key_b64: &'a str,
            role: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            principal_owner_ura: Option<&'a str>,
        }

        serde_json::to_vec(&Wire {
            principal_ura: &self.principal_ura,
            public_key_b64: &self.public_key_b64,
            role: role_wire(self.role),
            principal_owner_ura: self.principal_owner_ura.as_deref(),
        })
    }
}

/// Narrow policy view of an `identity.register_pubkey` request.
///
/// The dispatcher needs this before persistence so it can decide
/// whether the admitted caller is allowed to author the requested
/// trust-row role. The full writer still owns public-key validation,
/// realm rules, duplicate policy, and atomic save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisterPubkeyIntent {
    principal_ura: String,
    role: TrustAnchorRole,
    principal_owner_ura: Option<String>,
}

impl RegisterPubkeyIntent {
    pub(crate) fn principal_ura(&self) -> &str {
        &self.principal_ura
    }

    pub(crate) fn role(&self) -> TrustAnchorRole {
        self.role
    }

    pub(crate) fn principal_owner_ura(&self) -> Option<&str> {
        self.principal_owner_ura.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn for_test(principal_ura: String, role: TrustAnchorRole) -> Self {
        Self {
            principal_ura,
            role,
            principal_owner_ura: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test_with_owner(
        principal_ura: String,
        role: TrustAnchorRole,
        principal_owner_ura: String,
    ) -> Self {
        Self {
            principal_ura,
            role,
            principal_owner_ura: Some(principal_owner_ura),
        }
    }
}

/// Verified first-key bootstrap claim for user self-registration.
///
/// This is the only `identity.register_pubkey` form that may be admitted with
/// a request-scoped bootstrap candidate key. It covers the cold-start case
/// where the user key is necessarily presented by the request being admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisterPubkeyBootstrapClaim {
    principal_ura: String,
    public_key: [u8; 32],
}

impl RegisterPubkeyBootstrapClaim {
    pub(crate) fn principal_ura(&self) -> &str {
        &self.principal_ura
    }

    pub(crate) fn public_key(&self) -> [u8; 32] {
        self.public_key
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisterPubkeyBootstrapTuple {
    caller_ura: String,
}

impl RegisterPubkeyBootstrapTuple {
    pub(crate) fn from_envelope(envelope: &Envelope) -> Result<Self, Status> {
        let caller_ura = envelope
            .caller
            .as_ref()
            .map(|caller| caller.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or_else(|| Status::invalid_argument("identity.register_pubkey missing caller"))?;
        let callee_ura = envelope
            .callee
            .as_ref()
            .map(|callee| callee.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument("identity.register_pubkey missing authority callee")
            })?;
        let subject_ura = envelope
            .subject
            .as_ref()
            .map(|subject| subject.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or_else(|| Status::invalid_argument("identity.register_pubkey missing subject"))?;

        let caller = crate::core::ura::parse_ura(caller_ura).map_err(|err| {
            Status::invalid_argument(format!(
                "identity.register_pubkey caller is not a user URA: {err}"
            ))
        })?;
        if caller.kind != crate::core::ura::URAKind::User {
            return Err(Status::invalid_argument(format!(
                "identity.register_pubkey bootstrap caller must identify a user, got {:?}",
                caller.kind
            )));
        }
        let callee = crate::core::ura::parse_ura(callee_ura).map_err(|err| {
            Status::invalid_argument(format!(
                "identity.register_pubkey callee is not an authority URA: {err}"
            ))
        })?;
        if callee.kind != crate::core::ura::URAKind::Authority {
            return Err(Status::invalid_argument(format!(
                "identity.register_pubkey bootstrap callee must identify an authority, got {:?}",
                callee.kind
            )));
        }
        if caller.realm != callee.realm {
            return Err(Status::invalid_argument(format!(
                "identity.register_pubkey bootstrap realm mismatch: caller={}, callee={}",
                caller.realm, callee.realm
            )));
        }
        let descriptor_subject =
            crate::core::ura::owner_ability_ura(callee_ura, ABILITY_IDENTITY_REGISTER_PUBKEY);
        if subject_ura != caller_ura && descriptor_subject.as_deref() != Some(subject_ura) {
            return Err(Status::invalid_argument(
                "identity.register_pubkey bootstrap subject must be the user principal or the authority-owned ability URA",
            ));
        }

        Ok(Self {
            caller_ura: caller_ura.to_string(),
        })
    }

    pub(crate) fn matches(envelope: &Envelope) -> bool {
        Self::from_envelope(envelope).is_ok()
    }

    pub(crate) fn caller_ura(&self) -> &str {
        &self.caller_ura
    }
}

/// Outputs returned to the caller. Currently a `{ok: true}` ack —
/// PR-7 deliberately keeps this minimal so the caller's contract
/// is "ability did or did not succeed". Richer telemetry (e.g.
/// number of trust entries after) can ride on a future bump.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RegisterResponse {
    pub ok: bool,
}

/// Handle one `identity.register_pubkey` invocation.
///
/// Flow:
///
///   1. Decode `arguments` JSON into `RegisterArgs`. Any decoder
///      failure → `Status::invalid_argument`.
///   2. Validate `principal_ura` matches the trust-writer policy:
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
    handle_protecting(arguments, daemon_realm, trust_anchor_path, cell, None)
}

/// `handle`, plus one public key the user-key cap's LRU eviction must not
/// select while making room for this registration. See
/// `RuntimeTrust::register_pubkey_protecting` for why a multi-key import
/// loop needs this.
pub fn handle_protecting(
    arguments: &[u8],
    daemon_realm: &str,
    trust_anchor_path: &Path,
    cell: &SharedTrustAnchor,
    protected_public_key_b64: Option<&str>,
) -> Result<Vec<u8>, Status> {
    let (args, role) = decode_register_args(arguments)?;

    let owner = trusted_principal_owner_from_args(&args)?;
    let trust = RuntimeTrust::new(daemon_realm, trust_anchor_path, cell);
    if let Some(protected_public_key_b64) = protected_public_key_b64 {
        trust.register_pubkey_protecting(
            args.principal_ura,
            args.public_key_b64,
            role,
            owner,
            Some(protected_public_key_b64),
        )?;
    } else {
        trust.register_pubkey_with_owner(args.principal_ura, args.public_key_b64, role, owner)?;
    }

    serde_json::to_vec(&RegisterResponse { ok: true }).map_err(|err| {
        Status::internal(format!(
            "identity.register_pubkey: response JSON encode failed: {err}"
        ))
    })
}

pub(crate) fn parse_register_pubkey_intent(
    arguments: &[u8],
) -> Result<RegisterPubkeyIntent, Status> {
    let (args, role) = decode_register_args(arguments)?;
    Ok(RegisterPubkeyIntent {
        principal_ura: args.principal_ura,
        role,
        principal_owner_ura: args.principal_owner_ura,
    })
}

pub(crate) fn verify_user_register_pubkey_bootstrap_claim(
    envelope: &Envelope,
    arguments: &[u8],
) -> Result<RegisterPubkeyBootstrapClaim, Status> {
    let (args, role) = decode_register_args(arguments)?;
    if role != TrustAnchorRole::User {
        return Err(Status::permission_denied(
            "identity.register_pubkey bootstrap is restricted to user role",
        ));
    }
    if args.principal_owner_ura.is_some() {
        return Err(Status::permission_denied(
            "identity.register_pubkey bootstrap user self-registration must not bind a separate owner",
        ));
    }

    let tuple = RegisterPubkeyBootstrapTuple::from_envelope(envelope)?;

    if args.principal_ura != tuple.caller_ura() {
        return Err(Status::invalid_argument(
            "identity.register_pubkey bootstrap principal_ura must match envelope caller",
        ));
    }

    let public_key_bytes = BASE64_STANDARD
        .decode(args.public_key_b64.trim())
        .map_err(|err| {
            Status::invalid_argument(format!(
                "identity.register_pubkey public_key_b64 is not base64: {err}"
            ))
        })?;
    let public_key: [u8; 32] = public_key_bytes.try_into().map_err(|bytes: Vec<u8>| {
        Status::invalid_argument(format!(
            "identity.register_pubkey public_key_b64 must decode to 32 bytes, got {}",
            bytes.len()
        ))
    })?;

    Ok(RegisterPubkeyBootstrapClaim {
        principal_ura: args.principal_ura,
        public_key,
    })
}

fn decode_register_args(arguments: &[u8]) -> Result<(RegisterArgs, TrustAnchorRole), Status> {
    let args: RegisterArgs = serde_json::from_slice(arguments).map_err(|err| {
        Status::invalid_argument(format!(
            "identity.register_pubkey: arguments JSON decode failed: {err}"
        ))
    })?;

    if args.principal_ura.is_empty() {
        return Err(Status::invalid_argument(
            "identity.register_pubkey: principal_ura is required",
        ));
    }
    if args.public_key_b64.is_empty() {
        return Err(Status::invalid_argument(
            "identity.register_pubkey: public_key_b64 is required",
        ));
    }
    let role = parse_role(&args.role)?;
    Ok((args, role))
}

fn trusted_principal_owner_from_args(
    args: &RegisterArgs,
) -> Result<Option<TrustedPrincipalOwner>, Status> {
    let owner_ura = args
        .principal_owner_ura
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(owner_ura) = owner_ura else {
        return Ok(None);
    };
    let parsed_owner = crate::core::ura::parse_ura(owner_ura).map_err(|err| {
        Status::invalid_argument(format!(
            "identity.register_pubkey: principal_owner_ura must be a canonical User URA: {err}"
        ))
    })?;
    if parsed_owner.kind != crate::core::ura::URAKind::User {
        return Err(Status::invalid_argument(
            "identity.register_pubkey: principal_owner_ura must be a User URA",
        ));
    }
    let owner_user_id = parsed_owner.user_id().ok_or_else(|| {
        Status::invalid_argument(
            "identity.register_pubkey: principal_owner_ura must include a user id",
        )
    })?;
    Ok(Some(TrustedPrincipalOwner {
        principal_ura: args.principal_ura.clone(),
        owner_user_id: owner_user_id.to_string(),
        owner_ura: owner_ura.to_string(),
        added_at_unix_ms: crate::daemon::invocation::admission::runtime_trust::now_unix_ms(),
    }))
}

/// Parse a `TrustAnchorRole` from a wire-string. The wire shape
/// uses lowercase strings (`device` / `backend` / `hub`) per
/// `TrustAnchorRole`'s `serde(rename_all = "lowercase")`. We do
/// not just `serde_json::from_value` here because the surrounding
/// argument struct is already deserialised — a per-field hand
/// match keeps error messages aimed at the caller.
fn parse_role(raw: &str) -> Result<TrustAnchorRole, Status> {
    match raw {
        "device" => Ok(TrustAnchorRole::Device),
        "backend" => Ok(TrustAnchorRole::Backend),
        "hub" => Ok(TrustAnchorRole::Hub),
        "user" => Ok(TrustAnchorRole::User),
        other => Err(Status::invalid_argument(format!(
            "identity.register_pubkey: role `{other}` is not one of \
             device|backend|hub|user",
        ))),
    }
}

fn role_wire(role: TrustAnchorRole) -> &'static str {
    match role {
        TrustAnchorRole::Device => "device",
        TrustAnchorRole::Backend => "backend",
        TrustAnchorRole::Hub => "hub",
        TrustAnchorRole::User => "user",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent};
    use ed25519_dalek::SigningKey;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn args_bytes(ura: &str, key: &str, role: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "principal_ura": ura,
            "public_key_b64": key,
            "role": role
        }))
        .expect("encode")
    }

    fn args_bytes_with_owner_ura(ura: &str, key: &str, role: &str, owner_ura: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "principal_ura": ura,
            "public_key_b64": key,
            "role": role,
            "principal_owner_ura": owner_ura
        }))
        .expect("encode")
    }

    fn canonical_hub_ura(realm: &str) -> String {
        crate::core::ura::hub_ura(realm)
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
    fn register_pubkey_request_encodes_principal_scoped_tuple() {
        let args = RegisterPubkeyRequest::new(
            "easynet:///r/r1/device/alpha",
            test_pub_b64(),
            TrustAnchorRole::Device,
        )
        .with_principal_owner("easynet:///r/r1/user/user-1")
        .to_arguments_bytes()
        .expect("request encodes");
        let value: serde_json::Value = serde_json::from_slice(&args).expect("request JSON");

        assert_eq!(value["principal_ura"], "easynet:///r/r1/device/alpha");
        assert_eq!(value["role"], "device");
        assert_eq!(value["principal_owner_ura"], "easynet:///r/r1/user/user-1");
        assert!(
            value.get("principal_owner_username").is_none(),
            "register pubkey request DTO must not emit retired owner username aliases"
        );
        assert!(
            value.get("agent_ura").is_none(),
            "register pubkey request DTO must not emit retired agent_ura"
        );
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
        assert!(matches!(entry.role, TrustAnchorRole::Device));

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
    fn principal_owner_is_derived_from_owner_ura_only() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes_with_owner_ura(
            "easynet:///r/r1/device/owned",
            &test_pub_b64(),
            "device",
            "easynet:///r/r1/user/user-1",
        );

        handle(&args, "r1", &path, &cell).expect("owned device ok");
        let snap = cell.snapshot();
        let owner = snap
            .lookup_principal_owner("easynet:///r/r1/device/owned")
            .expect("owner binding");
        assert_eq!(owner.owner_ura, "easynet:///r/r1/user/user-1");
        assert_eq!(owner.owner_user_id, "user-1");
    }

    #[test]
    fn retired_principal_owner_username_is_rejected() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = serde_json::to_vec(&json!({
            "principal_ura": "easynet:///r/r1/device/owned",
            "public_key_b64": test_pub_b64(),
            "role": "device",
            "principal_owner_ura": "easynet:///r/r1/user/user-1",
            "principal_owner_username": "dev"
        }))
        .expect("encode");

        let err = handle(&args, "r1", &path, &cell)
            .expect_err("retired owner username alias must fail closed");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("principal_owner_username"));
    }

    #[test]
    fn principal_owner_ura_must_be_user_ura() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes_with_owner_ura(
            "easynet:///r/r1/device/owned",
            &test_pub_b64(),
            "device",
            "easynet:///r/r1/device/not-user",
        );

        let err = handle(&args, "r1", &path, &cell).expect_err("owner must be user URA");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err
            .message()
            .contains("principal_owner_ura must be a User URA"));
    }

    #[test]
    fn cross_realm_backend_ura_rejected_with_permission_denied() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes(&canonical_hub_ura("r2"), &test_pub_b64(), "backend");

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
    fn duplicate_device_ura_same_key_is_idempotent() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes("easynet:///r/r1/device/dup", &test_pub_b64(), "device");
        handle(&args, "r1", &path, &cell).expect("first ok");
        handle(&args, "r1", &path, &cell).expect("same key retry ok");
    }

    #[test]
    fn duplicate_device_ura_different_key_replaces_stale_trust_entry() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let device_ura = "easynet:///r/r1/device/dup";
        let old_key = test_pub_b64_with_seed(1);
        let new_key = test_pub_b64_with_seed(2);
        let first = args_bytes(device_ura, &old_key, "device");
        let second = args_bytes(device_ura, &new_key, "device");
        handle(&first, "r1", &path, &cell).expect("first ok");
        handle(&second, "r1", &path, &cell).expect("device key rotation replaces stale row");
        assert_eq!(
            cell.snapshot()
                .lookup(device_ura)
                .map(|entry| entry.public_key_b64.as_str()),
            Some(new_key.as_str())
        );
    }

    #[test]
    fn backend_same_ura_new_key_replaces_stale_trust_entry() {
        let (_dir, path) = fresh_path();
        let old_key = test_pub_b64_with_seed(7);
        let new_key = test_pub_b64_with_seed(8);
        let r1_hub = canonical_hub_ura("r1");
        let cell = SharedTrustAnchor::new(Arc::new(
            RealmTrustAnchor::from_entries(vec![TrustedAgent {
                agent_ura: r1_hub.clone(),
                public_key_b64: old_key,
                role: TrustAnchorRole::Hub,
                added_at_unix_ms: 1,
                origin_realm: Some("r1".to_string()),
                hub_endpoint: Some("https://127.0.0.1:50443".to_string()),
                tls_ca_pem_path: Some(std::path::PathBuf::from("/tmp/r1.ca.pem")),
            }])
            .expect("stale hub anchor"),
        ));
        handle(
            &args_bytes(&r1_hub, &new_key, "backend"),
            "r1",
            &path,
            &cell,
        )
        .expect("backend key rotation ok");

        let snap = cell.snapshot();
        let entry = snap.lookup(&r1_hub).expect("backend present");
        assert_eq!(entry.public_key_b64, new_key);
        assert_eq!(entry.role, TrustAnchorRole::Hub);
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
        let disk_entry = from_disk.lookup(&r1_hub).expect("backend present on disk");
        assert_eq!(disk_entry.public_key_b64, new_key);
        assert_eq!(disk_entry.role, TrustAnchorRole::Hub);
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
    fn empty_principal_ura_rejected_with_invalid_argument() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = args_bytes("", &test_pub_b64(), "device");
        let err = handle(&args, "r1", &path, &cell).expect_err("must reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("principal_ura is required"));
    }

    #[test]
    fn register_rejects_retired_agent_ura_request_field() {
        let (_dir, path) = fresh_path();
        let cell = empty_cell();
        let args = serde_json::to_vec(&json!({
            "agent_ura": "easynet:///r/r1/device/alpha",
            "public_key_b64": test_pub_b64(),
            "role": "device"
        }))
        .expect("encode legacy args");

        let err = handle(&args, "r1", &path, &cell).expect_err("must reject retired field");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("unknown field"));
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
        let r1_hub = canonical_hub_ura("r1");

        handle(
            &args_bytes(&r1_hub, &test_pub_b64(), "backend"),
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
}

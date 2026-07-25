// EasyNet CLI
// ===========
//
// File: src/cli/commands/join.rs
// Description: `easynet device join <token-or-hub-ura>` — join this device to
//              an EasyNet Hub via the staged HTTP pairing facade or the
//              product Hub URA federation path.
//
// Protocol Responsibility:
// - Preserves the historical Backend HTTP pairing path while the staged
//   product facade remains.
// - Supports product Hub URA joins through Axon `federation.join`.
// - Carries optional product-neutral PrincipalLifecycle proof for Hub URA joins.
// - Creates ~/.easynet/credentials.json; other commands consume it.
//
// Implementation Approach:
// - Routes `easynet:///r/<realm>/authority` through the daemon federation client.
// - Routes legacy tokens through synchronous HTTP until the SPEC authorizes
//   irreversible deletion.
// - Lowers PrincipalLifecycle proof without product account fields.
//
// Usage Contract:
// - Run once per device. Re-running overwrites existing credentials.
// - Principal proof options are valid only on the Hub URA path.
// - After join, run `easynet connect` to start the device agent.
//
// Architectural Position:
// - Device lifecycle entrypoint: join → start → heartbeat → stop → reset.
// - The Hub URA path is the canonical runtime model; HTTP pairing is a staged
//   Backend product facade pending SPEC cutover.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use base64::Engine as _;
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::daemon::boot::join_connection_state::{
    record_snapshot, JoinConnectionSnapshot, JoinConnectionState, JoinFailureCode,
    JoinFailureParts, JoinTransition,
};
use crate::daemon::persistence::config;
use crate::support::platform::{output, sysinfo};

use super::pairing_contract::PairingCredentialEnvelope;
use super::{auth, login, profile};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingPreflight {
    /// Realm reserved by the Hub for this one-shot pairing token.
    realm: String,
    node_id: String,
    /// Realm hub's Ed25519 pubkey (base64). The cold-start
    /// cross-machine fix: backend surfaces this here so the
    /// device can write the hub's `(ura, pubkey, role=hub)` row
    /// into its local `realm-trust.toml` during join, without
    /// needing on-host access to the hub runtime keyring. Empty
    /// responses are rejected by the trust wiring step.
    #[serde(default)]
    hub_public_key_b64: String,
    /// Optional base64-encoded PEM trust anchor for the hub's
    /// public TLS listener. Self-hosted hubs populate this so the
    /// join flow can pin the CA locally before runtime start;
    /// publicly-trusted hubs leave it empty and the daemon later
    /// falls back to native roots.
    #[serde(default)]
    hub_tls_ca_pem_b64: String,
    #[serde(default, rename = "hub_agent_ura")]
    _hub_agent_ura: String,
}

#[derive(Debug, Serialize)]
struct ValidatePairingPayload {
    #[serde(flatten)]
    info: sysinfo::DeviceInfo,
    node_id: String,
    device_public_key: String,
    /// Stable per-machine install id (survives reset). Lets the hub recognise
    /// a returning machine and reuse its node_id on re-pair instead of minting
    /// a fresh one. A hub that predates this field ignores the extra key.
    #[serde(skip_serializing_if = "Option::is_none")]
    install_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct JoinArgs {
    /// Pairing token, hub URA, or '<login-hint>@<realm>'. Omit to use current profile.
    pub target: Option<String>,
    /// Profile to use when target is omitted.
    #[arg(long)]
    pub profile: Option<String>,
    /// Explicit login hint for one-step login+join.
    #[arg(long)]
    pub user: Option<String>,
    /// Explicit Realm for one-step login+join.
    #[arg(long)]
    pub realm: Option<String>,
    /// Hub/Auth endpoint override for profile joins; Hub API base for token joins.
    // No `(default: ...)` in the doc-comment — clap already renders
    // the `[default: …]` suffix from `default_value_t` in `--help`.
    // Listing it twice (once in prose, once via clap) is the kind
    // of duplication silan flagged in the layout review.
    #[arg(long, default_value_t = format!("https://{}", config::DEFAULT_HUB_HOST))]
    pub hub: String,
    // Description kept to one short line — clap 4's wrap algorithm
    // declines to break URLs / backtick-fenced strings, so a long
    // description spills past the term_width set in
    // `bin/easynet.rs::apply_help_layout`. Verbose context lives
    // in docs/ and the join.rs commit history.
    /// Override Hub REST API base URL (local-dev only).
    #[arg(long)]
    pub hub_api: Option<String>,
    /// Password for one-step login+join. If omitted, prompt when login is needed.
    #[arg(long)]
    pub password: Option<String>,
    /// Register the user if one-step login fails and the backend supports registration.
    #[arg(long)]
    pub register_if_missing: bool,
    /// Nickname to use when --register-if-missing creates a user.
    #[arg(long)]
    pub nickname: Option<String>,
    // The doc-comment below is a single paragraph on purpose. clap
    // switches `--help` into multi-paragraph "long help" mode the
    // moment ANY arg's doc-comment has a blank line in it — every
    // other arg in the same struct then renders with extra spacing
    // around it. The detailed rationale for `--peer-hub` (Hub
    // pairing response carries the backend Axon endpoint, not the
    // peer daemon's TLS listener; multi-hub deployments diverge)
    // lives in docs/spec/RFC-002 §canonical Invocation::Invoke and in
    // the auto-wire commit message — that's where verbose context
    // belongs, not in `--help`.
    /// Peer hub's daemon TLS listener (https://host:port).
    #[arg(long)]
    pub peer_hub: Option<String>,
    /// PEM CA bundle for a private/self-hosted hub URA join.
    #[arg(long)]
    pub hub_ca: Option<PathBuf>,
    /// Override the daemon TLS port derived from a hub URA.
    #[arg(long)]
    pub hub_port: Option<u16>,
    /// Principal URA to bind this joined device to on the Hub URA path.
    #[arg(long)]
    pub principal_ura: Option<String>,
    /// Enrollment capability id for joining this device as --principal-ura.
    #[arg(long)]
    pub principal_enrollment_id: Option<String>,
    /// PrincipalLifecycle proof kind for --principal-ura.
    #[arg(long)]
    pub principal_proof_kind: Option<String>,
    /// PrincipalLifecycle proof reference for --principal-ura.
    #[arg(long)]
    pub principal_proof_ref: Option<String>,
    /// Skip confirmation prompts (for non-interactive use)
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Start the daemon after pairing. Pass `--boot no` to skip the
    /// auto-start and keep the historical "join only" behaviour
    /// (useful for scripted enrolment where the daemon is started
    /// later by a supervisor). `--boot yes` is the default.
    #[arg(long, value_enum, default_value_t = JoinBoot::Yes)]
    pub boot: JoinBoot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum JoinBoot {
    Yes,
    No,
}

enum ResolvedJoinTarget {
    Direct(String),
    Profile {
        profile: profile::ProfileEntry,
        token: Option<String>,
        login_recovery: bool,
    },
}

impl ResolvedJoinTarget {
    fn login_recovery_profile(&self) -> Option<String> {
        match self {
            Self::Profile {
                profile,
                login_recovery: true,
                ..
            } => Some(profile.profile_name.clone()),
            _ => None,
        }
    }
}

fn resolve_join_target(args: &JoinArgs) -> anyhow::Result<ResolvedJoinTarget> {
    let target = args
        .target
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let login_requested =
        args.user.is_some() || args.realm.is_some() || target.is_some_and(looks_like_login_target);

    if login_requested {
        let hub_override = explicit_join_login_hub_override(args);
        let outcome = login::login_and_select_profile(login::LoginArgs {
            target: args.target.clone(),
            user: args.user.clone(),
            realm: args.realm.clone(),
            hub: hub_override,
            password: args.password.clone(),
            register_if_missing: args.register_if_missing,
            nickname: args.nickname.clone(),
        })?;
        login::render_login_outcome(&outcome);
        return Ok(ResolvedJoinTarget::Profile {
            profile: outcome.profile,
            token: None,
            login_recovery: true,
        });
    }

    if let Some(target) = target {
        return Ok(ResolvedJoinTarget::Direct(target.to_string()));
    }

    let profile = profile::selected_profile(args.profile.as_deref())?;
    Ok(ResolvedJoinTarget::Profile {
        profile,
        token: None,
        login_recovery: false,
    })
}

fn explicit_join_login_hub_override(args: &JoinArgs) -> Option<String> {
    let default_hub = format!("https://{}", config::DEFAULT_HUB_HOST);
    (args.hub.trim_end_matches('/') != default_hub)
        .then(|| args.hub.trim_end_matches('/').to_string())
}

fn looks_like_login_target(target: &str) -> bool {
    target.contains('@') && !target.starts_with(crate::core::ura::URA_SCHEME)
}

fn mint_profile_pairing_token(profile: &profile::ProfileEntry) -> anyhow::Result<String> {
    if profile.account_session != profile::ProfileAccountSessionState::Authenticated {
        anyhow::bail!(
            "profile '{}' is logged out — run 'easynet login {}' first",
            profile.profile_name,
            profile.profile_name
        );
    }
    let session = auth::load_session()?.ok_or_else(|| {
        anyhow::anyhow!(
            "not logged in — run 'easynet login {}' first",
            profile.profile_name
        )
    })?;
    profile::ensure_auth_session_owns_profile(profile, &session)?;
    let token = auth::mint_pairing_token()?.pairing_token;
    Ok(token)
}

pub fn run(args: JoinArgs) -> anyhow::Result<()> {
    let target = resolve_join_target(&args)?;
    let login_recovery_profile = target.login_recovery_profile();
    run_resolved(args, target).map_err(|err| {
        if let Some(profile_name) = login_recovery_profile {
            err.context(format!(
                "login succeeded and profile '{profile_name}' was saved, but device join failed; retry with `easynet join --profile {profile_name}`"
            ))
        } else {
            err
        }
    })
}

fn run_resolved(args: JoinArgs, target: ResolvedJoinTarget) -> anyhow::Result<()> {
    if let ResolvedJoinTarget::Profile {
        profile,
        token: None,
        ..
    } = &target
    {
        if let Ok(existing) = config::load_credentials() {
            if existing_credentials_match_profile(&existing, profile) {
                output::info(&format!(
                    "This device is already joined to Realm {}.",
                    profile.realm_alias
                ));
                return finish_join(args.boot, &existing, args.peer_hub.as_deref());
            }
        }
    }

    // Warn if already paired — prevent accidental overwrite.
    if let Ok(existing) = config::load_credentials() {
        output::warn(&format!(
            "Already paired as {} (hub: {})",
            existing.node_id, existing.hub_endpoint
        ));
        if !args.yes {
            output::info(
                "This will overwrite existing credentials and deregister the old device from the hub.",
            );
            if !output::confirm("Continue?")? {
                output::info("Cancelled.");
                return Ok(());
            }
        }

        // Deregister the OLD node_id NOW, before `save_credentials` overwrites
        // these credentials. Re-pairing mints a FRESH hub-reserved node_id (the
        // pairing protocol has no stable per-machine id to reuse), so the old
        // device is otherwise abandoned: once its node_id is gone from local
        // credentials, neither `easynet reset` nor the shutdown hook (both keyed
        // on `creds.node_id`) can ever revoke it. It then zombies on the hub
        // until the heartbeat-timeout sweep, stranding its trust/pubkey row —
        // which is what breaks user-trust resync and forces credential churn on
        // the next session. Best-effort: a hub that already swept the old id, or
        // an unreachable old hub, must not block re-pairing.
        let old_device_ura = crate::core::ura::device_ura(existing.realm_str(), &existing.node_id);
        match invoke_federation_revoke_for_rejoin(&old_device_ura) {
            Ok(()) => output::info("Old device deregistered with the hub (federation.revoke)."),
            Err(e) => output::warn(&format!(
                "Could not deregister the old device (continuing with re-pair): {e}"
            )),
        }
    }

    let target = match target {
        ResolvedJoinTarget::Direct(target) => target,
        ResolvedJoinTarget::Profile {
            profile,
            token: Some(token),
            ..
        } => {
            output::info(&format!(
                "Using profile {} for Realm {}.",
                profile.profile_name, profile.realm_alias
            ));
            token
        }
        ResolvedJoinTarget::Profile {
            profile,
            token: None,
            ..
        } => {
            output::info(&format!(
                "Requesting device enrollment for profile {}.",
                profile.profile_name
            ));
            mint_profile_pairing_token(&profile)?
        }
    };
    let peer_hub = args.peer_hub.as_deref();
    let creds = if target.starts_with(crate::core::ura::URA_SCHEME) {
        let principal_enrollment = join_principal_enrollment_from_args(
            args.principal_ura.as_deref(),
            args.principal_enrollment_id.as_deref(),
            args.principal_proof_kind.as_deref(),
            args.principal_proof_ref.as_deref(),
        )?;
        run_ura_join_stages(
            &target,
            args.hub_port,
            args.hub_ca.as_deref(),
            peer_hub,
            principal_enrollment,
        )?
    } else {
        if args.principal_ura.is_some()
            || args.principal_proof_kind.is_some()
            || args.principal_proof_ref.is_some()
            || args.principal_enrollment_id.is_some()
        {
            anyhow::bail!(
                "principal enrollment proof is supported only for hub URA joins; use easynet:///r/<realm>/authority"
            );
        }
        let hub_api_override = args
            .hub_api
            .as_ref()
            .map(|s| s.trim_end_matches('/').to_string());
        let has_explicit_hub_api_override = hub_api_override.is_some();
        let validate_base = pick_validate_base(&args.hub, hub_api_override.as_deref());
        if let Err(err) = validate_token_format(&target) {
            record_snapshot(JoinConnectionSnapshot::failed_from_parts(
                JoinFailureParts {
                    failure_code: JoinFailureCode::JoinFailedPreflight,
                    transition: JoinTransition::PreflightToken,
                    realm: String::new(),
                    node_id: String::new(),
                    hub_endpoint: Some(validate_base.clone()),
                    message: err.to_string(),
                    retryable: false,
                    source: "cli.join".to_string(),
                },
            ));
            return Err(err);
        }

        run_join_stages(
            &target,
            &validate_base,
            has_explicit_hub_api_override,
            peer_hub,
        )?
    };

    if let Ok(profile) = profile::selected_profile(args.profile.as_deref()) {
        if profile.realm_alias == creds.realm_str() {
            let _ = profile::mark_device_membership(&profile.profile_name, "enrolled");
        }
    }

    finish_join(args.boot, &creds, peer_hub)
}

fn existing_credentials_match_profile(
    existing: &config::Credentials,
    profile: &profile::ProfileEntry,
) -> bool {
    if existing.realm_str() != profile.realm_alias {
        return false;
    }
    if let Some(subject) = profile
        .subject
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return existing
            .user_id()
            .ok()
            .is_some_and(|user_id| user_id == subject);
    }
    if let Some(login_hint) = profile
        .login_hint
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return existing
            .username
            .as_deref()
            .map(str::trim)
            .is_some_and(|username| username == login_hint);
    }
    true
}

fn finish_join(
    boot: JoinBoot,
    creds: &config::Credentials,
    peer_hub: Option<&str>,
) -> anyhow::Result<()> {
    match boot {
        JoinBoot::No => {
            render_pairing_summary("Paired successfully", creds, peer_hub);
            output::info("Run 'easynet runtime start' to start the device agent.");
            Ok(())
        }
        JoinBoot::Yes => {
            output::info("Pairing accepted. Starting daemon (pass '--boot no' to skip)...");
            super::start::run(super::start::StartArgs::for_join_autostart())
                .map(|()| {
                    render_pairing_summary("Join complete", creds, peer_hub);
                })
                .map_err(|err| {
                    err.context(
                        "pairing credentials were saved, but daemon startup failed; \
                         fix Hub reachability and rerun `easynet runtime start`",
                    )
                })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HubUraTarget {
    hub_ura: String,
    realm: String,
    hub_endpoint: String,
}

impl HubUraTarget {
    fn parse(value: &str, hub_port: Option<u16>, hub_ca: Option<&Path>) -> anyhow::Result<Self> {
        let value = value.trim();
        let parsed = crate::core::ura::parse_ura(value)
            .map_err(|err| anyhow::anyhow!("invalid hub URA `{value}`: {err}"))?;
        if parsed.kind != crate::core::ura::URAKind::Authority {
            anyhow::bail!("join target URA must identify a hub, got {:?}", parsed.kind);
        }
        require_ura_join_trust_policy(&parsed.realm, hub_ca)?;
        let hub_endpoint = hub_endpoint_for_realm(&parsed.realm, hub_port);
        Ok(Self {
            hub_ura: value.to_string(),
            realm: parsed.realm,
            hub_endpoint,
        })
    }
}

fn hub_endpoint_for_realm(realm: &str, hub_port: Option<u16>) -> String {
    let host = if is_loopback_or_localhost(realm) {
        "127.0.0.1"
    } else {
        realm
    };
    let port = hub_port.unwrap_or(50_443).to_string();
    format!("https://{}", format_authority(host, Some(&port)))
}

fn require_ura_join_trust_policy(realm: &str, hub_ca: Option<&Path>) -> anyhow::Result<()> {
    if hub_ca.is_some() || is_public_webpki_realm(realm) {
        return Ok(());
    }
    anyhow::bail!(
        "hub URA realm `{realm}` is not a public WebPKI host; pass --hub-ca for private/self-hosted hubs"
    );
}

fn is_public_webpki_realm(realm: &str) -> bool {
    let realm = realm.trim();
    if realm.is_empty() || is_loopback_or_localhost(realm) {
        return false;
    }
    if let Ok(ip) = realm.parse::<IpAddr>() {
        return is_public_ip(ip);
    }
    let lower = realm.to_ascii_lowercase();
    lower.contains('.')
        && !lower.ends_with(".local")
        && !lower.ends_with(".localhost")
        && !lower.ends_with(".internal")
        && !lower.ends_with(".test")
        && lower
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '.')
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified())
        }
        IpAddr::V6(ip) => {
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local())
        }
    }
}

fn run_ura_join_stages(
    hub_ura: &str,
    hub_port: Option<u16>,
    hub_ca: Option<&Path>,
    peer_hub: Option<&str>,
    principal_enrollment: Option<
        crate::daemon::federation::client::ability_contract::PrincipalEnrollmentProof,
    >,
) -> anyhow::Result<config::Credentials> {
    let mut renderer = crate::cli::presentation::stage::StageRenderer::new();

    renderer.set_active("parse-hub-ura");
    let target = match HubUraTarget::parse(hub_ura, hub_port, hub_ca) {
        Ok(target) => {
            renderer.stage_ok("parse-hub-ura");
            target
        }
        Err(err) => {
            renderer.stage_failed("parse-hub-ura", &format!("{err}"));
            renderer.finish();
            return Err(err);
        }
    };

    let node_id = uuid::Uuid::new_v4().to_string();
    let (membership_ura, _, public_key, key_service_startup) =
        ensure_device_runtime_identity_with_startup(&target.realm, &node_id)?;
    let _key_service_guard = JoinKeyServiceShutdownGuard::new(key_service_startup);
    let public_key_hex = hex::encode(public_key.to_bytes());
    let local_user_id = principal_enrollment
        .as_ref()
        .and_then(|proof| user_id_from_principal_ura(&proof.principal_ura));

    renderer.set_active("federation-join");
    let join = match do_federation_join_and_resolve_hub_key(
        &target,
        &membership_ura,
        &public_key_hex,
        hub_ca,
        principal_enrollment,
    ) {
        Ok(join) => {
            renderer.stage_ok("federation-join");
            join
        }
        Err(err) => {
            renderer.stage_failed("federation-join", &format!("{err}"));
            renderer.finish();
            return Err(err);
        }
    };

    let creds = config::Credentials {
        node_id,
        credential_token: String::new(),
        hub_endpoint: target.hub_endpoint.clone(),
        realm: target.realm.clone(),
        deploy_signature: String::new(),
        hub_api_base: None,
        username: local_user_id.clone(),
        user_id: local_user_id,
        hub_pubkey_b64: Some(hex_public_key_to_b64(&join.hub_public_key_hex)?),
        hub_tls_ca_pem_b64: hub_ca
            .map(read_ca_pem_b64)
            .transpose()
            .context("read --hub-ca PEM")?,
        join_receipt_hash: Some(join.receipt.join_receipt_hash),
    };
    if creds.join_receipt_hash().is_none() {
        anyhow::bail!("federation.join receipt missing join_receipt_hash");
    }
    persist_join_credentials(renderer, creds, peer_hub, "cli.join.ura")
}

fn user_id_from_principal_ura(principal_ura: &str) -> Option<String> {
    let parsed = crate::core::ura::parse_ura(principal_ura).ok()?;
    if parsed.kind == crate::core::ura::URAKind::User {
        parsed.user_id().map(str::to_string)
    } else {
        None
    }
}

fn join_principal_enrollment_from_args(
    principal_ura: Option<&str>,
    enrollment_id: Option<&str>,
    proof_kind: Option<&str>,
    proof_ref: Option<&str>,
) -> anyhow::Result<
    Option<crate::daemon::federation::client::ability_contract::PrincipalEnrollmentProof>,
> {
    if enrollment_id.is_some() && (proof_kind.is_some() || proof_ref.is_some()) {
        anyhow::bail!(
            "--principal-enrollment-id cannot be combined with --principal-proof-kind or --principal-proof-ref"
        );
    }
    match (principal_ura, enrollment_id, proof_kind, proof_ref) {
        (None, None, None, None) => Ok(None),
        (Some(principal_ura), Some(enrollment_id), None, None) => {
            join_principal_enrollment_proof(principal_ura, "enrollment", enrollment_id)
        }
        (Some(principal_ura), None, Some(kind), Some(reference)) => {
            join_principal_enrollment_proof(principal_ura, kind, reference)
        }
        _ => anyhow::bail!(
            "--principal-ura plus either --principal-enrollment-id or the complete --principal-proof-kind/--principal-proof-ref pair must be supplied together"
        ),
    }
}

fn join_principal_enrollment_proof(
    principal_ura: &str,
    kind: &str,
    reference: &str,
) -> anyhow::Result<
    Option<crate::daemon::federation::client::ability_contract::PrincipalEnrollmentProof>,
> {
    let principal_ura = principal_ura.trim();
    let kind = kind.trim();
    let reference = reference.trim();
    if principal_ura.is_empty() || kind.is_empty() || reference.is_empty() {
        anyhow::bail!(
            "--principal-ura, --principal-proof-kind and --principal-proof-ref must not be empty"
        );
    }
    let identity = crate::core::identity::RuntimeIdentityUra::parse(principal_ura)
        .map_err(|err| anyhow::anyhow!("invalid --principal-ura `{principal_ura}`: {err}"))?;
    if identity.kind() != crate::core::ura::URAKind::User {
        anyhow::bail!("--principal-ura must identify a User URA");
    }
    Ok(Some(
        crate::daemon::federation::client::ability_contract::PrincipalEnrollmentProof {
            principal_ura: identity.into_string(),
            proof: crate::daemon::federation::client::ability_contract::PrincipalProofRef {
                kind: kind.to_string(),
                reference: reference.to_string(),
            },
        },
    ))
}

#[derive(Debug)]
struct UraJoinResult {
    receipt: crate::daemon::federation::client::ability_contract::JoinReceipt,
    hub_public_key_hex: String,
}

#[cfg(feature = "axon-pb")]
struct ProvisionalJoinSigner {
    provisional_ura: String,
    device_ura: String,
    public_key: ed25519_dalek::VerifyingKey,
}

#[cfg(feature = "axon-pb")]
#[async_trait::async_trait]
impl crate::daemon::identity::self_identity::CanonicalSigner for ProvisionalJoinSigner {
    fn owner_ura(&self) -> &str {
        &self.provisional_ura
    }

    async fn sign_canonical(
        &self,
        canonical_bytes: &[u8],
    ) -> Result<ed25519_dalek::Signature, crate::daemon::identity::self_identity::SelfIdentityError>
    {
        let device_ura = self.device_ura.clone();
        let public_key = self.public_key;
        let canonical_bytes = canonical_bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            let client = crate::daemon::identity::self_identity::KeyringClient::default_path();
            crate::daemon::identity::self_identity::SelfIdentity::sign_bound(
                &client,
                &device_ura,
                &public_key,
                &canonical_bytes,
            )
        })
        .await
        .map_err(|error| {
            crate::daemon::identity::self_identity::SelfIdentityError::Transport(format!(
                "provisional join signing worker terminated unexpectedly: {error}"
            ))
        })?
    }

    fn signing_public_key(
        &self,
    ) -> Result<
        ed25519_dalek::VerifyingKey,
        crate::daemon::identity::self_identity::SelfIdentityError,
    > {
        Ok(self.public_key)
    }
}

fn hex_public_key_to_b64(public_key_hex: &str) -> anyhow::Result<String> {
    let raw = hex::decode(public_key_hex.trim()).context("decode public key hex")?;
    if raw.len() != 32 {
        anyhow::bail!("public key hex must decode to 32 bytes, got {}", raw.len());
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(raw))
}

fn read_ca_pem_b64(path: &Path) -> anyhow::Result<String> {
    let pem = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(pem))
}

#[cfg(feature = "axon-pb")]
fn do_federation_join_and_resolve_hub_key(
    target: &HubUraTarget,
    membership_ura: &str,
    public_key_hex: &str,
    hub_ca: Option<&Path>,
    principal_enrollment: Option<
        crate::daemon::federation::client::ability_contract::PrincipalEnrollmentProof,
    >,
) -> anyhow::Result<UraJoinResult> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for hub URA join")?;
    runtime.block_on(do_federation_join_and_resolve_hub_key_async(
        target,
        membership_ura,
        public_key_hex,
        hub_ca,
        principal_enrollment,
    ))
}

#[cfg(not(feature = "axon-pb"))]
fn do_federation_join_and_resolve_hub_key(
    _target: &HubUraTarget,
    _membership_ura: &str,
    _public_key_hex: &str,
    _hub_ca: Option<&Path>,
    _principal_enrollment: Option<
        crate::daemon::federation::client::ability_contract::PrincipalEnrollmentProof,
    >,
) -> anyhow::Result<UraJoinResult> {
    Err(
        crate::support::platform::local_invoke::federation_capability_unsupported_error(
            "joining hub by URA",
        ),
    )
}

#[cfg(feature = "axon-pb")]
async fn do_federation_join_and_resolve_hub_key_async(
    target: &HubUraTarget,
    membership_ura: &str,
    public_key_hex: &str,
    hub_ca: Option<&Path>,
    principal_enrollment: Option<
        crate::daemon::federation::client::ability_contract::PrincipalEnrollmentProof,
    >,
) -> anyhow::Result<UraJoinResult> {
    use axon_sdk::pb::axon::v1::invocation_client::InvocationClient;

    let channel = connect_hub_invocation_channel(&target.hub_endpoint, hub_ca).await?;
    let mut client = InvocationClient::new(channel);

    let public_key = hex::decode(public_key_hex).context("decode device public key hex")?;
    let provisional_caller = crate::core::ura::provisional::provisional_ura_for_pubkey(&public_key);
    let public_key_bytes: [u8; 32] = public_key
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("device public key must be 32 bytes"))?;
    let public_key =
        ed25519_dalek::VerifyingKey::from_bytes(&public_key_bytes).context("decode device key")?;
    let provisional_signer = ProvisionalJoinSigner {
        provisional_ura: provisional_caller.clone(),
        device_ura: membership_ura.to_string(),
        public_key,
    };
    let join_args = crate::daemon::federation::client::ability_contract::JoinArgs {
        realm: target.realm.clone(),
        membership_ura: membership_ura.to_string(),
        public_key_hex: public_key_hex.to_string(),
        principal_enrollment,
    };
    let join_arguments =
        crate::daemon::federation::client::ability_contract::args_to_bytes(&join_args);
    let join_descriptor_ref =
        crate::daemon::axon_bridge::descriptor_ref::catalog_descriptor_ref_for_wire(
            &target.hub_ura,
            crate::daemon::ability::conformance::ABILITY_FEDERATION_JOIN,
            crate::daemon::ability::CallMode::Rpc,
        )
        .map_err(|err| anyhow::anyhow!("derive federation.join descriptor ref: {err}"))?;
    let join_request = crate::daemon::invocation::ProtoEnvelope::federation_join_genesis(
        provisional_caller,
        target.hub_ura.clone(),
        membership_ura.to_string(),
        crate::daemon::invocation::RootInvocationDerivationIssuer::fresh_root(),
    )?
    .signed_descriptor_ref_invoke_request_with_signer(
        crate::daemon::ability::conformance::ABILITY_FEDERATION_JOIN,
        join_descriptor_ref,
        join_arguments,
        &provisional_signer,
    )
    .await?;
    let join_response = client.invoke(join_request).await.map_err(|status| {
        anyhow::anyhow!(
            "hub rejected federation.join: code={:?} message={}",
            status.code(),
            status.message()
        )
    })?;
    let join_response = join_response.into_inner();
    crate::daemon::invocation::routing::remote_invoke::ensure_completed_invoke_response(
        "federation.join",
        &join_response,
    )?;
    let receipt: crate::daemon::federation::client::ability_contract::JoinReceipt =
        crate::daemon::federation::client::ability_contract::parse_receipt(&join_response.result)?;
    if receipt.realm != target.realm {
        anyhow::bail!(
            "federation.join receipt realm `{}` does not match requested realm `{}`",
            receipt.realm,
            target.realm
        );
    }
    if receipt.membership_ura != membership_ura {
        anyhow::bail!(
            "federation.join receipt membership `{}` does not match requested `{membership_ura}`",
            receipt.membership_ura
        );
    }

    let membership_device_id = membership_ura_device_id(membership_ura)?;
    let (_, signer, _) = ensure_device_runtime_identity(&target.realm, &membership_device_id)?;
    let resolve_arguments =
        crate::daemon::federation::wire_contract::ResolveKeyRequest::new(target.hub_ura.clone())
            .to_arguments_bytes()?;
    let subject = crate::core::ura::owner_ability_ura(
        &target.hub_ura,
        crate::daemon::ability::conformance::ABILITY_FEDERATION_RESOLVE_KEY,
    )
    .ok_or_else(|| anyhow::anyhow!("derive federation.resolve_key subject URA"))?;
    let descriptor_ref =
        crate::daemon::axon_bridge::descriptor_ref::catalog_descriptor_ref_for_wire(
            &target.hub_ura,
            crate::daemon::ability::conformance::ABILITY_FEDERATION_RESOLVE_KEY,
            crate::daemon::ability::CallMode::Rpc,
        )
        .map_err(|err| anyhow::anyhow!("derive federation.resolve_key descriptor ref: {err}"))?;
    let resolve_request = crate::daemon::invocation::ProtoEnvelope::from_target(
        membership_ura.to_string(),
        target.hub_ura.clone(),
        subject,
        crate::daemon::invocation::RootInvocationDerivationIssuer::fresh_root(),
    )?
    .signed_descriptor_ref_invoke_request(
        crate::daemon::ability::conformance::ABILITY_FEDERATION_RESOLVE_KEY,
        descriptor_ref,
        resolve_arguments,
        &signer,
    )?;
    let resolve_response = client.invoke(resolve_request).await.map_err(|status| {
        anyhow::anyhow!(
            "hub rejected federation.resolve_key: code={:?} message={}",
            status.code(),
            status.message()
        )
    })?;
    let resolve_response = resolve_response.into_inner();
    crate::daemon::invocation::routing::remote_invoke::ensure_completed_invoke_response(
        "federation.resolve_key",
        &resolve_response,
    )?;
    let resolved: crate::daemon::federation::client::ability_contract::ResolveKeyReceipt =
        crate::daemon::federation::client::ability_contract::parse_receipt(
            &resolve_response.result,
        )?;
    if resolved.public_key_hex.trim().is_empty() {
        anyhow::bail!("federation.resolve_key returned an empty hub public key");
    }

    Ok(UraJoinResult {
        receipt,
        hub_public_key_hex: resolved.public_key_hex,
    })
}

#[cfg(feature = "axon-pb")]
async fn connect_hub_invocation_channel(
    hub_endpoint: &str,
    hub_ca: Option<&Path>,
) -> anyhow::Result<tonic::transport::Channel> {
    use std::time::Duration;
    use tonic::transport::{ClientTlsConfig, Endpoint};

    let mut endpoint = Endpoint::from_shared(hub_endpoint.to_string())
        .with_context(|| format!("parse hub endpoint `{hub_endpoint}`"))?
        .connect_timeout(Duration::from_secs(10))
        .http2_keep_alive_interval(Duration::from_secs(5))
        .keep_alive_timeout(Duration::from_secs(10))
        .keep_alive_while_idle(true)
        .tcp_keepalive(Some(Duration::from_secs(15)));

    if let Some(ca_path) = hub_ca {
        let tls = crate::daemon::federation::client::pinned_tls_config(ca_path)
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        endpoint = endpoint
            .tls_config(tls)
            .with_context(|| format!("configure pinned TLS for `{hub_endpoint}`"))?;
    } else {
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new().with_native_roots())
            .with_context(|| format!("configure WebPKI TLS for `{hub_endpoint}`"))?;
    }

    endpoint
        .connect()
        .await
        .with_context(|| format!("connect hub Invocation endpoint `{hub_endpoint}`"))
}

#[cfg(feature = "axon-pb")]
fn membership_ura_device_id(membership_ura: &str) -> anyhow::Result<String> {
    let parsed = crate::core::ura::parse_ura(membership_ura)
        .map_err(|err| anyhow::anyhow!("invalid membership URA `{membership_ura}`: {err}"))?;
    parsed
        .device_id()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("membership URA `{membership_ura}` is not a device URA"))
}

fn render_pairing_summary(title: &str, creds: &config::Credentials, peer_hub: Option<&str>) {
    // Final summary block — same `kv_section` styling as `start`
    // so the two commands look like siblings, not strangers.
    output::success(title);
    let realm = creds.realm.clone();
    let mut rows = vec![
        ("node_id", creds.node_id.as_str()),
        ("hub_endpoint", creds.hub_endpoint.as_str()),
        ("realm", realm.as_str()),
    ];
    let peer_hub_value;
    if let Some(peer) = peer_hub {
        peer_hub_value = peer.to_string();
        rows.push(("peer_hub", peer_hub_value.as_str()));
    }
    output::kv_section(&rows);
    eprintln!();
}

/// Walk through the join-time side effects under a live stage renderer.
/// Pairing, credential persistence, local runtime authority wiring, and
/// key custody are required lifecycle transitions: a failed transition renders
/// `stage_failed` and aborts the join instead of producing credentials that the
/// daemon cannot later admit.
///
/// Returns the resolved `Credentials` so the caller can render the
/// summary block.
fn run_join_stages(
    token: &str,
    validate_base: &str,
    has_explicit_hub_api_override: bool,
    peer_hub: Option<&str>,
) -> anyhow::Result<config::Credentials> {
    let mut renderer = crate::cli::presentation::stage::StageRenderer::new();

    renderer.set_active("preflight");
    let preflight = match preflight_pairing_token(token, validate_base) {
        Ok(p) => {
            renderer.stage_ok("preflight");
            record_snapshot(JoinConnectionSnapshot::from_parts(
                JoinConnectionState::PairingTokenPreflighted,
                Some(JoinTransition::PreflightToken),
                p.realm.clone(),
                p.node_id.clone(),
                Some(validate_base.to_string()),
                "cli.join",
            ));
            p
        }
        Err(e) => {
            record_snapshot(JoinConnectionSnapshot::failed_from_parts(
                JoinFailureParts {
                    failure_code: JoinFailureCode::JoinFailedPreflight,
                    transition: JoinTransition::PreflightToken,
                    realm: String::new(),
                    node_id: String::new(),
                    hub_endpoint: Some(validate_base.to_string()),
                    message: e.to_string(),
                    retryable: false,
                    source: "cli.join".to_string(),
                },
            ));
            renderer.stage_failed("preflight", &format!("{e}"));
            renderer.finish();
            return Err(e);
        }
    };

    renderer.set_active("validate-token");
    let (device_public_key, key_service_startup) =
        match derive_device_public_key_hex_with_startup(&preflight.realm, &preflight.node_id) {
            Ok(public_key) => public_key,
            Err(error) => {
                renderer.stage_failed("validate-token", &error.to_string());
                renderer.finish();
                return Err(error);
            }
        };
    let _key_service_guard = JoinKeyServiceShutdownGuard::new(key_service_startup);
    let mut creds =
        match validate_pairing_token(token, validate_base, &preflight, &device_public_key) {
            Ok(c) => {
                renderer.stage_ok("validate-token");
                record_snapshot(JoinConnectionSnapshot::from_credentials(
                    JoinConnectionState::DeviceValidatedJoining,
                    Some(JoinTransition::ValidateToken),
                    &c,
                    "cli.join",
                ));
                c
            }
            Err(e) => {
                record_snapshot(JoinConnectionSnapshot::failed_from_parts(
                    JoinFailureParts {
                        failure_code: JoinFailureCode::JoinFailedValidate,
                        transition: JoinTransition::ValidateToken,
                        realm: preflight.realm.clone(),
                        node_id: preflight.node_id.clone(),
                        hub_endpoint: Some(validate_base.to_string()),
                        message: e.to_string(),
                        retryable: false,
                        source: "cli.join".to_string(),
                    },
                ));
                renderer.stage_failed("validate-token", &format!("{e}"));
                renderer.finish();
                return Err(e);
            }
        };
    let _ = rewrite_local_docker_session_endpoint(&mut creds, validate_base);
    creds.hub_api_base =
        persisted_hub_api_base_for_pairing(&creds, validate_base, has_explicit_hub_api_override);

    persist_join_credentials(renderer, creds, peer_hub, "cli.join")
}

fn persist_join_credentials(
    mut renderer: crate::cli::presentation::stage::StageRenderer,
    creds: config::Credentials,
    peer_hub: Option<&str>,
    source: &str,
) -> anyhow::Result<config::Credentials> {
    renderer.set_active("save-credentials");
    if let Err(e) = config::save_credentials(&creds) {
        record_snapshot(JoinConnectionSnapshot::failed_from_credentials(
            JoinFailureCode::JoinFailedValidate,
            JoinTransition::SaveCredentials,
            &creds,
            e.to_string(),
            false,
            source.to_string(),
        ));
        renderer.stage_failed("save-credentials", &format!("{e}"));
        renderer.finish();
        return Err(e);
    }
    renderer.stage_ok("save-credentials");
    record_snapshot(JoinConnectionSnapshot::from_credentials(
        JoinConnectionState::CredentialsSaved,
        Some(JoinTransition::SaveCredentials),
        &creds,
        source,
    ));

    run_required_join_stage(&mut renderer, "daemon-config", || {
        crate::daemon::persistence::daemon_config::ensure_minimal_device_config(&creds)
            .context("ensure daemon-config.toml for joined device")
    })?;

    run_required_join_stage(&mut renderer, "federated-peers", || {
        super::federation_wire::auto_wire_federated_peer_from_credentials(&creds, peer_hub)
            .context("wire federated peers for joined device")
    })?;

    run_required_join_stage(&mut renderer, "keyring", || {
        ensure_join_runtime_identity_custody(&creds, &KeyServiceJoinRuntimeIdentityCustody)
            .context("ensure joined runtime identity custody")
    })?;

    run_required_join_stage(&mut renderer, "realm-trust", || {
        super::federation_wire::auto_wire_self_realm_trust_from_credentials(&creds)
            .context("wire local realm trust for joined device")
    })?;
    record_snapshot(JoinConnectionSnapshot::from_credentials(
        JoinConnectionState::LocalTrustWired,
        Some(JoinTransition::WireLocalTrust),
        &creds,
        source,
    ));

    renderer.set_active("refresh-runtime");
    refresh_running_runtime_after_join(&creds);
    renderer.stage_ok("refresh-runtime");

    renderer.finish();
    Ok(creds)
}

fn run_required_join_stage<F>(
    renderer: &mut crate::cli::presentation::stage::StageRenderer,
    name: &'static str,
    action: F,
) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    renderer.set_active(name);
    match action() {
        Ok(()) => {
            renderer.stage_ok(name);
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            renderer.stage_failed(name, &message);
            renderer.finish();
            Err(error).with_context(|| format!("join stage `{name}` failed"))
        }
    }
}

/// A running daemon loaded identity and authority state before this join.
/// Restart is the single supported transition into the newly paired state.
fn refresh_running_runtime_after_join(_creds: &config::Credentials) {
    if config::load().is_ok() {
        output::warn(
            "paired successfully, but a local easynet-daemon is already running. \
             Restart it with `easynet runtime stop && easynet runtime start` so it picks up the new credentials.",
        );
    }
}

/// Ensure every runtime identity implied by joined credentials exists in the
/// daemon custody service.
///
/// Bound-user credentials imply both a device signer and a managed User signer:
/// canonical SDK descriptor resolution signs as the User for user-scoped reads,
/// so letting join finish with only the device signer keeps an obsolete
/// "start later and maybe reconcile" compatibility window alive. Federation-
/// native device-only credentials remain explicitly unbound and must not mint a
/// placeholder User signer.
fn ensure_join_runtime_identity_custody(
    creds: &config::Credentials,
    custody: &dyn JoinRuntimeIdentityCustody,
) -> anyhow::Result<()> {
    custody.ensure_device(&creds.realm, &creds.node_id)?;
    match creds.runtime_user_binding()? {
        config::RuntimeUserBinding::Bound { user_ura } => custody.ensure_user(&user_ura),
        config::RuntimeUserBinding::Unbound { .. } => Ok(()),
    }
}

trait JoinRuntimeIdentityCustody {
    fn ensure_device(&self, realm: &str, node_id: &str) -> anyhow::Result<()>;
    fn ensure_user(&self, user_ura: &str) -> anyhow::Result<()>;
}

struct KeyServiceJoinRuntimeIdentityCustody;

impl JoinRuntimeIdentityCustody for KeyServiceJoinRuntimeIdentityCustody {
    fn ensure_device(&self, realm: &str, node_id: &str) -> anyhow::Result<()> {
        ensure_device_runtime_identity(realm, node_id).map(|_| ())
    }

    fn ensure_user(&self, user_ura: &str) -> anyhow::Result<()> {
        let client = crate::daemon::identity::self_identity::KeyringClient::default_path();
        crate::daemon::identity::self_identity::ensure_user_runtime_signing_identity(
            &client, user_ura,
        )
        .map(|_| ())
        .map_err(|error| {
            anyhow::anyhow!("ensure managed User runtime signer `{user_ura}`: {error}")
        })
    }
}

/// Ensure the joined device identity exists in the daemon custody service.
/// Join is fail-closed when the service cannot create or project the identity;
/// there is no deterministic private-key fallback.
fn ensure_device_runtime_identity(
    realm: &str,
    node_id: &str,
) -> anyhow::Result<(
    String,
    crate::daemon::identity::self_identity::KeyringClient,
    ed25519_dalek::VerifyingKey,
)> {
    let (primary_self, client, public_key, _) =
        ensure_device_runtime_identity_with_startup(realm, node_id)?;
    Ok((primary_self, client, public_key))
}

fn ensure_device_runtime_identity_with_startup(
    realm: &str,
    node_id: &str,
) -> anyhow::Result<(
    String,
    crate::daemon::identity::self_identity::KeyringClient,
    ed25519_dalek::VerifyingKey,
    crate::daemon::keyring::lifecycle::KeyServiceStartup,
)> {
    use crate::daemon::identity::self_identity::KeyringClient;

    let realm = realm.trim();
    let node_id = node_id.trim();
    if realm.is_empty() || node_id.is_empty() {
        anyhow::bail!("credentials missing realm or node_id");
    }
    let primary_self = crate::core::ura::device_ura(realm, node_id);

    let client = KeyringClient::default_path();
    // Probe reachability with the constant-size health operation first. When the
    // daemon is already up (operator started it, or a prior join
    // spawned it) we go straight to `ensure`. When it is down we
    // auto-provision it below so the encrypted vault is the default
    // posture rather than something only `dev-backend.sh` sets up.
    let startup = if client.health().is_err() {
        crate::daemon::keyring::lifecycle::ensure_key_service_running()?
    } else {
        crate::daemon::keyring::lifecycle::KeyServiceStartup::Attached
    };

    let public_key = client
        .ensure(&primary_self)
        .map_err(|error| anyhow::anyhow!("ensure runtime identity: {error}"))?;
    Ok((primary_self, client, public_key, startup))
}

fn validate_token_format(token: &str) -> anyhow::Result<()> {
    if token.len() < 8 {
        anyhow::bail!(
            "invalid pairing token: too short (minimum 8 characters, got {})",
            token.len()
        );
    }
    if token.len() > 256 {
        anyhow::bail!(
            "invalid pairing token: too long (maximum 256 characters, got {})",
            token.len()
        );
    }
    // Accept hex, alphanumeric, dashes, and underscores (covers hex tokens, UUIDs, base64url).
    if !token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "invalid pairing token: must contain only alphanumeric characters, dashes, or underscores"
        );
    }
    Ok(())
}

fn pairing_status_error_message(code: u16, body: &str) -> String {
    match code {
        404 => "pairing token expired or already used — create a new token from the Hub dashboard"
            .into(),
        409 => "device already paired — run 'easynet reset' first to un-pair, then retry".into(),
        _ => format!("Hub rejected pairing (HTTP {code}): {body}"),
    }
}

fn looks_like_plain_http_tls_mismatch(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("invalidcontenttype")
        || lower.contains("corrupt message")
        || (lower.contains("tls") && lower.contains("invalid content type"))
}

fn pairing_transport_error_message(base: &str, error: &dyn std::fmt::Display) -> String {
    let base = base.trim_end_matches('/');
    let error = error.to_string();
    let mut message = format!("cannot reach Hub at {base}: {error}");

    if let Some(parts) = parse_url_endpoint(base) {
        if parts.scheme.eq_ignore_ascii_case("https")
            && is_loopback_or_localhost(&parts.host)
            && looks_like_plain_http_tls_mismatch(&error)
        {
            let http_base = format!(
                "http://{}{}",
                format_authority(&parts.host, parts.port.as_deref()),
                parts.suffix
            );
            message.push_str(
                "\n  The URL uses https://, but the local Hub API appears to be speaking plain HTTP.",
            );
            message.push_str(&format!("\n  Retry with `--hub {http_base}`."));
            return message;
        }
    }

    message.push_str("\n  Check your network connection and Hub URL.");
    message
}

fn validate_pairing_response(
    envelope: PairingCredentialEnvelope,
) -> anyhow::Result<PairingCredentialEnvelope> {
    if envelope.node_id.is_empty() {
        anyhow::bail!("pairing response missing node_id");
    }
    if envelope.credential_token.is_empty() {
        anyhow::bail!("pairing response missing credential_token");
    }
    if envelope.hub_endpoint.is_empty() {
        anyhow::bail!("pairing response missing hub_endpoint");
    }
    if envelope.realm.is_empty() {
        anyhow::bail!("pairing response missing realm");
    }
    if envelope
        .username
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_none()
    {
        anyhow::bail!("pairing response missing username");
    }
    let user_id = envelope
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("pairing response missing user_id"))?;
    if crate::core::identity::is_all_zero_principal_id(user_id) {
        anyhow::bail!("pairing response carries all-zero user_id");
    }
    Ok(envelope)
}

fn credentials_from_pairing_contract(envelope: PairingCredentialEnvelope) -> config::Credentials {
    config::Credentials {
        node_id: envelope.node_id,
        credential_token: envelope.credential_token,
        hub_endpoint: envelope.hub_endpoint,
        realm: envelope.realm,
        deploy_signature: envelope.deploy_signature,
        hub_api_base: None,
        username: envelope.username.map(|v| v.trim().to_string()),
        user_id: envelope.user_id.map(|v| v.trim().to_string()),
        hub_pubkey_b64: None,
        hub_tls_ca_pem_b64: None,
        join_receipt_hash: None,
    }
}

/// Pick the REST-API base URL the pairing-token validation call
/// should hit. Operators commonly run a self-hosted Hub where the
/// user-facing portal (`--hub`) and the REST API (`--hub-api`)
/// live on different hosts/ports — e.g. portal at
/// `https://easynet.run`, REST API at `http://localhost:18080`.
/// Without preferring `--hub-api` when set, the validation call
/// hits the portal URL, gets a 404, and surfaces as "pairing
/// token expired or already used" — a misleading error mode.
fn pick_validate_base(hub: &str, hub_api_override: Option<&str>) -> String {
    hub_api_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| hub.to_string())
}

fn persisted_hub_api_base_for_pairing(
    creds: &config::Credentials,
    validate_base: &str,
    explicit_override: bool,
) -> Option<String> {
    let normalized = validate_base.trim_end_matches('/').to_string();
    if normalized.is_empty() {
        return None;
    }
    if explicit_override || normalized != creds.api_base() {
        return Some(normalized);
    }
    None
}

#[derive(Debug, PartialEq, Eq)]
struct UrlEndpointParts {
    scheme: String,
    host: String,
    port: Option<String>,
    suffix: String,
}

fn parse_url_endpoint(value: &str) -> Option<UrlEndpointParts> {
    let (scheme, rest) = value.trim().split_once("://")?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let suffix = rest[authority_end..].to_string();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, hp)| hp);
    if host_port.is_empty() {
        return None;
    }

    let (host, port) = if let Some(after_bracket) = host_port.strip_prefix('[') {
        let bracket_end = after_bracket.find(']')?;
        let host = after_bracket[..bracket_end].to_string();
        let tail = &after_bracket[bracket_end + 1..];
        let port = tail
            .strip_prefix(':')
            .filter(|p| !p.is_empty())
            .map(str::to_string);
        (host, port)
    } else if let Some((host, port)) = host_port.rsplit_once(':') {
        if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            (host.to_string(), Some(port.to_string()))
        } else {
            (host_port.to_string(), None)
        }
    } else {
        (host_port.to_string(), None)
    };

    Some(UrlEndpointParts {
        scheme: scheme.to_string(),
        host,
        port,
        suffix,
    })
}

fn format_authority(host: &str, port: Option<&str>) -> String {
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

fn is_loopback_or_localhost(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    lower == "localhost" || lower == "::1" || lower.starts_with("127.")
}

fn is_internal_authority_transport_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    lower == "authority" || lower == "hub" || lower.starts_with("hub-")
}

fn rewrite_local_docker_session_endpoint(
    creds: &mut config::Credentials,
    validate_base: &str,
) -> bool {
    let Some(validate_parts) = parse_url_endpoint(validate_base) else {
        return false;
    };
    if !is_loopback_or_localhost(&validate_parts.host) {
        return false;
    }

    let Some(session_parts) = parse_url_endpoint(&creds.hub_endpoint) else {
        return false;
    };
    if !is_internal_authority_transport_host(&session_parts.host) {
        return false;
    }

    let rewritten = format!(
        "{}://{}{}",
        session_parts.scheme,
        format_authority(&validate_parts.host, session_parts.port.as_deref()),
        session_parts.suffix
    );
    if rewritten == creds.hub_endpoint {
        return false;
    }
    creds.hub_endpoint = rewritten;
    true
}

fn preflight_pairing_token(token: &str, hub_base: &str) -> anyhow::Result<PairingPreflight> {
    let base = hub_base.trim_end_matches('/');
    let install_id = config::load_or_create_install_id().ok();
    let url = match install_id.as_deref() {
        Some(id) if !id.is_empty() => format!(
            "{base}/api/v1/devices/pairing/{token}/preflight?install_id={}",
            urlencoding::encode(id)
        ),
        _ => format!("{base}/api/v1/devices/pairing/{token}/preflight"),
    };

    let resp = match ureq::get(&url)
        .timeout(std::time::Duration::from_secs(30))
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            anyhow::bail!("{}", pairing_status_error_message(code, &body));
        }
        Err(ureq::Error::Transport(e)) => {
            anyhow::bail!("{}", pairing_transport_error_message(base, &e));
        }
    };

    let preflight: PairingPreflight = resp.into_json().map_err(|e| {
        anyhow::Error::from(e).context(
            "Hub returned an unreadable pairing preflight response — the Hub is likely on an \
             incompatible version, or a proxy rewrote the response. Verify the Hub URL and \
             that CLI + Hub versions match; re-run with a fresh pairing token if so.",
        )
    })?;
    if preflight.realm.is_empty() {
        anyhow::bail!("pairing preflight response missing realm");
    }
    if preflight.node_id.is_empty() {
        anyhow::bail!("pairing preflight response missing node_id");
    }
    Ok(preflight)
}

fn validate_pairing_token(
    token: &str,
    hub_base: &str,
    preflight: &PairingPreflight,
    device_public_key: &str,
) -> anyhow::Result<config::Credentials> {
    let payload = build_validate_pairing_payload(preflight, device_public_key.to_string());
    let base = hub_base.trim_end_matches('/');
    let url = format!("{base}/api/v1/devices/pairing/{token}/validate");

    let resp = match ureq::post(&url)
        .timeout(std::time::Duration::from_secs(30))
        .send_json(&payload)
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            anyhow::bail!("{}", pairing_status_error_message(code, &body));
        }
        Err(ureq::Error::Transport(e)) => {
            anyhow::bail!("{}", pairing_transport_error_message(base, &e));
        }
    };

    // The Hub's pairing endpoint is a versioned REST contract (see the
    // Hub's OpenAPI spec under /api/v1/devices/pairing). If `into_json`
    // fails, the bytes we got back are either not JSON at all (a proxy
    // inserted an HTML error page, a middlebox rewrote the response) or
    // the JSON shape no longer matches the product pairing contract
    // (the CLI and Hub are on incompatible versions). Either way, the underlying
    // serde error is noise to an operator — they need to know *what to
    // do*, not which field's tag didn't match. We keep the raw cause in
    // the error chain via `context`, so `--verbose` / log scrapers still
    // surface the full detail, while the top-line stays operator-friendly.
    let envelope: PairingCredentialEnvelope = resp.into_json().map_err(|e| {
        anyhow::Error::from(e).context(
            "Hub returned an unreadable pairing response — the Hub is likely on an \
             incompatible version, or a proxy rewrote the response. Verify the Hub URL \
             and that CLI + Hub versions match; re-run with a fresh pairing token if so.",
        )
    })?;

    let envelope = validate_pairing_response(envelope)?;
    if envelope.node_id != preflight.node_id {
        anyhow::bail!(
            "Hub returned node_id {} but pairing preflight reserved {}; aborting to avoid \
             booting with mismatched identity",
            envelope.node_id,
            preflight.node_id
        );
    }
    if envelope.realm != preflight.realm {
        anyhow::bail!(
            "Hub returned realm {} but pairing preflight reserved {}; aborting to avoid \
             deriving credentials under the wrong realm",
            envelope.realm,
            preflight.realm
        );
    }
    // Cross-machine cold-start: persist the hub's signing and TLS trust
    // material from preflight onto the in-memory and on-disk credentials so
    // trust synchronization can populate `realm-trust.toml` plus any local
    // pinned CA file without needing on-host access to hub-local files.
    let mut creds = credentials_from_pairing_contract(envelope);
    if !preflight.hub_public_key_b64.trim().is_empty() {
        creds.hub_pubkey_b64 = Some(preflight.hub_public_key_b64.trim().to_string());
    }
    if !preflight.hub_tls_ca_pem_b64.trim().is_empty() {
        creds.hub_tls_ca_pem_b64 = Some(preflight.hub_tls_ca_pem_b64.trim().to_string());
    }
    Ok(creds)
}

fn build_validate_pairing_payload(
    preflight: &PairingPreflight,
    device_public_key: String,
) -> ValidatePairingPayload {
    ValidatePairingPayload {
        info: sysinfo::collect_system_info(),
        node_id: preflight.node_id.clone(),
        device_public_key,
        // Best-effort: a failure to read/persist the install id (e.g. an
        // unwritable state dir) must not block pairing — the hub then simply
        // mints a fresh node_id as before.
        install_id: config::load_or_create_install_id().ok(),
    }
}

fn derive_device_public_key_hex_with_startup(
    realm: &str,
    node_id: &str,
) -> anyhow::Result<(String, crate::daemon::keyring::lifecycle::KeyServiceStartup)> {
    let (_, _, public_key, startup) = ensure_device_runtime_identity_with_startup(realm, node_id)?;
    Ok((hex::encode(public_key.to_bytes()), startup))
}

struct JoinKeyServiceShutdownGuard {
    should_shutdown: bool,
}

impl JoinKeyServiceShutdownGuard {
    fn new(startup: crate::daemon::keyring::lifecycle::KeyServiceStartup) -> Self {
        Self {
            should_shutdown: startup
                == crate::daemon::keyring::lifecycle::KeyServiceStartup::Spawned,
        }
    }
}

impl Drop for JoinKeyServiceShutdownGuard {
    fn drop(&mut self) {
        if self.should_shutdown {
            let _ = crate::daemon::keyring::lifecycle::shutdown_bootstrap_key_service();
        }
    }
}

// Deregister the previously-paired device from the hub before a re-pair
// overwrites its credentials. Mirrors `reset.rs`'s revoke helper (the
// `federation.revoke` surface is proto-gated, so both feature arms must
// compile).
#[cfg(feature = "axon-pb")]
fn invoke_federation_revoke_for_rejoin(device_ura: &str) -> anyhow::Result<()> {
    crate::daemon::invocation::routing::remote_invoke::invoke_federation_revoke(
        device_ura,
        "device-rejoin",
        device_ura,
    )
}

#[cfg(not(feature = "axon-pb"))]
fn invoke_federation_revoke_for_rejoin(_device_ura: &str) -> anyhow::Result<()> {
    Err(
        crate::support::platform::local_invoke::federation_capability_unsupported_error(
            "deregistering the old device on re-pair",
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::net::TcpListener;

    #[derive(Default)]
    struct RecordingJoinRuntimeIdentityCustody {
        devices: RefCell<Vec<(String, String)>>,
        users: RefCell<Vec<String>>,
        fail_user: Option<&'static str>,
    }

    impl RecordingJoinRuntimeIdentityCustody {
        fn failing_user(message: &'static str) -> Self {
            Self {
                fail_user: Some(message),
                ..Self::default()
            }
        }
    }

    impl JoinRuntimeIdentityCustody for RecordingJoinRuntimeIdentityCustody {
        fn ensure_device(&self, realm: &str, node_id: &str) -> anyhow::Result<()> {
            self.devices
                .borrow_mut()
                .push((realm.to_string(), node_id.to_string()));
            Ok(())
        }

        fn ensure_user(&self, user_ura: &str) -> anyhow::Result<()> {
            self.users.borrow_mut().push(user_ura.to_string());
            if let Some(message) = self.fail_user {
                anyhow::bail!("{message}");
            }
            Ok(())
        }
    }

    fn join_identity_test_credentials(
        user_id: Option<&str>,
        join_receipt_hash: Option<&str>,
    ) -> config::Credentials {
        config::Credentials {
            node_id: "dev-one".to_string(),
            credential_token: if join_receipt_hash.is_some() {
                String::new()
            } else {
                "credential-token".to_string()
            },
            hub_endpoint: "https://hub.acme.internal:50443".to_string(),
            realm: "acme".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: user_id.map(|_| "silan".to_string()),
            user_id: user_id.map(str::to_string),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: join_receipt_hash.map(str::to_string),
        }
    }

    #[test]
    fn token_format_accepts_alnum_dash_underscore() {
        for token in ["abc12345", "A_B-C_99", "token_2026_04"] {
            assert!(
                validate_token_format(token).is_ok(),
                "expected valid token: {token}"
            );
        }
    }

    #[test]
    fn token_format_rejects_short_long_and_invalid_chars() {
        assert!(validate_token_format("short").is_err());
        assert!(validate_token_format(&"a".repeat(257)).is_err());
        assert!(validate_token_format("bad token").is_err());
        assert!(validate_token_format("bad/token").is_err());
    }

    #[test]
    fn join_identity_custody_ensures_bound_user_signer() {
        let creds = join_identity_test_credentials(Some("usr_silan"), None);
        let custody = RecordingJoinRuntimeIdentityCustody::default();

        ensure_join_runtime_identity_custody(&creds, &custody)
            .expect("bound join must ensure device and User custody");

        assert_eq!(
            custody.devices.borrow().as_slice(),
            &[("acme".to_string(), "dev-one".to_string())]
        );
        assert_eq!(
            custody.users.borrow().as_slice(),
            &["easynet:///r/acme/user/usr_silan".to_string()]
        );
    }

    #[test]
    fn join_identity_custody_leaves_federation_native_device_only_unbound() {
        let creds = join_identity_test_credentials(None, Some(&"a".repeat(64)));
        let custody = RecordingJoinRuntimeIdentityCustody::default();

        ensure_join_runtime_identity_custody(&creds, &custody)
            .expect("device-only federation-native join must not mint a User signer");

        assert_eq!(
            custody.devices.borrow().as_slice(),
            &[("acme".to_string(), "dev-one".to_string())]
        );
        assert!(
            custody.users.borrow().is_empty(),
            "unbound federation-native credentials must not synthesize User custody"
        );
    }

    #[test]
    fn join_identity_custody_fails_closed_when_bound_user_signer_fails() {
        let creds = join_identity_test_credentials(Some("usr_silan"), None);
        let custody =
            RecordingJoinRuntimeIdentityCustody::failing_user("managed User signer unavailable");

        let error = ensure_join_runtime_identity_custody(&creds, &custody)
            .expect_err("bound join must fail when User signer custody fails");

        assert!(
            error
                .to_string()
                .contains("managed User signer unavailable"),
            "error must preserve User signer custody failure: {error:#}"
        );
        assert_eq!(
            custody.users.borrow().as_slice(),
            &["easynet:///r/acme/user/usr_silan".to_string()]
        );
    }

    #[test]
    fn existing_join_idempotency_requires_profile_account_owner() {
        let existing = config::Credentials {
            node_id: "dev-one".to_string(),
            credential_token: "credential-token".to_string(),
            hub_endpoint: "https://hub.acme.internal".to_string(),
            realm: "acme".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("silan".to_string()),
            user_id: Some("usr_silan".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        let silan_profile = profile::ProfileEntry {
            profile_name: "silan@acme".to_string(),
            realm_alias: "acme".to_string(),
            realm_id: None,
            issuer: "https://hub.acme.internal".to_string(),
            login_hint: Some("silan".to_string()),
            subject: Some("usr_silan".to_string()),
            credential_ref: None,
            trust_anchor: None,
            account_session: profile::ProfileAccountSessionState::Authenticated,
            device_membership: "enrolled".to_string(),
        };
        let admin_profile = profile::ProfileEntry {
            profile_name: "admin@acme".to_string(),
            login_hint: Some("admin".to_string()),
            subject: Some("usr_admin".to_string()),
            ..silan_profile.clone()
        };

        assert!(existing_credentials_match_profile(
            &existing,
            &silan_profile
        ));
        assert!(!existing_credentials_match_profile(
            &existing,
            &admin_profile
        ));
    }

    #[test]
    fn hub_ura_target_derives_endpoint_for_public_realm() {
        let target = HubUraTarget::parse("easynet:///r/easynet.run/authority", None, None)
            .expect("public realm");
        assert_eq!(target.realm, "easynet.run");
        assert_eq!(target.hub_endpoint, "https://easynet.run:50443");
    }

    #[test]
    fn hub_ura_target_requires_ca_for_private_realm() {
        let err = HubUraTarget::parse("easynet:///r/localhost/authority", None, None)
            .expect_err("localhost requires CA");
        assert!(err.to_string().contains("--hub-ca"));

        let target = HubUraTarget::parse(
            "easynet:///r/localhost/authority",
            Some(55443),
            Some(Path::new("/tmp/authority-ca.pem")),
        )
        .expect("private realm with CA");
        assert_eq!(target.hub_endpoint, "https://127.0.0.1:55443");
    }

    #[test]
    fn pairing_status_error_message_maps_common_cases() {
        assert!(pairing_status_error_message(404, "x").contains("expired or already used"));
        assert!(pairing_status_error_message(409, "x").contains("device already paired"));
        assert_eq!(
            pairing_status_error_message(500, "oops"),
            "Hub rejected pairing (HTTP 500): oops"
        );
    }

    #[test]
    fn pairing_transport_error_message_explains_local_https_plain_http_mismatch() {
        let error =
            "tls connection init failed: received corrupt message of type InvalidContentType";
        let message = pairing_transport_error_message("https://localhost:8080", &error);

        assert!(message.contains("cannot reach Hub at https://localhost:8080"));
        assert!(message.contains("appears to be speaking plain HTTP"));
        assert!(message.contains("--hub http://localhost:8080"));
    }

    #[test]
    fn pairing_transport_error_message_keeps_generic_network_hint_for_other_failures() {
        let message =
            pairing_transport_error_message("https://hub.acme.internal", &"connection refused");

        assert!(message.contains("cannot reach Hub at https://hub.acme.internal"));
        assert!(message.contains("Check your network connection and Hub URL"));
        assert!(!message.contains("plain HTTP"));
    }

    #[test]
    fn validate_pairing_response_rejects_empty_node_id() {
        let envelope = PairingCredentialEnvelope {
            node_id: String::new(),
            credential_token: "cred".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            ..Default::default()
        };
        let err = validate_pairing_response(envelope).expect_err("missing node_id must fail");
        assert!(err.to_string().contains("missing node_id"));
    }

    #[test]
    fn credentials_from_pairing_contract_projects_product_credentials() {
        let envelope = PairingCredentialEnvelope {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            ..Default::default()
        };
        let creds = credentials_from_pairing_contract(envelope);
        assert_eq!(creds.node_id, "node");
        assert_eq!(creds.realm, "tenant");
        assert_eq!(creds.username.as_deref(), Some("alice"));
        assert_eq!(creds.user_id.as_deref(), Some("user-alice"));
    }

    #[test]
    fn validate_pairing_response_rejects_missing_username() {
        let envelope = PairingCredentialEnvelope {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            username: None,
            user_id: Some("user-alice".into()),
            ..Default::default()
        };
        let err = validate_pairing_response(envelope).expect_err("missing username must fail");
        assert!(err.to_string().contains("missing username"));
    }

    #[test]
    fn validate_pairing_response_rejects_missing_user_id() {
        let envelope = PairingCredentialEnvelope {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            username: Some("alice".into()),
            user_id: None,
            ..Default::default()
        };
        let err = validate_pairing_response(envelope).expect_err("missing user_id must fail");
        assert!(err.to_string().contains("missing user_id"));
    }

    #[test]
    fn validate_pairing_response_rejects_all_zero_user_before_credentials_projection() {
        let envelope = PairingCredentialEnvelope {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            username: Some("alice".into()),
            user_id: Some("00000000-0000-0000-0000-000000000000".into()),
            ..Default::default()
        };
        let err =
            validate_pairing_response(envelope).expect_err("all-zero user_id must fail at pairing");
        assert!(err.to_string().contains("all-zero user_id"));
    }

    #[test]
    fn pick_validate_base_prefers_hub_api_when_set() {
        let chosen = pick_validate_base("https://easynet.run", Some("http://localhost:18080"));
        assert_eq!(chosen, "http://localhost:18080");
    }

    #[test]
    fn pick_validate_base_falls_back_to_hub_when_api_unset() {
        let chosen = pick_validate_base("https://easynet.run", None);
        assert_eq!(chosen, "https://easynet.run");
    }

    #[test]
    fn persisted_hub_api_base_keeps_explicit_override() {
        let creds = config::Credentials {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "https://authority:50443".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        let persisted = persisted_hub_api_base_for_pairing(&creds, "http://127.0.0.1:8080/", true);
        assert_eq!(persisted.as_deref(), Some("http://127.0.0.1:8080"));
    }

    #[test]
    fn persisted_hub_api_base_keeps_validate_base_when_session_endpoint_is_internal() {
        let creds = config::Credentials {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "https://authority:50443".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        let persisted = persisted_hub_api_base_for_pairing(&creds, "http://127.0.0.1:8080", false);
        assert_eq!(persisted.as_deref(), Some("http://127.0.0.1:8080"));
    }

    #[test]
    fn persisted_hub_api_base_omits_default_when_it_matches_derived_api_base() {
        let creds = config::Credentials {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "https://easynet.run:50443".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        let persisted = persisted_hub_api_base_for_pairing(&creds, "https://easynet.run", false);
        assert_eq!(persisted, None);
    }

    #[test]
    fn rewrite_local_docker_session_endpoint_uses_loopback_api_host() {
        let mut creds = config::Credentials {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "https://authority:50443".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        assert!(rewrite_local_docker_session_endpoint(
            &mut creds,
            "http://127.0.0.1:8080"
        ));
        assert_eq!(creds.hub_endpoint, "https://127.0.0.1:50443");
    }

    #[test]
    fn rewrite_local_docker_session_endpoint_keeps_container_to_container_join() {
        let mut creds = config::Credentials {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "https://authority:50443".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        assert!(!rewrite_local_docker_session_endpoint(
            &mut creds,
            "http://authority:8080"
        ));
        assert_eq!(creds.hub_endpoint, "https://authority:50443");
    }

    #[test]
    fn rewrite_local_docker_session_endpoint_keeps_public_session_endpoint() {
        let mut creds = config::Credentials {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "https://easynet.run:50443".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        assert!(!rewrite_local_docker_session_endpoint(
            &mut creds,
            "http://127.0.0.1:8080"
        ));
        assert_eq!(creds.hub_endpoint, "https://easynet.run:50443");
    }

    #[test]
    fn pairing_preflight_accepts_current_realm_schema() {
        let preflight: PairingPreflight = serde_json::from_value(serde_json::json!({
            "realm": "tenant-a",
            "node_id": "en-test-node",
            "hub_public_key_b64": "",
            "hub_tls_ca_pem_b64": "",
            "hub_agent_ura": crate::core::ura::hub_ura("tenant-a")
        }))
        .expect("current preflight schema");

        assert_eq!(preflight.realm, "tenant-a");
        assert_eq!(preflight.node_id, "en-test-node");
    }

    #[test]
    fn pairing_preflight_rejects_retired_tenant_id_alias() {
        let err = serde_json::from_value::<PairingPreflight>(serde_json::json!({
            "realm": "tenant-a",
            "tenant_id": "tenant-a",
            "node_id": "en-test-node"
        }))
        .expect_err("retired tenant_id must not be accepted");

        assert!(
            err.to_string().contains("tenant_id"),
            "error should name the retired field: {err}"
        );
    }

    #[test]
    fn build_validate_pairing_payload_carries_reserved_identity() {
        let preflight = PairingPreflight {
            realm: "tenant-a".into(),
            node_id: "en-test-node".into(),
            hub_public_key_b64: String::new(),
            hub_tls_ca_pem_b64: String::new(),
            _hub_agent_ura: String::new(),
        };
        let payload = build_validate_pairing_payload(&preflight, "ab".repeat(32));
        assert_eq!(payload.node_id, "en-test-node");
        assert_eq!(payload.device_public_key.len(), 64);
    }

    #[test]
    fn join_principal_enrollment_requires_complete_product_neutral_proof() {
        let err = join_principal_enrollment_from_args(
            Some("easynet:///r/tenant-a/user/alice"),
            None,
            Some("active_key"),
            None,
        )
        .expect_err("partial proof must fail");
        assert!(err.to_string().contains("must be supplied together"));
    }

    #[test]
    fn join_principal_enrollment_rejects_all_zero_user() {
        let error = join_principal_enrollment_from_args(
            Some("easynet:///r/tenant/user/00000000-0000-0000-0000-000000000000"),
            Some("enrollment-1"),
            None,
            None,
        )
        .expect_err("all-zero User enrollment must reject");

        assert!(error.to_string().contains("all-zero principal"));
    }

    #[test]
    fn join_principal_enrollment_lowers_without_product_account_fields() {
        let proof = join_principal_enrollment_from_args(
            Some("easynet:///r/tenant-a/user/alice"),
            None,
            Some("active_key"),
            Some("binding-1"),
        )
        .expect("proof")
        .expect("present");
        let value = serde_json::to_value(&proof).expect("json");

        assert_eq!(value["principal_ura"], "easynet:///r/tenant-a/user/alice");
        assert_eq!(value["proof"]["kind"], "active_key");
        assert_eq!(value["proof"]["reference"], "binding-1");
        assert!(value.get("username").is_none());
        assert!(value.get("user_id").is_none());
    }

    #[test]
    fn join_principal_enrollment_id_lowers_to_enrollment_proof() {
        let proof = join_principal_enrollment_from_args(
            Some(" easynet:///r/tenant-a/user/bob "),
            Some(" enrollment_1 "),
            None,
            None,
        )
        .expect("proof")
        .expect("present");
        let value = serde_json::to_value(&proof).expect("json");

        assert_eq!(value["principal_ura"], "easynet:///r/tenant-a/user/bob");
        assert_eq!(value["proof"]["kind"], "enrollment");
        assert_eq!(value["proof"]["reference"], "enrollment_1");
        assert!(value.get("username").is_none());
        assert!(value.get("user_id").is_none());
        assert!(value.get("account_id").is_none());
    }

    #[test]
    fn hub_ura_join_credentials_can_derive_local_user_id_from_principal_ura() {
        assert_eq!(
            user_id_from_principal_ura("easynet:///r/tenant-a/user/alice").as_deref(),
            Some("alice")
        );
        assert_eq!(
            user_id_from_principal_ura("easynet:///r/tenant-a/authority"),
            None
        );
    }

    #[test]
    fn join_principal_enrollment_id_rejects_generic_proof_mix() {
        let err = join_principal_enrollment_from_args(
            Some("easynet:///r/tenant-a/user/bob"),
            Some("enrollment_1"),
            Some("enrollment"),
            None,
        )
        .expect_err("mixed shorthand and generic proof must fail");

        assert!(err
            .to_string()
            .contains("cannot be combined with --principal-proof-kind"));
    }

    #[test]
    fn preflight_pairing_token_surfaces_transport_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let addr = listener.local_addr().expect("local_addr");
        drop(listener);
        let base = format!("http://{}", addr);
        let err = preflight_pairing_token("token_1234", &base)
            .expect_err("transport failure should error");
        assert!(err.to_string().contains("cannot reach Hub"));
    }

    #[test]
    fn validate_pairing_token_surfaces_transport_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let addr = listener.local_addr().expect("local_addr");
        drop(listener);
        let base = format!("http://{}", addr);
        let preflight = PairingPreflight {
            realm: "tenant-a".into(),
            node_id: "en-test-node".into(),
            hub_public_key_b64: String::new(),
            hub_tls_ca_pem_b64: String::new(),
            _hub_agent_ura: String::new(),
        };
        let err = validate_pairing_token("token_1234", &base, &preflight, &"ab".repeat(32))
            .expect_err("transport failure should error");
        assert!(err.to_string().contains("cannot reach Hub"));
    }
}

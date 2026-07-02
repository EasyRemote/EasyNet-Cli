// EasyNet CLI
// ===========
//
// File: src/cli/join.rs
// Description: `easynet device join <token>` — pair this device with EasyNet Hub via a one-time
//              pairing token, establishing a persistent trust relationship.
//
// Protocol Responsibility:
// - Validates a one-time pairing token (32-64 hex chars) against the Hub REST API.
// - POST /api/v1/devices/pairing/{token}/validate with device sysinfo (hostname, OS, arch).
// - Receives and persists: node_id, credential_token, hub_endpoint, realm, deploy_signature.
// - This is the ONLY command that creates ~/.easynet/credentials.json; all other commands consume it.
//
// Implementation Approach:
// - Synchronous HTTP via ureq with 30s timeout. No retry — pairing tokens are one-shot.
// - Token format validation before network call to fail fast on typos.
// - Supports --hub for self-hosted Hubs (defaults to https://easynet.run).
//
// Usage Contract:
// - Run once per device. Re-running overwrites existing credentials (re-pair).
// - Requires network access to Hub REST API (not the gRPC Axon endpoint).
// - After join, run `easynet connect` to start the device agent.
//
// Architectural Position:
// - Entry point of the device lifecycle: join → start → (heartbeat loop) → stop → reset.
// - Bridges the Hub's web-based pairing flow with the CLI's local credential store.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use base64::Engine as _;
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::persistence::config;
use crate::runtime::join_connection_state::{
    record_snapshot, JoinConnectionSnapshot, JoinConnectionState, JoinFailureCode,
    JoinFailureParts, JoinTransition,
};
use crate::support::{output, sysinfo};

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
    /// needing on-host access to `~/.easynet-hub/<realm>/
    /// identity.json`. Empty on pre-v4.1.4 hubs (legacy fallback
    /// path reads the on-disk identity file when same-host).
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
    /// One-time pairing token or hub URA (easynet:///r/<realm>/hub).
    pub token: String,
    /// Hub API base URL for self-hosted Hubs.
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
    // The doc-comment below is a single paragraph on purpose. clap
    // switches `--help` into multi-paragraph "long help" mode the
    // moment ANY arg's doc-comment has a blank line in it — every
    // other arg in the same struct then renders with extra spacing
    // around it. The detailed rationale for `--peer-hub` (Hub
    // pairing response carries the backend Axon endpoint, not the
    // peer daemon's TLS listener; multi-hub deployments diverge)
    // lives in docs/spec/RFC-002 §federation.forward_invoke and in
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

pub fn run(args: JoinArgs) -> anyhow::Result<()> {
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
        let old_device_ura = crate::ura::device_ura(existing.realm_str(), &existing.node_id);
        match invoke_federation_revoke_for_rejoin(&old_device_ura) {
            Ok(()) => output::info("Old device deregistered with the hub (federation.revoke)."),
            Err(e) => output::warn(&format!(
                "Could not deregister the old device (continuing with re-pair): {e}"
            )),
        }
    }

    let target = args.token.trim().to_string();
    let peer_hub = args.peer_hub.as_deref();
    let creds = if target.starts_with(crate::ura::URA_SCHEME) {
        run_ura_join_stages(&target, args.hub_port, args.hub_ca.as_deref(), peer_hub)?
    } else {
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

    finish_join(args.boot, &creds, peer_hub)
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
        let parsed = crate::ura::parse_ura(value)
            .map_err(|err| anyhow::anyhow!("invalid hub URA `{value}`: {err}"))?;
        if parsed.kind != crate::ura::URAKind::Hub {
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
) -> anyhow::Result<config::Credentials> {
    let mut renderer = super::presentation::stage::StageRenderer::new();

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
    let membership_ura = crate::ura::device_ura(&target.realm, &node_id);
    let public_key_hex = derive_device_public_key_hex(&target.realm, &node_id)?;

    renderer.set_active("federation-join");
    let join = match do_federation_join_and_resolve_hub_key(
        &target,
        &membership_ura,
        &public_key_hex,
        hub_ca,
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
        username: None,
        user_id: None,
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

#[derive(Debug)]
struct UraJoinResult {
    receipt: crate::runtime::federation_client::JoinReceipt,
    hub_public_key_hex: String,
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
    ))
}

#[cfg(not(feature = "axon-pb"))]
fn do_federation_join_and_resolve_hub_key(
    _target: &HubUraTarget,
    _membership_ura: &str,
    _public_key_hex: &str,
    _hub_ca: Option<&Path>,
) -> anyhow::Result<UraJoinResult> {
    Err(crate::support::local_invoke::federation_not_wired_error(
        "joining hub by URA",
    ))
}

#[cfg(feature = "axon-pb")]
async fn do_federation_join_and_resolve_hub_key_async(
    target: &HubUraTarget,
    membership_ura: &str,
    public_key_hex: &str,
    hub_ca: Option<&Path>,
) -> anyhow::Result<UraJoinResult> {
    use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;

    let channel = connect_hub_invocation_channel(&target.hub_endpoint, hub_ca).await?;
    let mut client = InvocationClient::new(channel);

    let public_key = hex::decode(public_key_hex).context("decode device public key hex")?;
    let provisional_caller =
        crate::runtime::provisional_ura::provisional_ura_for_pubkey(&public_key);
    let join_args = crate::runtime::federation_client::JoinArgs {
        realm: target.realm.clone(),
        membership_ura: membership_ura.to_string(),
        public_key_hex: public_key_hex.to_string(),
        pairing_secret: None,
    };
    let join_request = crate::daemon::invocation::ProtoEnvelope::federation_join_genesis(
        provisional_caller,
        target.hub_ura.clone(),
        membership_ura.to_string(),
    )?
    .invoke_request(
        crate::runtime::ability::conformance::ABILITY_FEDERATION_JOIN,
        crate::runtime::federation_client::args_to_bytes(&join_args),
    )?;
    let join_response = client.invoke(join_request).await.map_err(|status| {
        anyhow::anyhow!(
            "hub rejected federation.join: code={:?} message={}",
            status.code(),
            status.message()
        )
    })?;
    let join_response = join_response.into_inner();
    crate::daemon::invocation::federation_invoke::ensure_completed_invoke_response(
        "federation.join",
        &join_response,
    )?;
    let receipt: crate::runtime::federation_client::JoinReceipt =
        crate::runtime::federation_client::parse_receipt(&join_response.result)?;
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
    let seed = derive_device_seed_hex(&target.realm, &membership_device_id)?;
    let signer = DeterministicJoinSigner::from_seed_hex(&seed)?;
    let resolve_args = crate::runtime::federation_client::ResolveKeyArgs {
        agent_ura: target.hub_ura.clone(),
    };
    let resolve_arguments = crate::runtime::federation_client::args_to_bytes(&resolve_args);
    let subject = crate::ura::owner_ability_ura(
        &target.hub_ura,
        crate::runtime::ability::conformance::ABILITY_FEDERATION_RESOLVE_KEY,
    )
    .ok_or_else(|| anyhow::anyhow!("derive federation.resolve_key subject URA"))?;
    let descriptor_ref =
        crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            &target.hub_ura,
            crate::runtime::ability::conformance::ABILITY_FEDERATION_RESOLVE_KEY,
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
        )
        .map_err(|err| anyhow::anyhow!("derive federation.resolve_key descriptor ref: {err}"))?;
    let resolve_request = crate::daemon::invocation::ProtoEnvelope::targeted(
        membership_ura.to_string(),
        target.hub_ura.clone(),
        subject,
    )?
    .signed_descriptor_ref_invoke_request(
        crate::runtime::ability::conformance::ABILITY_FEDERATION_RESOLVE_KEY,
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
    crate::daemon::invocation::federation_invoke::ensure_completed_invoke_response(
        "federation.resolve_key",
        &resolve_response,
    )?;
    let resolved: crate::runtime::federation_client::ResolveKeyReceipt =
        crate::runtime::federation_client::parse_receipt(&resolve_response.result)?;
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
    let parsed = crate::ura::parse_ura(membership_ura)
        .map_err(|err| anyhow::anyhow!("invalid membership URA `{membership_ura}`: {err}"))?;
    parsed
        .device_id()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("membership URA `{membership_ura}` is not a device URA"))
}

#[cfg(feature = "axon-pb")]
struct DeterministicJoinSigner {
    seed: [u8; 32],
}

#[cfg(feature = "axon-pb")]
impl DeterministicJoinSigner {
    fn from_seed_hex(seed_hex: &str) -> anyhow::Result<Self> {
        let raw = hex::decode(seed_hex).context("decode deterministic device seed")?;
        let seed: [u8; 32] = raw.try_into().map_err(|raw: Vec<u8>| {
            anyhow::anyhow!("device seed must be 32 bytes, got {}", raw.len())
        })?;
        Ok(Self { seed })
    }
}

#[cfg(feature = "axon-pb")]
impl crate::daemon::identity::self_identity::SelfIdentity for DeterministicJoinSigner {
    fn sign(
        &self,
        _self_ura: &str,
        canonical_bytes: &[u8],
    ) -> Result<ed25519_dalek::Signature, crate::daemon::identity::self_identity::SelfIdentityError>
    {
        use ed25519_dalek::Signer as _;
        Ok(ed25519_dalek::SigningKey::from_bytes(&self.seed).sign(canonical_bytes))
    }

    fn public_key(
        &self,
        _self_ura: &str,
    ) -> Result<
        ed25519_dalek::VerifyingKey,
        crate::daemon::identity::self_identity::SelfIdentityError,
    > {
        Ok(ed25519_dalek::SigningKey::from_bytes(&self.seed).verifying_key())
    }
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

/// Walk through the eight join-time side effects under a live
/// stage renderer. Network failures abort with `stage_failed +
/// anyhow::bail`; best-effort steps (keyring, federated-peers,
/// realm-trust, runtime-refresh) surface as `stage_ok` or
/// `stage_skipped("(reason)")` and never short-circuit the join.
///
/// Returns the resolved `Credentials` so the caller can render the
/// summary block.
fn run_join_stages(
    token: &str,
    validate_base: &str,
    has_explicit_hub_api_override: bool,
    peer_hub: Option<&str>,
) -> anyhow::Result<config::Credentials> {
    let mut renderer = super::presentation::stage::StageRenderer::new();

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
    let mut creds = match validate_pairing_token(token, validate_base, &preflight) {
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
    mut renderer: super::presentation::stage::StageRenderer,
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

    renderer.set_active("daemon-config");
    match crate::persistence::daemon_config::ensure_minimal_device_config(&creds) {
        Ok(()) => renderer.stage_ok("daemon-config"),
        Err(e) => renderer.stage_skipped("daemon-config", &format!("({e})")),
    }

    renderer.set_active("federated-peers");
    match super::federation_wire::auto_wire_federated_peer_from_credentials(&creds, peer_hub) {
        Ok(()) => renderer.stage_ok("federated-peers"),
        Err(e) => renderer.stage_skipped("federated-peers", &format!("({e})")),
    }

    renderer.set_active("keyring");
    match put_device_keypair_to_keyring(&creds) {
        Ok(()) => renderer.stage_ok("keyring"),
        Err(e) => renderer.stage_skipped(
            "keyring",
            &format!("(offline: {e}; deterministic key fallback)"),
        ),
    }

    renderer.set_active("realm-trust");
    match super::federation_wire::auto_wire_self_realm_trust_from_credentials(&creds) {
        Ok(()) => renderer.stage_ok("realm-trust"),
        Err(e) => renderer.stage_skipped("realm-trust", &format!("({e})")),
    }
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

/// Push a fresh device keypair into the keyring under the
/// canonical self URA + hub-role overlay. Phase 3C bridge: when
/// the keyring is reachable, this is the production secret
/// When the operator paired AFTER starting the local runtime, the
/// initial boot missed the joined credentials and therefore never ran
/// the bootstrap/advertise/register sequence that requires realm +
/// node identity. Refresh that running runtime in place instead of
/// forcing a restart.
///
/// Best-effort by contract:
/// - no runtime metadata on disk => nothing is running, silently skip
/// - stale runtime metadata / failed bridge connect => warn, keep join success
/// - successful connect => reuse the exact same republish helper
///   `easynet runtime start` already uses so the bootstrap semantics
///   stay single-sourced
fn refresh_running_runtime_after_join(creds: &config::Credentials) {
    let state = match config::load() {
        Ok(state) => state,
        Err(_) => return,
    };
    if matches!(
        state.runtime_kind,
        crate::persistence::config::RuntimeKind::DaemonOnly
    ) {
        output::warn(
            "paired successfully, but a local easynet-daemon is already running. \
             Restart it with `easynet runtime stop && easynet runtime start` so it picks up the new credentials.",
        );
        return;
    }
    match state.connect_bridge() {
        Ok(bridge) => {
            output::detail(
                "runtime",
                "running runtime detected; refreshing identity + federation advertisement",
            );
            super::start::republish_via_federation_best_effort(&bridge, creds);
        }
        Err(e) => output::warn(&format!(
            "paired successfully, but could not refresh the running runtime at {}: {e}. \
             Restart it with `easynet runtime start` if cross-hub lookups keep failing.",
            state.endpoint
        )),
    }
}

/// surface; when offline, the caller logs + continues, and the
/// daemon falls back to deterministic key derivation per
/// `boot.rs::load_daemon_identity`.
///
/// Returns `Ok(())` when the put landed (or when the entry
/// already existed — pairing the same node twice is a noop, the
/// pre-existing entry stays). Errors only on transport faults
/// the operator should see.
fn put_device_keypair_to_keyring(creds: &config::Credentials) -> anyhow::Result<()> {
    use crate::daemon::identity::self_identity::{
        canonical_self_uras, KeyringClient, SelfIdentityError,
    };

    let realm = creds.realm.trim();
    let node_id = creds.node_id.trim();
    if realm.is_empty() || node_id.is_empty() {
        anyhow::bail!("credentials missing realm or node_id");
    }
    let (primary_self, role_overlays) = canonical_self_uras(realm, node_id);

    let client = KeyringClient::default_path();
    // Probe reachability with a lightweight `list` first. When the
    // daemon is already up (operator started it, or a prior join
    // spawned it) we go straight to `put`. When it is down we
    // auto-provision it below so the encrypted vault is the default
    // posture rather than something only `dev-backend.sh` sets up.
    if client.list().is_err() {
        ensure_keyring_daemon_running()?;
    }

    let seed_hex = derive_device_seed_hex(realm, node_id)?;
    match client.put(&primary_self, role_overlays, seed_hex) {
        Ok(()) => Ok(()),
        // already_exists is benign — re-pairing the same device
        // keeps the existing keypair. Any other error is real.
        Err(SelfIdentityError::Rejected { kind, .. }) if kind == "already_exists" => Ok(()),
        Err(e) => Err(anyhow::anyhow!("keyring put: {e}")),
    }
}

/// Spawn the `easynet-keyring` daemon and wait until its socket
/// answers, auto-provisioning a passphrase if the operator has not
/// supplied one.
///
/// Mirrors the daemon-spawn shape in `daemon::process`: locate the
/// sibling binary next to the running `easynet` executable, run it
/// detached (`setsid`, stdio to a log), and poll the socket until it
/// accepts a `list` RPC. The passphrase comes from
/// `keyring::load_or_create_passphrase`, which is also what `start`
/// injects into the `easynet-daemon` environment so the daemon can
/// read the same vault across restarts.
fn ensure_keyring_daemon_running() -> anyhow::Result<()> {
    use crate::daemon::identity::self_identity::KeyringClient;
    use crate::daemon::keyring::{default_socket_path, load_or_create_passphrase};
    use anyhow::Context as _;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let (passphrase, _generated) =
        load_or_create_passphrase().context("provision keyring passphrase")?;

    // A stale socket file (previous daemon crashed without unlinking)
    // makes `easynet-keyring` refuse to bind. Remove it iff nothing is
    // listening — the `list` ping above already failed, so a leftover
    // file here is dead.
    let socket_path = default_socket_path();
    #[cfg(unix)]
    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }

    let binary = resolve_keyring_bin();
    let log_path = config::state_dir().join("logs").join("easynet-keyring.log");
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open keyring log at {}", log_path.display()))?;

    let mut cmd = Command::new(&binary);
    cmd.env("EASYNET_KEYRING_PASSPHRASE", &passphrase);
    cmd.stdin(Stdio::null());
    if let Ok(out) = log.try_clone() {
        cmd.stdout(Stdio::from(out));
    }
    cmd.stderr(Stdio::from(log));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    cmd.spawn()
        .with_context(|| format!("spawn easynet-keyring at {}", binary.display()))?;

    // Poll the socket until the daemon answers. The keyring binds and
    // serves in well under a second on a warm disk; 5s covers a cold
    // Argon2id KDF on the first vault init.
    let client = KeyringClient::default_path().with_timeout(Duration::from_secs(2));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if client.list().is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "easynet-keyring did not become ready within 5s (see {})",
                log_path.display()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Locate the `easynet-keyring` binary. Prefers an explicit
/// `EASYNET_KEYRING_BIN` override, then the sibling of the running
/// executable (the install layout ships all three binaries in one
/// dir), then bare `easynet-keyring` on `PATH`.
fn resolve_keyring_bin() -> std::path::PathBuf {
    use std::path::PathBuf;
    const KEYRING_BIN: &str = "easynet-keyring";
    std::env::var_os("EASYNET_KEYRING_BIN")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join(KEYRING_BIN)))
        })
        .unwrap_or_else(|| PathBuf::from(KEYRING_BIN))
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

fn validate_pairing_response(
    envelope: easynet_axon::DeviceJoinCredentialEnvelope,
) -> anyhow::Result<easynet_axon::DeviceJoinCredentialEnvelope> {
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
    if envelope
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_none()
    {
        anyhow::bail!("pairing response missing user_id");
    }
    Ok(envelope)
}

fn credentials_from_join_envelope(
    envelope: easynet_axon::DeviceJoinCredentialEnvelope,
) -> config::Credentials {
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

fn is_docker_internal_hub_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    lower == "hub" || lower.starts_with("hub-")
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
    if !is_docker_internal_hub_host(&session_parts.host) {
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
    let url = format!("{base}/api/v1/devices/pairing/{token}/preflight");

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
            anyhow::bail!(
                "cannot reach Hub at {base}: {e}\n  Check your network connection and Hub URL."
            );
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
) -> anyhow::Result<config::Credentials> {
    let payload = build_validate_pairing_payload(preflight)?;
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
            anyhow::bail!(
                "cannot reach Hub at {base}: {e}\n  Check your network connection and Hub URL."
            );
        }
    };

    // The Hub's pairing endpoint is a versioned REST contract (see the
    // Hub's OpenAPI spec under /api/v1/devices/pairing). If `into_json`
    // fails, the bytes we got back are either not JSON at all (a proxy
    // inserted an HTML error page, a middlebox rewrote the response) or
    // the JSON shape no longer matches Axon's join credential envelope
    // (the CLI and Hub are on incompatible versions). Either way, the underlying
    // serde error is noise to an operator — they need to know *what to
    // do*, not which field's tag didn't match. We keep the raw cause in
    // the error chain via `context`, so `--verbose` / log scrapers still
    // surface the full detail, while the top-line stays operator-friendly.
    let envelope: easynet_axon::DeviceJoinCredentialEnvelope = resp.into_json().map_err(|e| {
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
    // Cross-machine cold-start fix: stash the hub's signing and
    // TLS trust material from preflight onto the in-memory +
    // on-disk credentials so the follow-up trust auto-wire can
    // populate `realm-trust.toml` plus any local pinned CA file
    // without needing on-host access to hub-local files.
    let mut creds = credentials_from_join_envelope(envelope);
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
) -> anyhow::Result<ValidatePairingPayload> {
    Ok(ValidatePairingPayload {
        info: sysinfo::collect_system_info(),
        node_id: preflight.node_id.clone(),
        device_public_key: derive_device_public_key_hex(&preflight.realm, &preflight.node_id)?,
        // Best-effort: a failure to read/persist the install id (e.g. an
        // unwritable state dir) must not block pairing — the hub then simply
        // mints a fresh node_id as before.
        install_id: config::load_or_create_install_id().ok(),
    })
}

fn derive_device_public_key_hex(realm: &str, node_id: &str) -> anyhow::Result<String> {
    use anyhow::Context as _;
    use base64::Engine as _;

    let (_seed, public_key_b64) = derive_device_keypair(realm, node_id);
    let public_key = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64.as_bytes())
        .context("decode derived device public key")?;
    Ok(hex::encode(public_key))
}

fn derive_device_seed_hex(realm: &str, node_id: &str) -> anyhow::Result<String> {
    let (seed, _public_key_b64) = derive_device_keypair(realm, node_id);
    Ok(hex::encode(seed))
}

fn derive_device_keypair(realm: &str, node_id: &str) -> ([u8; 32], String) {
    let subject_id = easynet_axon::invocation::private_agent_subject_id(node_id);
    crate::runtime::publish::derive_subject_keypair(realm, &subject_id)
}

// Deregister the previously-paired device from the hub before a re-pair
// overwrites its credentials. Mirrors `reset.rs`'s revoke helper (the
// `federation.revoke` surface is proto-gated, so both feature arms must
// compile).
#[cfg(feature = "axon-pb")]
fn invoke_federation_revoke_for_rejoin(device_ura: &str) -> anyhow::Result<()> {
    crate::daemon::invocation::federation_invoke::invoke_federation_revoke(
        device_ura,
        "device-rejoin",
    )
}

#[cfg(not(feature = "axon-pb"))]
fn invoke_federation_revoke_for_rejoin(_device_ura: &str) -> anyhow::Result<()> {
    Err(crate::support::local_invoke::federation_not_wired_error(
        "deregistering the old device on re-pair",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

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
    fn hub_ura_target_derives_endpoint_for_public_realm() {
        let target =
            HubUraTarget::parse("easynet:///r/easynet.run/hub", None, None).expect("public realm");
        assert_eq!(target.realm, "easynet.run");
        assert_eq!(target.hub_endpoint, "https://easynet.run:50443");
    }

    #[test]
    fn hub_ura_target_requires_ca_for_private_realm() {
        let err = HubUraTarget::parse("easynet:///r/localhost/hub", None, None)
            .expect_err("localhost requires CA");
        assert!(err.to_string().contains("--hub-ca"));

        let target = HubUraTarget::parse(
            "easynet:///r/localhost/hub",
            Some(55443),
            Some(Path::new("/tmp/hub-ca.pem")),
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
    fn validate_pairing_response_rejects_empty_node_id() {
        let envelope = easynet_axon::DeviceJoinCredentialEnvelope {
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
    fn credentials_from_join_envelope_projects_axon_wire_shape() {
        let envelope = easynet_axon::DeviceJoinCredentialEnvelope {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            realm: "tenant".into(),
            deploy_signature: "sig".into(),
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            ..Default::default()
        };
        let creds = credentials_from_join_envelope(envelope);
        assert_eq!(creds.node_id, "node");
        assert_eq!(creds.realm, "tenant");
        assert_eq!(creds.username.as_deref(), Some("alice"));
        assert_eq!(creds.user_id.as_deref(), Some("user-alice"));
    }

    #[test]
    fn validate_pairing_response_rejects_missing_username() {
        let envelope = easynet_axon::DeviceJoinCredentialEnvelope {
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
        let envelope = easynet_axon::DeviceJoinCredentialEnvelope {
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
            hub_endpoint: "https://hub:50443".into(),
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
            hub_endpoint: "https://hub:50443".into(),
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
            hub_endpoint: "https://hub:50443".into(),
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
            hub_endpoint: "https://hub:50443".into(),
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
            "http://hub:8080"
        ));
        assert_eq!(creds.hub_endpoint, "https://hub:50443");
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
    fn derive_device_public_key_hex_matches_runtime_derivation() {
        let realm = "tenant-a";
        let node_id = "en-test-node";
        let got = derive_device_public_key_hex(realm, node_id).expect("derive hex");
        let want_b64 = crate::runtime::publish::derive_owner_public_key_b64(realm, node_id);
        let want = hex::encode(
            base64::engine::general_purpose::STANDARD
                .decode(want_b64.as_bytes())
                .expect("decode owner b64"),
        );
        assert_eq!(got, want);
    }

    #[test]
    fn derive_device_seed_hex_matches_pairing_public_key() {
        let realm = "tenant-a";
        let node_id = "en-test-node";
        let seed_hex = derive_device_seed_hex(realm, node_id).expect("derive seed");
        let seed_bytes = hex::decode(seed_hex).expect("decode seed hex");
        let seed: [u8; 32] = seed_bytes.as_slice().try_into().expect("seed length");
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);

        assert_eq!(
            hex::encode(signing_key.verifying_key().to_bytes()),
            derive_device_public_key_hex(realm, node_id).expect("derive public key")
        );
    }

    #[test]
    fn pairing_preflight_accepts_current_realm_schema() {
        let preflight: PairingPreflight = serde_json::from_value(serde_json::json!({
            "realm": "tenant-a",
            "node_id": "en-test-node",
            "hub_public_key_b64": "",
            "hub_tls_ca_pem_b64": "",
            "hub_agent_ura": crate::ura::hub_ura("tenant-a")
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
        let payload = build_validate_pairing_payload(&preflight).expect("build payload");
        assert_eq!(payload.node_id, "en-test-node");
        assert_eq!(payload.device_public_key.len(), 64);
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
        let err = validate_pairing_token("token_1234", &base, &preflight)
            .expect_err("transport failure should error");
        assert!(err.to_string().contains("cannot reach Hub"));
    }
}

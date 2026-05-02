// EasyNet CLI — Ability publishing via federation.advertise_*
// =============================================================
//
// File: src/runtime/publish.rs
//
// Per AXON-RFC-001 §A4 + plan v4.1.2 §1, abilities are published to
// the realm directory by invoking the hub-profile Agent's
// `federation.advertise_agent` + `federation.advertise_abilities`
// abilities — NOT by the legacy `register_runtime_local_mcp_tool`
// path that was deleted in P1.2.a.
//
// Pre-RFC history this module replaces
// ------------------------------------
// The pre-RFC publish.rs registered every per-agent manifest +
// every "system ability" against an in-memory MCP catalog held by
// the local axon-runtime. That layer was the single biggest source
// of "frontend Skills page is empty" bugs because the catalog was
// not persistent and the MCP path was load-bearing for Hub-mediated
// discovery. P1.2.a deleted the underlying RPC; the module then
// stubbed every public function to `Ok(false)` until P3+ shipped
// the federation alternative.
//
// What this module does now
// -------------------------
//   * `republish_abilities_via_advertise(invoker, tenant, plan)`
//     bootstraps URAs, persists local-agents.json, advertises
//     every enabled Agent + its descriptors. The single entry
//     point the daemon-boot path and `easynet agent add` both
//     call.
//   * `unpublish_abilities_via_revoke(invoker, tenant, realm,
//     agent_uri)` revokes one Agent's directory entry — used by
//     `easynet agent remove`. Maps to `federation.revoke` per
//     plan §18.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::persistence::local_agents::{self, LocalAgentsFile};
use crate::runtime::advertise::{self, AbilityInvoker, AdvertiseOutcome};
use crate::runtime::agents::profiles::{
    self as profiles_mod,
    bootstrap::{self, BootstrapOutcome, BootstrapPlan, UriMinter, UuidMinter},
};
use serde_json::Value;

/// Per-call summary returned by `republish_abilities_via_advertise`.
/// Each row is one Agent the daemon advertised — either the device
/// itself (Selfsigned, Model A) or a hosted profile (HostedBy,
/// Model B). The CLI / daemon-boot output layer renders these.
#[derive(Debug, Clone)]
pub struct PublishOutcome {
    /// Canonical URA the advertise call targeted. Empty when
    /// bootstrap returned no rows (operator hasn't enabled any
    /// hosted profiles).
    pub agent_uri: String,
    /// Free-form descriptor of which Agent this row corresponds to,
    /// e.g. `device`, `consent/default`, `llm/claude`. Used for
    /// log lines, not for protocol decisions.
    pub label: String,
    /// `Ok(())` on a clean advertise round trip; `Err(msg)` on any
    /// failure. Per the historical contract, this layer is best-
    /// effort: callers log + continue rather than abort startup.
    pub result: Result<(), String>,
}

/// The single entry point the daemon-boot path and `easynet agent
/// add` use to keep the realm directory in sync with the local
/// install state.
///
/// Steps:
///   1. Run `bootstrap_local_agents` to mint or reuse URAs for
///      every enabled hosted profile.
///   2. Persist the resulting `local-agents.json` (mode 0600).
///   3. Advertise the device-profile Agent itself (Selfsigned).
///   4. Advertise each hosted Agent (HostedBy).
///   5. Advertise the AbilityDescriptors emitted by each profile
///      module's `descriptors_for(...)`.
///
/// Returns a flat Vec<PublishOutcome> the caller renders. The
/// function never panics on a failed advertise — every per-row
/// error becomes one Err entry.
pub fn republish_abilities_via_advertise<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    plan: &BootstrapPlan,
) -> Vec<PublishOutcome> {
    republish_with_minter(invoker, tenant_id, plan, &UuidMinter)
}

/// Same as `republish_abilities_via_advertise` but accepts a
/// custom URI minter. Used by tests with a deterministic minter.
pub fn republish_with_minter<I: AbilityInvoker, M: UriMinter>(
    invoker: &I,
    tenant_id: &str,
    plan: &BootstrapPlan,
    minter: &M,
) -> Vec<PublishOutcome> {
    let mut outcomes = Vec::new();

    // Step 1+2: bootstrap + persist.
    let mut file = match local_agents::load() {
        Ok(f) => f,
        Err(e) => {
            outcomes.push(PublishOutcome {
                agent_uri: String::new(),
                label: "local-agents.json".into(),
                result: Err(format!("read failed; using empty file: {e}")),
            });
            LocalAgentsFile::default()
        }
    };
    let bootstrap_outcomes = bootstrap::bootstrap_local_agents(plan, &mut file, minter);
    if let Err(e) = local_agents::save(&file) {
        outcomes.push(PublishOutcome {
            agent_uri: String::new(),
            label: "local-agents.json".into(),
            result: Err(format!("save failed: {e}")),
        });
        // Continue — in-memory state still allows advertise to run.
    }

    if plan.realm.is_empty() || plan.host_device_uri.is_empty() {
        // Pre-join: nothing to advertise yet (the hub-profile that
        // would receive the call doesn't know us). The bootstrap
        // file has been persisted with `<unjoined>` placeholders;
        // a post-join boot will retry.
        outcomes.push(PublishOutcome {
            agent_uri: String::new(),
            label: "skipped".into(),
            result: Err("daemon not yet joined to a realm; advertise deferred".into()),
        });
        return outcomes;
    }

    // Step 3: advertise the device-profile (Selfsigned, Model A).
    // RFC-002: pass host_node_id so federation.forward_invoke can
    // route inbound forward requests to this daemon's local-tool
    // dispatch surface. Falls back to the legacy node-less form
    // when host_device_uri lacks an `/agent/<id>` suffix.
    let host_node_id = host_node_id_from_uri(&plan.host_device_uri);
    let device_outcome = advertise::advertise_self_signed_device_with_host_node(
        invoker,
        tenant_id,
        &plan.realm,
        &plan.host_device_uri,
        // P5 supplies the actual public_key_hex; P4.8a ships an
        // empty placeholder so the advertise wire shape stays
        // stable. The hub still records the URA + status.
        "",
        host_node_id.clone(),
    );
    outcomes.push(advertise_outcome_to_publish_outcome(
        device_outcome,
        "device".into(),
    ));

    // Lookup tables from bootstrap_outcomes for the descriptor
    // advertise step that follows.
    let consent_uri = first_uri(&bootstrap_outcomes, "consent", "default");
    let policy_uri = first_uri(&bootstrap_outcomes, "policy", "default");
    let mcp_uri = first_uri(&bootstrap_outcomes, "mcp", "default");
    let llm_uris: Vec<(String, String)> = bootstrap_outcomes
        .iter()
        .filter(|o| o.profile == "llm")
        .map(|o| (o.name.clone(), o.agent_uri.clone()))
        .collect();

    // Step 4: advertise each hosted Agent (HostedBy, Model B).
    for o in &bootstrap_outcomes {
        let outcome = advertise::advertise_hosted_agent_with_host_node(
            invoker,
            tenant_id,
            &plan.realm,
            &o.agent_uri,
            &plan.host_device_uri,
            host_node_id.clone(),
        );
        outcomes.push(advertise_outcome_to_publish_outcome(
            outcome,
            format!("{}/{}", o.profile, o.name),
        ));
    }

    // Step 5: advertise descriptors per Agent. We use the
    // profiles aggregator so each Agent's descriptor list is
    // computed once from the live registry.
    let mut descriptors = profiles_mod::all_descriptors_for_host(
        &plan.host_device_uri,
        consent_uri.as_deref(),
        policy_uri.as_deref(),
        mcp_uri.as_deref(),
        &llm_uris,
    );

    // Step 5b: advertise the abilities OWNED by each user-installed
    // agent (e.g. `alice.chat` and any per-agent verbs declared in
    // `<workspace>/abilities/*.ability.toml`). The `llm` profile's
    // descriptors_for() only emits the generic conversation/session/
    // meta/skill prefixes — without this step the realm directory
    // never learns that `alice.chat` exists, so the EasyNet
    // frontend's Abilities catalog cannot list it and the user
    // cannot invoke per-agent abilities through the UI.
    //
    // Read the live registry once, look up each user agent's URA
    // in `llm_uris` (bootstrap minted these earlier), call
    // `abilities_for(name, entry)` to get the per-agent specs, and
    // convert to AbilityDescriptors owned by the user-agent URA.
    // A registry-load failure degrades to "no per-agent advertise
    // this cycle" rather than blocking the rest of publish — the
    // outcome row surfaces the reason.
    match crate::registry::agents::load_agents() {
        Ok(reg) => {
            for (name, entry) in &reg.agents {
                let owner_uri = match llm_uris
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, u)| u.clone())
                {
                    Some(u) => u,
                    None => continue, // bootstrap didn't mint a URA for this agent
                };
                let specs = crate::runtime::abilities::abilities_for(name, entry);
                for spec in specs {
                    let desc = crate::runtime::ability_descriptor::AbilityDescriptor::new(
                        spec.name(),
                        &owner_uri,
                        crate::runtime::ability_descriptor::Visibility::Scoped,
                    );
                    match desc {
                        Ok(d) => {
                            let d = d
                                .with_description(spec.description())
                                .with_input_schema(spec.parameters().clone())
                                .with_source(format!("agent:{name}"));
                            descriptors.push(d);
                        }
                        Err(e) => {
                            outcomes.push(PublishOutcome {
                                agent_uri: owner_uri.clone(),
                                label: format!("agent-ability/{}", spec.name()),
                                result: Err(format!("descriptor build failed: {e}")),
                            });
                        }
                    }
                }
            }
        }
        Err(e) => {
            outcomes.push(PublishOutcome {
                agent_uri: String::new(),
                label: "user-agent-abilities".into(),
                result: Err(format!(
                    "load agent registry failed; per-agent abilities not advertised this cycle: {e}"
                )),
            });
        }
    }

    // Group descriptors by owner Agent and advertise each group.
    let mut by_owner: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
    for d in descriptors {
        by_owner
            .entry(d.owner_agent_uri.clone())
            .or_default()
            .push(d);
    }
    for (owner_uri, abilities) in by_owner {
        let result =
            advertise::advertise_abilities(invoker, tenant_id, &plan.realm, &owner_uri, &abilities);
        outcomes.push(PublishOutcome {
            agent_uri: owner_uri.clone(),
            label: format!("abilities/{}", abilities.len()),
            result: result.map(|_| ()),
        });
    }

    outcomes
}

/// Register every daemon-owned ability with the local axon-runtime
/// via the `runtime.register_local_tool` admin RPC. After this call
/// returns, an external Invoke arriving at axon-runtime for any
/// of the registered ability names is routed back to the daemon's
/// `dispatch_endpoint` (a UDS path the daemon's
/// `services::control::runtime_dispatch` server is listening on).
///
/// This is Step 3-completion on the daemon side. The runtime side
/// (EasyNet-Axon `runtime_admin.rs` + `try_dispatch_runtime_local_tool`)
/// already exposes the registration RPC and reads `dispatch_endpoint`
/// to forward invokes; without this registration the runtime has
/// nothing to look up and falls through to `NoBinding` for every
/// daemon-owned ability.
///
/// Inputs:
///   * `invoker` — same `BridgeAbilityInvoker` used by advertise;
///     wraps the dendrite-bridge `ability_call_raw` path. The
///     resource URI follows the canonical 5-segment shape required
///     by `AxonClient::canonicalize_easynet_resource_uri`:
///     `easynet:///r/prv/hub/<realm>/abilities/runtime.register_local_tool@1`.
///     The `runtime.*` namespace is intercepted before membership +
///     admission checks (rpc_handlers.rs::is_runtime_admin_ability)
///     so the hub-style subject is purely a URI shape, not an actual
///     hub-routing decision.
///   * `tenant_id` — runtime key namespace.
///   * `realm` — used to construct the URI's subject_value. Any non-
///     empty value works since runtime.* is intercepted by ability
///     name; we reuse the daemon's joined realm for consistency with
///     federation.advertise URIs.
///   * `node_id` — the local device's node id from `~/.easynet/credentials.json`.
///   * `dispatch_endpoint` — `ipc://<absolute-path>`. Typically
///     `runtime_dispatch::dispatch_endpoint_uri()`.
///
/// Returns one `PublishOutcome` per registration attempt. Best-
/// effort: a registration failure is logged but never aborts boot.
/// The daemon stays advertising in the directory; only the
/// runtime-side dispatch path degrades to NoBinding.
pub fn register_local_tools_via_runtime<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    node_id: &str,
    dispatch_endpoint: &str,
) -> Vec<PublishOutcome> {
    let mut outcomes = Vec::new();

    // The set of ability names we register equals the device-
    // profile's own published abilities + every user-agent's
    // per-agent abilities. We mirror the advertise side's
    // descriptor walk so the registry and the directory stay
    // consistent: anything we *advertised* must also be
    // *dispatchable*, otherwise an Abilities-page user clicks
    // Invoke and gets NoBinding.
    let names = collect_daemon_owned_ability_names();

    // Non-pub URIs require `?tenant_id=<id>` per the SDK
    // canonicalizer; without it we get "tenant_id query is required"
    // before any admission/membership check runs.
    let resource_uri = format!(
        "easynet:///r/prv/hub/{realm}/abilities/runtime.register_local_tool@1?tenant_id={tenant_id}"
    );
    for name in names {
        let args = build_register_args(tenant_id, node_id, &name, dispatch_endpoint);
        let result = invoker.invoke_ability(tenant_id, &resource_uri, args);
        outcomes.push(PublishOutcome {
            agent_uri: format!("local:{node_id}"),
            label: format!("runtime/register/{name}"),
            result: result.map(|_| ()),
        });
    }

    outcomes
}

/// Bootstrap this node's trusted-key material with the local
/// axon-runtime via `runtime.bootstrap_self_identity`. Must run
/// before any signed Invoke is attempted (i.e. before
/// `register_local_tools_via_runtime` and before any
/// federation.advertise_* call).
///
/// Why this is needed:
///
/// AXON-RFC-001 P5-rewrite-13 deleted the legacy `register_node`
/// RPC that historically populated `state.identity.node_keys` /
/// `node_key_materials` and inserted into `state.topology.nodes`.
/// `verify_easynet_subject_key_binding` (rpc_handlers' early
/// envelope-metadata check) still requires those tables to be
/// populated before a signed Invoke is admitted. With `register_node`
/// gone, every Invoke fails with
/// `AXON_EASYNET_SUBJECT_KEY_UNREGISTERED` until something else
/// fills the gap.
///
/// `runtime.bootstrap_self_identity` is the v1 self-bootstrap. The
/// daemon derives the same deterministic public key the bridge will
/// later sign under (via `AxonClient::derive_owner_auth`, which
/// hashes `(tenant_id, subject_id, DERIVE_CONTEXT)`) and passes it
/// to the runtime; the runtime stores it once per node. From that
/// moment on, every signed Invoke from the bridge passes verification.
///
/// Best-effort by contract: a failure here logs and returns one
/// `PublishOutcome` per attempt. The runtime is still functional
/// for the runtime-dispatch path that does not require signed
/// metadata; only the federation/Invoke path degrades.
pub fn bootstrap_self_identity_via_runtime<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    node_id: &str,
) -> PublishOutcome {
    // Two keys to register, both under this node_id:
    //
    //   1. agent key — derived from `easynet:prv:reg:agent.<node_id>`.
    //      Used by every Invoke whose canonical subject is the
    //      daemon's own agent identity (most calls — chat, custom
    //      verbs, runtime.register_local_tool's signed envelope).
    //
    //   2. hub key   — derived from `easynet:prv:hub:<realm>`.
    //      Used by every federation hub-profile call
    //      (federation.advertise_*, federation.resolve, …) because
    //      the SDK's `default_auth_for_subject` derives a SUBJECT-
    //      KEYED ed25519 key — and the hub subject differs from the
    //      agent subject, so the derived key differs too. Without
    //      this second registration every federation hub call
    //      fails with PUBLIC_KEY_UNTRUSTED.
    //
    // The two calls share `node_id`. axon-runtime's
    // `runtime.bootstrap_self_identity` appends a NEW key under the
    // existing node when the (node_id, public_key) tuple is novel,
    // so the second call doesn't overwrite the first.
    let resource_uri = format!(
        "easynet:///r/prv/hub/{realm}/abilities/runtime.bootstrap_self_identity@1?tenant_id={tenant_id}"
    );
    let agent_key_b64 = derive_owner_public_key_b64(tenant_id, node_id);
    let agent_args = serde_json::json!({
        "tenant_id": tenant_id,
        "node_id": node_id,
        "owner_id": node_id, // v1: daemon runs as the owner of its own node
        "display_name": "",
        "public_key_b64": agent_key_b64,
    });
    let agent_result = invoker.invoke_ability(tenant_id, &resource_uri, agent_args);
    if let Err(e) = agent_result {
        return PublishOutcome {
            agent_uri: format!("local:{node_id}"),
            label: "runtime/bootstrap_self_identity".into(),
            result: Err(e),
        };
    }

    let hub_key_b64 = derive_hub_public_key_b64(tenant_id, realm);
    if hub_key_b64 != agent_key_b64 {
        let hub_args = serde_json::json!({
            "tenant_id": tenant_id,
            "node_id": node_id,
            "owner_id": node_id,
            "display_name": "",
            "public_key_b64": hub_key_b64,
        });
        let hub_result = invoker.invoke_ability(tenant_id, &resource_uri, hub_args);
        if let Err(e) = hub_result {
            return PublishOutcome {
                agent_uri: format!("local:{node_id}"),
                label: "runtime/bootstrap_self_identity_hub".into(),
                result: Err(e),
            };
        }
    }

    PublishOutcome {
        agent_uri: format!("local:{node_id}"),
        label: "runtime/bootstrap_self_identity".into(),
        result: Ok(()),
    }
}

/// Compute the standard-base64 (with padding) ed25519 public key
/// the local node will sign under for the *prv-visibility*
/// `r/prv/reg/agent.<node>/abilities/...` URI shape — the canonical
/// shape every daemon-owned ability invocation uses end-to-end.
///
/// Mirrors `AxonClient::default_auth_for_subject` in the SDK:
///
///   * For prv/org subjects it derives directly from the FULL
///     subject_id string (`derive_subject_auth(subject_id, tenant)`).
///   * For pub-visibility subjects it derives from the bare
///     owner_id string (`derive_owner_auth(owner_id, tenant)`).
///
/// We bootstrap the prv-visibility key here because the entire
/// daemon-owned-ability call path uses URIs that canonicalize to a
/// prv subject; a key derived under any other subject would fail
/// `verify_easynet_subject_key_binding` even though the math is
/// otherwise identical. If/when a public-visibility caller is
/// introduced, the daemon will need a second bootstrap call for
/// that subject.
///
/// The runtime's `KeyInfo` storage expects standard base64; the
/// bridge uses URL-SAFE-NO-PAD when it transmits the key in
/// `easynet.public_key`. Both encodings decode to the same 32
/// bytes; the admin RPC stays on standard base64 to avoid a needless
/// translation step on the runtime side.
/// Public alias for the heartbeat keepalive call site. Same
/// derivation as `derive_owner_public_key_b64`; kept as a separate
/// `pub(crate)` symbol so the heartbeat module can call it without
/// promoting the underlying helper to the public surface.
pub(crate) fn derive_owner_public_key_b64_for_keepalive(tenant_id: &str, node_id: &str) -> String {
    derive_owner_public_key_b64(tenant_id, node_id)
}

pub(crate) fn derive_owner_public_key_b64(tenant_id: &str, node_id: &str) -> String {
    let subject_id = format!("easynet:prv:reg:agent.{node_id}");
    derive_subject_public_key_b64(tenant_id, &subject_id)
}

/// Hub-profile counterpart of `derive_owner_public_key_b64`. Returns
/// the public key the bridge will sign under for hub-shaped resource
/// URIs (`r/prv/hub/<realm>/abilities/...`). The SDK's
/// `default_auth_for_subject` derives a DIFFERENT key for the hub
/// subject than for the agent subject, so the daemon needs to
/// register both — see `bootstrap_self_identity_via_runtime`.
pub(crate) fn derive_hub_public_key_b64(tenant_id: &str, realm: &str) -> String {
    let subject_id = format!("easynet:prv:hub:{realm}");
    derive_subject_public_key_b64(tenant_id, &subject_id)
}

fn derive_subject_public_key_b64(tenant_id: &str, subject_id: &str) -> String {
    let (_seed, pk_b64) = derive_subject_keypair(tenant_id, subject_id);
    pk_b64
}

/// Extract the node id segment from a canonical device URA.
/// Accepts both the legacy `easynet:///r/<tenant>/agent/<node>` shape
/// and the URA-conformant `easynet:///r/<scope>/reg/agent.<node>`
/// shape. Returns None when the URA does not match either form.
fn host_node_id_from_uri(uri: &str) -> Option<String> {
    if let Some(rest) = uri.strip_prefix("easynet:///r/") {
        // Drop optional query-string suffix.
        let rest = rest.split('?').next().unwrap_or(rest);
        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        // Pattern A: <tenant>/agent/<node> (legacy 3-segment)
        if segments.len() >= 3 && segments[1] == "agent" {
            return Some(segments[2].to_string());
        }
        // Pattern B: <scope>/reg/agent.<node> (URA-conformant)
        if segments.len() >= 3 && segments[1] == "reg" {
            if let Some(node) = segments[2].strip_prefix("agent.") {
                return Some(node.to_string());
            }
        }
    }
    None
}

/// URI v2 device-keypair wrapper. Returns `(seed, pk_b64)` for the
/// device-profile agent under the given realm. Subject ID shape
/// `easynet:prv:reg:device.<node_id>` (URI v2 device kind).
///
/// Phase 14 production deploy switches daemons to use this in place
/// of `derive_agent_keypair` once the backend trust anchor has been
/// regenerated under the device-URI shape.
pub(crate) fn derive_device_keypair(realm: &str, node_id: &str) -> ([u8; 32], String) {
    let subject_id = format!("easynet:prv:reg:device.{node_id}");
    derive_subject_keypair(realm, &subject_id)
}

/// URI v1 (and current production) agent-keypair wrapper. Subject
/// ID shape `easynet:prv:reg:agent.<node_id>`. Kept as the default
/// path through Phase 14; the device-shaped wrapper above flips
/// in only after the backend re-mints its trust anchor.
pub(crate) fn derive_agent_keypair(realm: &str, node_id: &str) -> ([u8; 32], String) {
    let subject_id = format!("easynet:prv:reg:agent.{node_id}");
    derive_subject_keypair(realm, &subject_id)
}

/// Deterministic keypair derivation used by the SDK's
/// `derive_subject_auth`. Returns `(seed_bytes, public_key_b64)` so
/// the daemon can both publish the public key AND mirror the seed
/// into the keyring (RFC-002 P3) without re-deriving in two places.
pub(crate) fn derive_subject_keypair(tenant_id: &str, subject_id: &str) -> ([u8; 32], String) {
    use base64::Engine as _;
    use ed25519_dalek::SigningKey;
    use sha2::{Digest, Sha256};

    const DERIVE_CONTEXT: &str = "axon-client-sdk-ed25519-v1";
    let mut hasher = Sha256::new();
    hasher.update(tenant_id.as_bytes());
    hasher.update(b":");
    hasher.update(subject_id.as_bytes());
    hasher.update(b":");
    hasher.update(DERIVE_CONTEXT.as_bytes());
    let digest = hasher.finalize();
    let mut seed = [0_u8; 32];
    seed.copy_from_slice(&digest[..32]);
    let signing = SigningKey::from_bytes(&seed);
    let pk_b64 =
        base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes());
    (seed, pk_b64)
}

/// Mirror the deterministic agent + hub keys into the keyring so
/// federation `KeyResolver` queries return the same bytes the SDK
/// signs under. Idempotent: re-running with the same inputs is a
/// no-op (existing matching entry is reused). The keyring stores
/// the seed encrypted so `keyring.sign` works for these entries
/// (necessary for forward_invoke to sign envelopes locally without
/// going through the SDK).
pub(crate) fn mirror_derived_keys_into_keyring(
    keyring: &crate::runtime::keyring::KeyringHandle,
    tenant_id: &str,
    node_id: &str,
    realm: &str,
    device_uri: &str,
) -> anyhow::Result<()> {
    let agent_subject_id = format!("easynet:prv:reg:agent.{node_id}");
    let (agent_seed, agent_pk_b64) = derive_subject_keypair(tenant_id, &agent_subject_id);
    let agent_pk = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        agent_pk_b64.as_bytes(),
    )?;
    keyring.mirror_external_key(
        "agent_signing",
        device_uri.to_string(),
        &agent_pk,
        Some(&agent_seed),
    )?;

    let hub_subject_id = format!("easynet:prv:hub:{realm}");
    let (hub_seed, hub_pk_b64) = derive_subject_keypair(tenant_id, &hub_subject_id);
    if hub_pk_b64 != agent_pk_b64 {
        let hub_pk = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            hub_pk_b64.as_bytes(),
        )?;
        let hub_subject_uri = format!("easynet:///r/prv/hub/{realm}?tenant_id={tenant_id}");
        keyring.mirror_external_key("hub_signing", hub_subject_uri, &hub_pk, Some(&hub_seed))?;
    }
    Ok(())
}

/// Build the JSON args for a single `runtime.register_local_tool`
/// invocation. Pulled out as a helper so the test below can pin
/// the wire shape without spinning up the bridge invoker.
fn build_register_args(
    tenant_id: &str,
    node_id: &str,
    tool_name: &str,
    dispatch_endpoint: &str,
) -> Value {
    serde_json::json!({
        "tenant_id": tenant_id,
        "node_id": node_id,
        "tool_name": tool_name,
        // We don't have a McpToolSpec proto encoded here — the
        // runtime's register handler accepts an empty
        // spec_proto_b64 and inherits the wire tool_name. A future
        // PR can encode the real spec via prost so meta.list_tools
        // surfaces input_schema; v1 trades that for simplicity.
        "spec_proto_b64": "",
        "dispatch_endpoint": dispatch_endpoint,
    })
}

/// Build the canonical list of ability names the daemon owns and
/// therefore must register with the runtime. Drives the publish
/// loop above; pulled out so tests can pin the set against
/// `published_ability_names` + per-agent abilities.
fn collect_daemon_owned_ability_names() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    // Device-profile + consent + policy + mcp + llm published
    // names — these are the names `meta.list_abilities` would
    // surface to a remote caller. Driven by the same source the
    // runtime-local registry surfaces via list_tools.
    names.extend(crate::runtime::agents::published_ability_names());

    // Per-user-agent abilities — `<agent>.chat` and any
    // `<agent>.<verb>` declared in the agent's
    // `<workspace>/abilities/*.ability.toml`. These don't appear
    // in `published_ability_names` (that table is device-level)
    // so we walk the agent registry the same way
    // `republish_with_minter` does at advertise time.
    if let Ok(reg) = crate::registry::agents::load_agents() {
        for (agent_name, entry) in &reg.agents {
            for spec in crate::runtime::abilities::abilities_for(agent_name, entry) {
                names.push(spec.name().to_string());
            }
        }
    }

    // Dedup. The published table has uniqueness by name; per-agent
    // walks may duplicate `<agent>.chat` for fixtures that didn't
    // get a unique name. Sort for deterministic test output.
    names.sort();
    names.dedup();
    names
}

/// Revoke one Agent's directory entry. Used by `easynet agent
/// remove` to keep the hub's directory in sync with the local
/// install state.
pub fn unpublish_abilities_via_revoke<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_uri: &str,
    reason: &str,
) -> PublishOutcome {
    let resource_uri =
        format!("easynet:///r/prv/hub/{realm}/abilities/federation.revoke@1?tenant_id={tenant_id}");
    let payload = serde_json::json!({
        "agent_uri": agent_uri,
        "reason": reason,
    });
    let result = invoker
        .invoke_ability(tenant_id, &resource_uri, payload)
        .map(|_| ());
    PublishOutcome {
        agent_uri: agent_uri.into(),
        label: "revoke".into(),
        result,
    }
}

fn advertise_outcome_to_publish_outcome(
    outcome: AdvertiseOutcome,
    label: String,
) -> PublishOutcome {
    PublishOutcome {
        agent_uri: outcome.agent_uri,
        label,
        result: outcome.result.map(|_receipt| ()),
    }
}

fn first_uri(outcomes: &[BootstrapOutcome], profile: &str, name: &str) -> Option<String> {
    outcomes
        .iter()
        .find(|o| o.profile == profile && o.name == name)
        .map(|o| o.agent_uri.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::cli::test_support::HomeGuard;
    use crate::runtime::agents::profiles::bootstrap::LlmSubAgent;
    use std::cell::RefCell;

    /// Recording fake invoker; mirrors the one in advertise.rs but
    /// counts calls per resource URI so we can assert the expected
    /// federation.* sequence happened.
    struct CountingInvoker {
        calls: RefCell<Vec<(String, Value)>>,
        reply: Value,
    }

    impl CountingInvoker {
        fn new(reply: Value) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                reply,
            }
        }
        fn calls(&self) -> Vec<(String, Value)> {
            self.calls.borrow().clone()
        }
    }

    impl AbilityInvoker for CountingInvoker {
        fn invoke_ability(
            &self,
            _tenant_id: &str,
            resource_uri: &str,
            payload_json: Value,
        ) -> Result<Value, String> {
            self.calls
                .borrow_mut()
                .push((resource_uri.to_string(), payload_json));
            Ok(self.reply.clone())
        }
    }

    struct FailingInvoker;
    impl AbilityInvoker for FailingInvoker {
        fn invoke_ability(&self, _: &str, _: &str, _: Value) -> Result<Value, String> {
            Err("transport down".into())
        }
    }

    /// Same deterministic minter we used in bootstrap tests.
    struct CountingMinter(std::cell::Cell<usize>);
    impl CountingMinter {
        fn new() -> Self {
            Self(std::cell::Cell::new(0))
        }
    }
    impl UriMinter for CountingMinter {
        fn mint_id(&self, profile: &str, name: &str) -> String {
            let n = self.0.get();
            self.0.set(n + 1);
            format!("{profile}-{name}-{n}")
        }
    }

    fn good_reply() -> Value {
        serde_json::json!({"ack": true, "replaced_prior": false})
    }

    fn plan_for(realm: &str, host: &str) -> BootstrapPlan {
        BootstrapPlan {
            realm: realm.into(),
            user_id: "test-user".into(),
            host_device_uri: host.into(),
            consent: true,
            policy: false,
            mcp: false,
            llm_sub_agents: vec![LlmSubAgent {
                name: "claude".into(),
                agent_type_display: "claude-code".into(),
            }],
        }
    }

    #[test]
    fn republish_emits_device_advertise_then_each_hosted_then_descriptors() {
        let _h = HomeGuard::new();
        let invoker = CountingInvoker::new(good_reply());
        let plan = plan_for("acme", "easynet:///r/acme/agent/01DEV");
        let outcomes = republish_with_minter(&invoker, "tenant", &plan, &CountingMinter::new());

        // We expect: 1 device-advertise + N hosted-advertises + M
        // ability-advertises. With consent + claude enabled, hosted
        // count = 2 (consent/default + llm/claude).
        let calls = invoker.calls();
        let resource_seq: Vec<&str> = calls.iter().map(|(u, _)| u.as_str()).collect();
        let device_count = resource_seq
            .iter()
            .filter(|u| u.contains("federation.advertise_agent@1"))
            .count();
        let abilities_count = resource_seq
            .iter()
            .filter(|u| u.contains("federation.advertise_abilities@1"))
            .count();
        assert_eq!(
            device_count, 3,
            "1 device + 2 hosted = 3 advertise_agent calls; got resource sequence {resource_seq:?}"
        );
        assert!(
            abilities_count >= 1,
            "at least one advertise_abilities call expected; got {resource_seq:?}"
        );
        // No outcome should be Err on a clean reply.
        for o in &outcomes {
            if o.label == "skipped" {
                panic!("post-join plan produced a skipped outcome: {o:?}");
            }
            assert!(o.result.is_ok(), "unexpected Err outcome: {o:?}");
        }
    }

    #[test]
    fn republish_skips_advertise_when_realm_empty() {
        let _h = HomeGuard::new();
        let invoker = CountingInvoker::new(good_reply());
        let mut plan = plan_for("", "");
        plan.consent = true;
        let outcomes = republish_with_minter(&invoker, "tenant", &plan, &CountingMinter::new());
        // Pre-join: bootstrap still ran but advertise was skipped.
        // We should see ZERO calls to the bridge.
        assert!(
            invoker.calls().is_empty(),
            "no advertise calls should have happened"
        );
        // The single outcome must report the skip.
        let skipped = outcomes
            .iter()
            .find(|o| o.label == "skipped")
            .expect("expected a 'skipped' outcome");
        assert!(skipped.result.is_err());
    }

    #[test]
    fn republish_surfaces_per_call_failure_without_aborting() {
        let _h = HomeGuard::new();
        let plan = plan_for("acme", "easynet:///r/acme/agent/01DEV");
        let outcomes =
            republish_with_minter(&FailingInvoker, "tenant", &plan, &CountingMinter::new());
        // Every advertise call must turn into one Err PublishOutcome.
        assert!(
            outcomes.iter().all(|o| {
                o.label == "skipped" || o.result.is_err() || o.label == "local-agents.json"
            }),
            "every advertise must surface its error; got {outcomes:?}"
        );
        let failed = outcomes
            .iter()
            .filter(|o| o.result.is_err() && o.label != "local-agents.json")
            .count();
        assert!(failed > 0, "at least one advertise failure expected");
    }

    #[test]
    fn republish_advertises_user_agent_chat_ability_under_user_agent_owner() {
        // Reproduces the gap caught by an end-to-end audit: when a
        // user installs a claude-code agent named `alice`, the daemon
        // must advertise `alice.chat` (and any other per-agent
        // verbs from <workspace>/abilities/*.ability.toml) so the
        // EasyNet frontend's Abilities catalog can list it AND the
        // backend can route invokes back to alice. Pre-fix the LLM
        // profile only published the generic conversation/session/
        // meta/skill prefixes, so `alice.chat` never reached the
        // realm directory and the UI could not see it.
        let _h = HomeGuard::new();

        // Persist an `alice` AgentEntry into the registry so that
        // `load_agents()` inside republish_with_minter sees it.
        let mut reg = crate::registry::agents::AgentRegistry::default();
        reg.agents.insert(
            "alice".to_string(),
            crate::registry::agents::AgentEntry::new(
                crate::registry::agents::AgentType::ClaudeCode,
                None,
            ),
        );
        crate::registry::agents::save_agents(&reg).expect("save alice into registry");

        // Plan: realm joined, alice listed as an LLM sub-agent so
        // bootstrap mints a URA for her.
        let plan = BootstrapPlan {
            realm: "acme".into(),
            user_id: "alice".into(),
            host_device_uri: "easynet:///r/acme/device/01DEV".into(),
            consent: false,
            policy: false,
            mcp: false,
            llm_sub_agents: vec![LlmSubAgent {
                name: "alice".into(),
                agent_type_display: "claude-code".into(),
            }],
        };
        let invoker = CountingInvoker::new(good_reply());
        let outcomes = republish_with_minter(&invoker, "tenant", &plan, &CountingMinter::new());

        // Locate the `alice` URA via the persisted local-agents file.
        let file_back = local_agents::load().expect("load local-agents.json");
        let alice_uri = &file_back
            .hosted_agents
            .iter()
            .find(|e| e.profile == "llm" && e.name == "alice")
            .expect("bootstrap must have minted a URA for alice")
            .agent_uri;

        // Find the advertise_abilities call whose payload's
        // `agent_uri` is alice's URA, and assert `alice.chat`
        // appears in its abilities list. The daemon may pack
        // multiple abilities per call; we scan, not require the
        // first match.
        let calls = invoker.calls();
        let alice_advert = calls
            .iter()
            .find(|(u, p)| {
                u.contains("federation.advertise_abilities@1")
                    && p["agent_uri"].as_str() == Some(alice_uri.as_str())
            })
            .unwrap_or_else(|| {
                panic!(
                    "expected an advertise_abilities call owned by {alice_uri:?}; \
                     resource_seq = {:?}",
                    calls.iter().map(|(u, _)| u).collect::<Vec<_>>()
                )
            });
        let abilities = alice_advert.1["abilities"]
            .as_array()
            .expect("abilities array on advertise_abilities payload");
        let names: Vec<&str> = abilities
            .iter()
            .filter_map(|a| a["name"].as_str())
            .collect();
        assert!(
            names.iter().any(|n| *n == "alice.chat"),
            "alice.chat must appear in advertised abilities for {alice_uri:?}; got names = {names:?}"
        );

        // Sanity: outcomes carry one row per advertise_abilities
        // group; alice's row must be Ok.
        let alice_row = outcomes
            .iter()
            .find(|o| o.agent_uri == *alice_uri && o.label.starts_with("abilities/"))
            .expect("alice's abilities-advertise outcome row must exist");
        assert!(
            alice_row.result.is_ok(),
            "alice abilities advertise: {alice_row:?}"
        );
    }

    #[test]
    fn republish_does_not_lose_device_descriptors_when_user_agent_added() {
        // Regression guard: stitching per-agent descriptors into the
        // existing list must not displace device-level ones. If a
        // refactor accidentally replaces (rather than appends) the
        // descriptors Vec, the device-profile abilities (fs.read,
        // shell.run, …) would silently drop off the wire.
        let _h = HomeGuard::new();
        let mut reg = crate::registry::agents::AgentRegistry::default();
        reg.agents.insert(
            "alice".into(),
            crate::registry::agents::AgentEntry::new(
                crate::registry::agents::AgentType::ClaudeCode,
                None,
            ),
        );
        crate::registry::agents::save_agents(&reg).unwrap();
        let plan = BootstrapPlan {
            realm: "acme".into(),
            user_id: "alice".into(),
            host_device_uri: "easynet:///r/acme/device/01DEV".into(),
            consent: false,
            policy: false,
            mcp: false,
            llm_sub_agents: vec![LlmSubAgent {
                name: "alice".into(),
                agent_type_display: "claude-code".into(),
            }],
        };
        let invoker = CountingInvoker::new(good_reply());
        let _ = republish_with_minter(&invoker, "tenant", &plan, &CountingMinter::new());

        // The device-owner advertise must still carry at least one
        // device-level ability (fs.read is the canary — it has been
        // in the device profile since Tier 2.5 baseline locomotion).
        // URI v4.1.4: device-profile self-URI uses the `device/` role
        // segment (was the v1 `agent/` shape).
        let calls = invoker.calls();
        let device_advert = calls
            .iter()
            .find(|(u, p)| {
                u.contains("federation.advertise_abilities@1")
                    && p["agent_uri"].as_str() == Some("easynet:///r/acme/device/01DEV")
            })
            .expect("device-owner advertise_abilities call must still exist");
        let abilities = device_advert.1["abilities"]
            .as_array()
            .expect("abilities array on device advertise");
        let names: Vec<&str> = abilities
            .iter()
            .filter_map(|a| a["name"].as_str())
            .collect();
        assert!(
            names.iter().any(|n| *n == "fs.read"),
            "device descriptors must survive per-agent stitch; got names = {names:?}"
        );
    }

    #[test]
    fn unpublish_targets_federation_revoke_resource_uri() {
        let invoker = CountingInvoker::new(good_reply());
        let outcome = unpublish_abilities_via_revoke(
            &invoker,
            "tenant",
            "acme",
            "easynet:///r/acme/agent/01OLD",
            "operator removed",
        );
        assert!(outcome.result.is_ok());
        let calls = invoker.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].0.contains("federation.revoke@1"));
        assert_eq!(calls[0].1["agent_uri"], "easynet:///r/acme/agent/01OLD");
        assert_eq!(calls[0].1["reason"], "operator removed");
    }

    #[test]
    fn republish_persists_local_agents_file_with_minted_uris() {
        let _h = HomeGuard::new();
        let invoker = CountingInvoker::new(good_reply());
        let plan = plan_for("acme", "easynet:///r/acme/agent/01DEV");
        let _ = republish_with_minter(&invoker, "tenant", &plan, &CountingMinter::new());
        let calls = invoker.calls();
        // Find the consent-hosted advertise and assert the URA shape.
        let consent_call = calls.iter().find(|(u, p)| {
            u.contains("federation.advertise_agent")
                && p["signing_authority"]["kind"] == "hosted_by"
                && p["agent_uri"].as_str().unwrap().contains("consent-default")
        });
        assert!(
            consent_call.is_some(),
            "expected a hosted_by advertise for consent/default, got calls = {calls:#?}"
        );

        // Persistence end-to-end: read local-agents.json back from
        // the isolated $HOME and confirm the consent + llm rows
        // landed with stable URAs.
        let file_back = local_agents::load().expect("load after save must succeed");
        assert_eq!(
            file_back.host_device_agent_uri,
            "easynet:///r/acme/agent/01DEV"
        );
        let consent_row = file_back
            .hosted_agents
            .iter()
            .find(|e| e.profile == "consent" && e.name == "default")
            .expect("consent/default row must be persisted");
        assert!(consent_row.agent_uri.contains("consent-default"));
        let llm_row = file_back
            .hosted_agents
            .iter()
            .find(|e| e.profile == "llm" && e.name == "claude")
            .expect("llm/claude row must be persisted");
        assert!(llm_row.agent_uri.contains("llm-claude"));
    }

    #[test]
    fn second_republish_reuses_persisted_uris_no_duplicate_advertise() {
        let _h = HomeGuard::new();
        let plan = plan_for("acme", "easynet:///r/acme/agent/01DEV");
        let invoker_a = CountingInvoker::new(good_reply());
        let _ = republish_with_minter(&invoker_a, "tenant", &plan, &CountingMinter::new());
        let first_calls = invoker_a.calls();
        let consent_uri_v1 = first_calls
            .iter()
            .find_map(|(u, p)| {
                if u.contains("federation.advertise_agent")
                    && p["signing_authority"]["kind"] == "hosted_by"
                    && p["agent_uri"].as_str().unwrap().contains("consent-default")
                {
                    p["agent_uri"].as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .expect("first run must have advertised consent/default");

        // Second run with a fresh minter — if the persistence path
        // works, we must NOT mint a new URI; the second advertise
        // must carry the same URA as the first.
        let invoker_b = CountingInvoker::new(good_reply());
        let _ = republish_with_minter(&invoker_b, "tenant", &plan, &CountingMinter::new());
        let consent_uri_v2 = invoker_b
            .calls()
            .iter()
            .find_map(|(u, p)| {
                if u.contains("federation.advertise_agent")
                    && p["signing_authority"]["kind"] == "hosted_by"
                    && p["agent_uri"].as_str().unwrap().contains("consent-default")
                {
                    p["agent_uri"].as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .expect("second run must still advertise consent/default");

        assert_eq!(
            consent_uri_v1, consent_uri_v2,
            "second republish must reuse the persisted URA for consent/default"
        );
    }

    #[test]
    fn register_local_tools_uses_canonical_runtime_admin_uri() {
        // The bridge requires a 5-segment /r URI, and the runtime
        // intercepts `runtime.*` ability names before admission/
        // membership gates. We pin both shape constraints here so
        // future refactors of the URI builder don't quietly break
        // the dispatch registration.
        let _h = HomeGuard::new();
        let invoker = CountingInvoker::new(serde_json::json!({"ack": true}));
        let outcomes = register_local_tools_via_runtime(
            &invoker,
            "tenant",
            "acme",
            "node-01DEV",
            "ipc:///tmp/runtime-dispatch-test.sock",
        );

        // At least one ability must have been published —
        // device-profile abilities are unconditional.
        assert!(
            !outcomes.is_empty(),
            "register must walk at least one ability"
        );
        // Every call must hit the canonical hub-style runtime admin
        // resource URI; ability_name on the wire derives from the
        // URI tail, so a wrong shape silently misroutes.
        let calls = invoker.calls();
        assert_eq!(calls.len(), outcomes.len(), "1 call per ability");
        for (uri, payload) in &calls {
            assert_eq!(
                uri,
                "easynet:///r/prv/hub/acme/abilities/runtime.register_local_tool@1?tenant_id=tenant",
                "register URI must be the canonical 5-segment runtime admin URI"
            );
            assert_eq!(payload["tenant_id"], "tenant");
            assert_eq!(payload["node_id"], "node-01DEV");
            assert_eq!(
                payload["dispatch_endpoint"],
                "ipc:///tmp/runtime-dispatch-test.sock"
            );
            assert!(
                payload["tool_name"].as_str().is_some_and(|s| !s.is_empty()),
                "tool_name must be present and non-empty"
            );
        }
        for o in &outcomes {
            assert!(
                o.result.is_ok(),
                "every register call should succeed: {o:?}"
            );
        }
    }

    #[test]
    fn register_local_tools_surfaces_failure_per_call() {
        let _h = HomeGuard::new();
        let outcomes = register_local_tools_via_runtime(
            &FailingInvoker,
            "tenant",
            "acme",
            "node-x",
            "ipc:///tmp/x.sock",
        );
        assert!(!outcomes.is_empty());
        assert!(
            outcomes.iter().all(|o| o.result.is_err()),
            "every register call must surface its transport error"
        );
    }
}

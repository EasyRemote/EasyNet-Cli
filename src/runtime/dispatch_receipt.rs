// EasyNet CLI — Dispatch-side receipt header builder (P4.8c)
// =============================================================
//
// File: src/runtime/dispatch_receipt.rs
//
// When the IPC dispatcher returns a result for a local ability, it
// may attach a legacy §A12 hosted-agent receipt header so older
// callers can verify the local authority projection behind the result.
// This module centralises the
// "ability_name + local-agents.json + host_device_ura →
// HostedAgentReceiptHeader" lookup that the proxy needs on every
// successful dispatch.
//
// Why a dedicated module
// ----------------------
// The mapping has three inputs that must agree:
//
//   1. The authority/projection classification for the ability.
//   2. The hosted Agent URA recorded in `local-agents.json`.
//   3. The host device-profile URA from the same file.
//
// Inlining this lookup into a transport adapter would couple that
// adapter to every profile module. Wrapping it here keeps dispatch
// responsibilities small.
//
// What this module DOES
// ---------------------
// `header_for_ability(ability_name, &local_agents_file)` returns
// the right header for an ability whose authority projection the file knows about.
// Returns `None` for abilities the file cannot map (e.g. before
// the daemon has joined a realm); the dispatcher then omits the
// optional `receipt_header` field and the wire is unchanged from
// the pre-RFC behaviour.
//
// What this module does NOT do
// -----------------------------
// - Does not own private keys. Callers must provide the host
//   attestation signature for hosted receipts. If they cannot, this
//   module returns `None` rather than emitting an unverifiable header.
// - Does not load local-agents.json itself. Callers pass the file
//   (loaded once at startup or refreshed on advertise) so this
//   module stays pure.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use easynet_axon::invocation::audit::HostedAgentReceiptHeader;

use crate::daemon::ability::catalog::profiles::{DEFAULT_CONSENT_AGENT_ID, DEFAULT_MCP_AGENT_ID};
use crate::persistence::local_agents::{lookup_hosted_ura, LocalAgentsFile};
use crate::runtime::ability_dispatch::OwnerKind;

pub trait HostAttestationProvider {
    fn host_attestation(&self, callee_ura: &str, host_ura: &str) -> Option<Vec<u8>>;
}

impl<F> HostAttestationProvider for F
where
    F: Fn(&str, &str) -> Option<Vec<u8>>,
{
    fn host_attestation(&self, callee_ura: &str, host_ura: &str) -> Option<Vec<u8>> {
        self(callee_ura, host_ura)
    }
}

struct NoHostAttestation;

impl HostAttestationProvider for NoHostAttestation {
    fn host_attestation(&self, _callee_ura: &str, _host_ura: &str) -> Option<Vec<u8>> {
        None
    }
}

/// Build a receipt header for the given ability, given the daemon's
/// local-agents.json snapshot. Returns:
///
///   * `Some(Selfsigned)` when the ability is governed by device
///     authority and the file has the host URA.
///   * `Some(HostedBy)` when the ability is dispatched through a hosted
///     profile projection (consent / mcp / llm) and the file has both
///     the hosted URA and the host URA.
///   * `None` otherwise — the dispatcher omits the receipt_header
///     field and the wire is unchanged from pre-RFC behaviour.
///
/// The caller may also pass `Some(sub_agent_name)` when the ability
/// is in the `skill.*` / `conversation.*` namespace and the
/// dispatch context knows which sub-agent projected it (chat dispatch
/// has this context; static system abilities don't).
pub fn header_for_ability(
    ability_name: &str,
    file: &LocalAgentsFile,
    llm_sub_agent_name: Option<&str>,
) -> Option<HostedAgentReceiptHeader> {
    header_for_ability_with_attestation(ability_name, file, llm_sub_agent_name, &NoHostAttestation)
}

pub fn header_for_ability_with_attestation(
    ability_name: &str,
    file: &LocalAgentsFile,
    llm_sub_agent_name: Option<&str>,
    attestation_provider: &impl HostAttestationProvider,
) -> Option<HostedAgentReceiptHeader> {
    let host_ura = file.host_device_agent_ura.as_str();
    if host_ura.is_empty() {
        return None;
    }

    let (profile_key, name) =
        match crate::daemon::ability::catalog::system_ability_owner(ability_name) {
            Some(OwnerKind::Device) => {
                // Self-signed: device authority projected through the host profile.
                return HostedAgentReceiptHeader::new_selfsigned(host_ura).ok();
            }
            Some(OwnerKind::Agent(agent_id)) => hosted_system_profile_for_agent_id(&agent_id)?,
            Some(OwnerKind::Hub) | Some(OwnerKind::User(_)) | None => {
                llm_dynamic_profile_for_ability(ability_name, llm_sub_agent_name)?
            }
        };

    let callee_ura = lookup_hosted_ura(file, profile_key, name)?;
    let host_attestation = attestation_provider.host_attestation(&callee_ura, host_ura)?;
    HostedAgentReceiptHeader::new_hosted(callee_ura, host_ura, host_attestation).ok()
}

fn hosted_system_profile_for_agent_id(agent_id: &str) -> Option<(&'static str, &'static str)> {
    match agent_id {
        DEFAULT_CONSENT_AGENT_ID => Some(("consent", "default")),
        DEFAULT_MCP_AGENT_ID => Some(("mcp", "default")),
        _ => None,
    }
}

fn llm_dynamic_profile_for_ability<'a>(
    ability_name: &'a str,
    llm_sub_agent_name: Option<&'a str>,
) -> Option<(&'static str, &'a str)> {
    if let Some((agent, "chat")) = ability_name.split_once('.') {
        return Some(("llm", agent));
    }

    if is_llm_contextual_ability(ability_name) {
        return llm_sub_agent_name.map(|name| ("llm", name));
    }

    None
}

fn is_llm_contextual_ability(ability_name: &str) -> bool {
    ability_name.starts_with("conversation.")
        || ability_name.starts_with("session.")
        || ability_name.starts_with("skill.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::test_support::HomeGuard;
    use crate::persistence::local_agents::{upsert_hosted_agent, LocalAgentsFile};
    use easynet_axon::invocation::audit::SigningModel;

    /// Build the test agents file together with a [`HomeGuard`]. The header
    /// builder resolves ability owners through the process-global static
    /// catalog, whose registration reads `HOME` / `EASYNET_PAGES_USER`.
    /// Without the guard, a parallel HomeGuard-holding test in another module
    /// can mutate those vars mid-sync and make the static-runtime sync panic.
    /// Returning the guard from the universal setup helper guarantees every
    /// test in this module — present and future — serializes against it.
    fn file_with(host: &str) -> (HomeGuard, LocalAgentsFile) {
        let guard = HomeGuard::new();
        let file = LocalAgentsFile {
            host_device_agent_ura: host.into(),
            hosted_agents: Vec::new(),
        };
        (guard, file)
    }

    fn test_attestation(callee: &str, host: &str) -> Option<Vec<u8>> {
        let signing_key = easynet_axon::invocation::signing_key_from_bytes(&[0xA7; 32]);
        Some(easynet_axon::invocation::sign_host_attestation(
            &signing_key,
            callee,
            host,
        ))
    }

    #[test]
    fn device_ability_emits_selfsigned_header() {
        let (_home, file) = file_with("easynet:///r/acme/device/01DEV");
        let h = header_for_ability("observe.health", &file, None)
            .expect("device ability must produce a header");
        assert_eq!(h.callee_agent_ura, "easynet:///r/acme/device/01DEV");
        assert_eq!(h.signer_agent_ura, "easynet:///r/acme/device/01DEV");
        assert_eq!(h.model, SigningModel::Selfsigned);
    }

    #[test]
    fn consent_ability_emits_hosted_by_header_when_ura_persisted() {
        let (_home, mut file) = file_with("easynet:///r/acme/device/01DEV");
        upsert_hosted_agent(
            &mut file,
            "consent",
            "default",
            "easynet:///r/acme/agent/u1.01CON",
        );
        let h = header_for_ability_with_attestation(
            "consent.subscribe",
            &file,
            None,
            &test_attestation,
        )
        .expect("consent ability must produce a header");
        assert_eq!(h.callee_agent_ura, "easynet:///r/acme/agent/u1.01CON");
        assert_eq!(h.signer_agent_ura, "easynet:///r/acme/device/01DEV");
        match &h.model {
            SigningModel::HostedBy {
                host_ura,
                host_attestation,
            } => {
                assert_eq!(host_ura, "easynet:///r/acme/device/01DEV");
                assert!(!host_attestation.is_empty());
                assert_eq!(host_attestation.len(), 64);
            }
            _ => panic!("expected HostedBy"),
        }
    }

    #[test]
    fn consent_ability_returns_none_when_consent_ura_missing_from_file() {
        // local-agents.json doesn't yet have a consent row (e.g.
        // bootstrap hasn't run). Better to return None than mint a
        // header pointing at a URA the hub doesn't know about.
        let (_home, file) = file_with("easynet:///r/acme/device/01DEV");
        assert!(
            header_for_ability("consent.request", &file, None).is_none(),
            "missing hosted URA must surface as None, not a fabricated header"
        );
    }

    #[test]
    fn llm_ability_requires_sub_agent_name_to_resolve_owner() {
        let (_home, mut file) = file_with("easynet:///r/acme/device/01DEV");
        upsert_hosted_agent(
            &mut file,
            "llm",
            "claude",
            "easynet:///r/acme/agent/u1.01LLM-claude",
        );
        // Without a sub-agent name we can't decide which LLM owns
        // the ability — return None.
        assert!(header_for_ability("conversation.send", &file, None).is_none());
        // With a name the lookup succeeds.
        let h = header_for_ability_with_attestation(
            "conversation.send",
            &file,
            Some("claude"),
            &test_attestation,
        )
        .expect("named LLM ability must produce a header");
        assert_eq!(
            h.callee_agent_ura,
            "easynet:///r/acme/agent/u1.01LLM-claude"
        );
    }

    #[test]
    fn llm_skill_ability_resolves_to_named_sub_agent() {
        let (_home, mut file) = file_with("easynet:///r/acme/device/01DEV");
        upsert_hosted_agent(
            &mut file,
            "llm",
            "claude",
            "easynet:///r/acme/agent/u1.01LLM",
        );
        let h = header_for_ability_with_attestation(
            "skill.alive-video",
            &file,
            Some("claude"),
            &test_attestation,
        )
        .expect("skill.* with sub_agent must produce a header");
        assert_eq!(h.callee_agent_ura, "easynet:///r/acme/agent/u1.01LLM");
    }

    #[test]
    fn mcp_bridge_ability_emits_hosted_by_header() {
        let (_home, mut file) = file_with("easynet:///r/acme/device/01DEV");
        upsert_hosted_agent(
            &mut file,
            "mcp",
            "default",
            "easynet:///r/acme/agent/u1.01MCP",
        );
        let h = header_for_ability_with_attestation(
            "mcp.bridge.list_tools",
            &file,
            None,
            &test_attestation,
        )
        .expect("mcp.bridge ability must produce a header");
        assert_eq!(h.callee_agent_ura, "easynet:///r/acme/agent/u1.01MCP");
        assert_eq!(h.signer_agent_ura, "easynet:///r/acme/device/01DEV");
    }

    #[test]
    fn empty_host_ura_yields_none_for_every_ability() {
        // Pre-join state: the dispatcher should not emit headers
        // because the host URA itself is unknown.
        let (_home, file) = file_with("");
        assert!(header_for_ability("observe.health", &file, None).is_none());
        assert!(header_for_ability("consent.subscribe", &file, None).is_none());
    }

    #[test]
    fn agent_dot_chat_routes_to_llm_sub_agent_ura() {
        // The wire shape `<agent>.chat` embeds the sub-agent name.
        // The header builder special-cases this so chat dispatch
        // doesn't need out-of-band sub-agent context.
        let (_home, mut file) = file_with("easynet:///r/acme/device/01DEV");
        upsert_hosted_agent(
            &mut file,
            "llm",
            "claude",
            "easynet:///r/acme/agent/u1.01LLM",
        );
        let h = header_for_ability_with_attestation("claude.chat", &file, None, &test_attestation)
            .expect("`<agent>.chat` must produce a header without sub_agent param");
        assert_eq!(h.callee_agent_ura, "easynet:///r/acme/agent/u1.01LLM");
        assert_eq!(h.signer_agent_ura, "easynet:///r/acme/device/01DEV");
    }

    #[test]
    fn agent_dot_chat_returns_none_when_sub_agent_not_registered() {
        let (_home, file) = file_with("easynet:///r/acme/device/01DEV");
        // No `llm/claude` row — no header.
        assert!(header_for_ability("claude.chat", &file, None).is_none());
    }

    #[test]
    fn unowned_ability_yields_none_without_panicking() {
        // Some abilities (e.g. federation.* dispatched against the
        // local-process cache) are not owned by any profile this
        // file knows about — header builder must just return None.
        let (_home, file) = file_with("easynet:///r/acme/device/01DEV");
        assert!(header_for_ability("federation.heartbeat", &file, None).is_none());
        assert!(header_for_ability("totally.unknown", &file, None).is_none());
    }

    #[test]
    fn hosted_ability_without_attestation_yields_none() {
        let (_home, mut file) = file_with("easynet:///r/acme/device/01DEV");
        upsert_hosted_agent(
            &mut file,
            "consent",
            "default",
            "easynet:///r/acme/agent/u1.01CON",
        );
        assert!(
            header_for_ability("consent.subscribe", &file, None).is_none(),
            "hosted receipt headers require a real host attestation"
        );
    }

    #[test]
    fn header_passes_validate() {
        let (_home, file) = file_with("easynet:///r/acme/device/01DEV");
        let h = header_for_ability("observe.health", &file, None).unwrap();
        assert!(
            h.validate().is_ok(),
            "every emitted header must round-trip validate"
        );
    }
}

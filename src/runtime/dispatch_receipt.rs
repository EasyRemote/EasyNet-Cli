// EasyNet CLI — Dispatch-side receipt header builder (P4.8c)
// =============================================================
//
// File: src/runtime/dispatch_receipt.rs
//
// When the IPC dispatcher returns a result for a local ability, it
// must attach a §A12 receipt header so the caller can verify which
// Agent the result came from. This module centralises the
// "ability_name + local-agents.json + host_device_uri →
// HostedAgentReceiptHeader" lookup that the proxy needs on every
// successful dispatch.
//
// Why a dedicated module
// ----------------------
// The mapping has three inputs that must agree:
//
//   1. The profile that owns the ability namespace
//      (`runtime::agents::profiles::*::owns(name)`).
//   2. The hosted Agent URA recorded in `local-agents.json`.
//   3. The host device-profile URA from the same file.
//
// Inlining this lookup into ability_proxy.rs would couple the
// proxy to every profile module. Wrapping it here keeps the
// proxy's responsibilities small (decode wire frames, run the
// dispatcher, render the response).
//
// What this module DOES
// ---------------------
// `header_for_ability(ability_name, &local_agents_file)` returns
// the right header for an ability whose owner the file knows about.
// Returns `None` for abilities the file cannot map (e.g. before
// the daemon has joined a realm); the dispatcher then omits the
// optional `receipt_header` field and the wire is unchanged from
// the pre-RFC behaviour.
//
// What this module does NOT do
// -----------------------------
// - Does not sign anything. P5 backend SDK rewrite ships the
//   actual ed25519 signing pass; until then, the staging shape
//   carries an empty host_attestation, which is honest about the
//   pre-signing state.
// - Does not load local-agents.json itself. Callers pass the file
//   (loaded once at startup or refreshed on advertise) so this
//   module stays pure.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::persistence::local_agents::{lookup_hosted_uri, LocalAgentsFile};
use crate::runtime::agents::profiles::{consent, device, llm, mcp as mcp_profile, policy};
use crate::runtime::hosted_receipt::HostedAgentReceiptHeader;

/// Build a receipt header for the given ability, given the daemon's
/// local-agents.json snapshot. Returns:
///
///   * `Some(Selfsigned)` when the ability is owned by the device-
///     profile and the file has the host URA.
///   * `Some(HostedBy)` when the ability is owned by a hosted
///     profile (consent / policy / mcp / llm) and the file has both
///     the hosted URA and the host URA.
///   * `None` otherwise — the dispatcher omits the receipt_header
///     field and the wire is unchanged from pre-RFC behaviour.
///
/// The caller may also pass `Some(sub_agent_name)` when the ability
/// is in the `skill.*` / `conversation.*` namespace and the
/// dispatch context knows which sub-agent owns it (chat dispatch
/// has this context; static system abilities don't).
pub fn header_for_ability(
    ability_name: &str,
    file: &LocalAgentsFile,
    llm_sub_agent_name: Option<&str>,
) -> Option<HostedAgentReceiptHeader> {
    let host_ura = file.host_device_agent_ura.as_str();
    if host_ura.is_empty() {
        return None;
    }

    if device::owns(ability_name) {
        // Self-signed: the device-profile dispatched its own ability.
        return HostedAgentReceiptHeader::new_selfsigned(host_ura).ok();
    }

    // Special-case `<agent>.chat`: the wire ability name embeds the
    // sub-agent before the verb. Profile prefix matching can't see
    // it because llm::owns checks for `conversation./session./meta./skill.`
    // prefixes only. We recognise the shape here so chat dispatch
    // attaches a header to the right LLM-profile URA.
    if let Some((agent, "chat")) = ability_name.split_once('.') {
        let callee_ura = lookup_hosted_uri(file, "llm", agent)?;
        let attestation_placeholder = b"P5-pending".to_vec();
        return HostedAgentReceiptHeader::new_hosted(callee_ura, host_ura, attestation_placeholder)
            .ok();
    }

    // Hosted abilities: the dispatching Agent is the host (device-
    // profile), but the apparent callee is the hosted profile Agent.
    let (profile_key, name) = if consent::owns(ability_name) {
        ("consent", "default")
    } else if policy::owns(ability_name) {
        ("policy", "default")
    } else if mcp_profile::owns(ability_name) {
        ("mcp", "default")
    } else if llm::owns(ability_name) {
        // LLM sub-agent name comes from the dispatch context (e.g.
        // skill.<v> → name=<agent>). When unknown we cannot map
        // the receipt to a specific Agent URA — return None and
        // let the wire stay quiet rather than guess wrong.
        match llm_sub_agent_name {
            Some(n) => ("llm", n),
            None => return None,
        }
    } else {
        return None;
    };

    let callee_ura = lookup_hosted_uri(file, profile_key, name)?;
    // host_attestation is the staging-mode placeholder until P5
    // signing lands. Empty would fail the constructor's invariant
    // check, so we use a single-byte sentinel that flags "P5 not
    // yet wired" without breaking the schema.
    let attestation_placeholder = b"P5-pending".to_vec();
    HostedAgentReceiptHeader::new_hosted(callee_ura, host_ura, attestation_placeholder).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::local_agents::{upsert_hosted_agent, LocalAgentsFile};
    use crate::runtime::hosted_receipt::SigningModel;

    fn file_with(host: &str) -> LocalAgentsFile {
        LocalAgentsFile {
            host_device_agent_ura: host.into(),
            hosted_agents: Vec::new(),
        }
    }

    #[test]
    fn device_ability_emits_selfsigned_header() {
        let file = file_with("easynet:///r/acme/device/01DEV");
        let h = header_for_ability("device.observe.health", &file, None)
            .expect("device ability must produce a header");
        assert_eq!(h.callee_agent_ura, "easynet:///r/acme/device/01DEV");
        assert_eq!(h.signer_agent_ura, "easynet:///r/acme/device/01DEV");
        assert_eq!(h.model, SigningModel::Selfsigned);
    }

    #[test]
    fn consent_ability_emits_hosted_by_header_when_uri_persisted() {
        let mut file = file_with("easynet:///r/acme/device/01DEV");
        upsert_hosted_agent(
            &mut file,
            "consent",
            "default",
            "easynet:///r/acme/agent/u1.01CON",
        );
        let h = header_for_ability("device.consent.subscribe", &file, None)
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
            }
            _ => panic!("expected HostedBy"),
        }
    }

    #[test]
    fn consent_ability_returns_none_when_consent_uri_missing_from_file() {
        // local-agents.json doesn't yet have a consent row (e.g.
        // bootstrap hasn't run). Better to return None than mint a
        // header pointing at a URA the hub doesn't know about.
        let file = file_with("easynet:///r/acme/device/01DEV");
        assert!(
            header_for_ability("device.consent.request", &file, None).is_none(),
            "missing hosted URA must surface as None, not a fabricated header"
        );
    }

    #[test]
    fn llm_ability_requires_sub_agent_name_to_resolve_owner() {
        let mut file = file_with("easynet:///r/acme/device/01DEV");
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
        let h = header_for_ability("conversation.send", &file, Some("claude"))
            .expect("named LLM ability must produce a header");
        assert_eq!(
            h.callee_agent_ura,
            "easynet:///r/acme/agent/u1.01LLM-claude"
        );
    }

    #[test]
    fn llm_skill_ability_resolves_to_named_sub_agent() {
        let mut file = file_with("easynet:///r/acme/device/01DEV");
        upsert_hosted_agent(
            &mut file,
            "llm",
            "claude",
            "easynet:///r/acme/agent/u1.01LLM",
        );
        let h = header_for_ability("skill.alive-video", &file, Some("claude"))
            .expect("skill.* with sub_agent must produce a header");
        assert_eq!(h.callee_agent_ura, "easynet:///r/acme/agent/u1.01LLM");
    }

    #[test]
    fn mcp_bridge_ability_emits_hosted_by_header() {
        let mut file = file_with("easynet:///r/acme/device/01DEV");
        upsert_hosted_agent(
            &mut file,
            "mcp",
            "default",
            "easynet:///r/acme/agent/u1.01MCP",
        );
        let h = header_for_ability("device.mcp.bridge.list_tools", &file, None)
            .expect("mcp.bridge ability must produce a header");
        assert_eq!(h.callee_agent_ura, "easynet:///r/acme/agent/u1.01MCP");
        assert_eq!(h.signer_agent_ura, "easynet:///r/acme/device/01DEV");
    }

    #[test]
    fn empty_host_ura_yields_none_for_every_ability() {
        // Pre-join state: the dispatcher should not emit headers
        // because the host URA itself is unknown.
        let file = file_with("");
        assert!(header_for_ability("device.observe.health", &file, None).is_none());
        assert!(header_for_ability("device.consent.subscribe", &file, None).is_none());
    }

    #[test]
    fn agent_dot_chat_routes_to_llm_sub_agent_ura() {
        // The wire shape `<agent>.chat` embeds the sub-agent name.
        // The header builder special-cases this so chat dispatch
        // doesn't need out-of-band sub-agent context.
        let mut file = file_with("easynet:///r/acme/device/01DEV");
        upsert_hosted_agent(
            &mut file,
            "llm",
            "claude",
            "easynet:///r/acme/agent/u1.01LLM",
        );
        let h = header_for_ability("claude.chat", &file, None)
            .expect("`<agent>.chat` must produce a header without sub_agent param");
        assert_eq!(h.callee_agent_ura, "easynet:///r/acme/agent/u1.01LLM");
        assert_eq!(h.signer_agent_ura, "easynet:///r/acme/device/01DEV");
    }

    #[test]
    fn agent_dot_chat_returns_none_when_sub_agent_not_registered() {
        let file = file_with("easynet:///r/acme/device/01DEV");
        // No `llm/claude` row — no header.
        assert!(header_for_ability("claude.chat", &file, None).is_none());
    }

    #[test]
    fn unowned_ability_yields_none_without_panicking() {
        // Some abilities (e.g. federation.* dispatched against the
        // local-process cache) are not owned by any profile this
        // file knows about — header builder must just return None.
        let file = file_with("easynet:///r/acme/device/01DEV");
        assert!(header_for_ability("federation.heartbeat", &file, None).is_none());
        assert!(header_for_ability("totally.unknown", &file, None).is_none());
    }

    #[test]
    fn header_passes_validate() {
        let file = file_with("easynet:///r/acme/device/01DEV");
        let h = header_for_ability("device.observe.health", &file, None).unwrap();
        assert!(
            h.validate().is_ok(),
            "every emitted header must round-trip validate"
        );
    }
}

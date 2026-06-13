// EasyNet CLI — `identity.get_trust` / `identity.set_trust`
// ===========================================================
//
// File: src/runtime/agents/trust_ability.rs
// Description: The trust-level directory abilities (seven-axes W2
//              T2.1) — the RFC-001 restatement of the former
//              `GetNodeTrust`/`SetNodeTrust` RPCs, hosted by this
//              daemon as flat `identity.*` system abilities (the
//              namespace is already non-gating, kernel.rs).
//
// Ontology (D8 default, flagged in the spec for CTO ratification):
// the trust subject is the *Agent URA* — the identity that answers
// for receipts. The Axon enforcement gate (resilience.rs:711,715)
// consumes a node-level projection; deriving agent→node is the
// daemon's job and happens at the consumption edge, never by storing
// node rows here.
//
// Two planes, deliberately distinct (commit-plan-2 invariant 8 /
// D10): the realm trust ANCHOR answers "whose keys does admission
// accept"; this LEVEL directory answers "once accepted, how far do
// we trust them". `easynet trust show` reads the anchor;
// `easynet trust level …` reads/writes this directory.
//
// `set_trust` is an ordinary invocation: it arrives through the
// daemon's Invocation surface, so the call itself is admitted,
// ledgered, and receipted like any other ability — the directory
// write is the *effect*, the receipt chain is the *record*.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::persistence::trust_levels;
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, EnvelopeContext, OwnerKind};

pub const GET_TRUST: &str = "identity.get_trust";
pub const SET_TRUST: &str = "identity.set_trust";

/// Baseline reported for subjects with no explicit ruling. Absence
/// of a directory entry is NOT a stored level — the response says so
/// via `source: "default"`.
const DEFAULT_LEVEL: &str = "STANDARD";

pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner(GET_TRUST, OwnerKind::Device, Arc::new(get_trust_handler));
    reg.register_rpc_with_envelope_and_owner(
        SET_TRUST,
        OwnerKind::Device,
        Arc::new(set_trust_handler),
    );
}

/// `{agent_ura}` → the current ruling, or the baseline when none
/// exists (`source` tells the caller which one they got).
fn get_trust_handler(args: Value) -> anyhow::Result<Value> {
    let subject = require_agent_ura(&args)?;
    let directory = trust_levels::load()?;
    Ok(match directory.get(&subject) {
        Some(record) => json!({
            "subject": subject,
            "trust_level": record.trust_level,
            "source": "device-directory",
            "updated_at": record.updated_at,
            "updated_by_invocation": record.updated_by_invocation,
        }),
        None => json!({
            "subject": subject,
            "trust_level": DEFAULT_LEVEL,
            "source": "default",
        }),
    })
}

/// `{agent_ura, trust_level}` → persists the ruling, reports the
/// previous one so the receipt tells the whole transition.
fn set_trust_handler(env: EnvelopeContext, args: Value) -> anyhow::Result<Value> {
    let subject = require_agent_ura(&args)?;
    let raw_level = args
        .get("trust_level")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("identity.set_trust requires `trust_level`"))?;
    let level = canonical_level(raw_level)?;

    let mut directory = trust_levels::load()?;
    let previous = directory.upsert(
        &subject,
        level,
        &chrono::Local::now().to_rfc3339(),
        env.invocation_id.clone(),
    );
    trust_levels::save(&directory)?;

    Ok(json!({
        "subject": subject,
        "trust_level": level,
        "previous": previous,
        "source": "device-directory",
        "updated_by_invocation": env.invocation_id,
    }))
}

/// Validate the D8-default subject shape: a canonical Agent URA that
/// round-trips the Axon parser. Anything else gets a precise refusal
/// — never a guess.
fn require_agent_ura(args: &Value) -> anyhow::Result<String> {
    let ura = args
        .get("agent_ura")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "trust abilities take `agent_ura` (RFC-001 restatement signature \
                 `identity.get_trust{{agent_ura}}`)"
            )
        })?;
    let parsed = crate::ura::parse_ura(ura)
        .map_err(|e| anyhow::anyhow!("invalid agent_ura {ura:?}: {e}"))?;
    if parsed.kind != crate::ura::URAKind::Agent {
        anyhow::bail!(
            "trust subject must be an Agent URA (D8 ruling); got a /{:?}/ URA — \
             node-level projections are derived at the enforcement edge, not stored",
            parsed.kind
        );
    }
    Ok(ura.to_string())
}

/// Rank a canonical level name by the pb enum's pinned wire integer.
/// Shared with the policy matcher so trust comparisons have exactly
/// one source. `None` for names outside the enum.
#[cfg(feature = "axon-pb")]
pub(crate) fn level_rank(name: &str) -> Option<i32> {
    use easynet_axon::pb::axon::v1::TrustLevel;
    TrustLevel::from_str_name(&format!("TRUST_LEVEL_{}", name.to_ascii_uppercase()))
        .filter(|l| *l != TrustLevel::Unspecified)
        .map(|l| l as i32)
}

/// Canonical short level name, validated against the Axon pb
/// `TrustLevel` enum — the one protocol truth.
#[cfg(feature = "axon-pb")]
fn canonical_level(raw: &str) -> anyhow::Result<&'static str> {
    use easynet_axon::pb::axon::v1::TrustLevel;
    let name = format!("TRUST_LEVEL_{}", raw.to_ascii_uppercase());
    let level = TrustLevel::from_str_name(&name)
        .filter(|l| *l != TrustLevel::Unspecified)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unknown trust level {raw:?}; expected one of \
                 untrusted | probation | standard | elevated | privileged"
            )
        })?;
    Ok(level
        .as_str_name()
        .strip_prefix("TRUST_LEVEL_")
        .expect("pb TrustLevel names carry the TRUST_LEVEL_ prefix"))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn level_rank(_name: &str) -> Option<i32> {
    None
}

#[cfg(not(feature = "axon-pb"))]
fn canonical_level(_raw: &str) -> anyhow::Result<&'static str> {
    anyhow::bail!(
        "trust-level validation requires the `axon-pb` feature; rebuild with \
         `cargo build --features axon-pb`"
    )
}

/// The level a subject is treated as: its recorded ruling, or the
/// `STANDARD` baseline — the same semantics `identity.get_trust`
/// reports, kept here so every consumer shares one definition.
pub(crate) fn effective_level(agent_ura: &str) -> String {
    trust_levels::load()
        .ok()
        .and_then(|d| d.get(agent_ura).map(|r| r.trust_level.clone()))
        .unwrap_or_else(|| DEFAULT_LEVEL.to_string())
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;
    use crate::facade::cli::test_support::HomeGuard;

    fn agent_ura() -> String {
        crate::ura::agent_ura("localhost", "dev", "alice")
    }

    #[test]
    fn get_without_ruling_reports_default_baseline() {
        let _g = HomeGuard::new();
        let resp = get_trust_handler(json!({ "agent_ura": agent_ura() })).expect("get");
        assert_eq!(resp["trust_level"], DEFAULT_LEVEL);
        assert_eq!(resp["source"], "default");
    }

    #[test]
    fn set_then_get_round_trips_and_reports_transition() {
        let _g = HomeGuard::new();
        let ura = agent_ura();
        let set = set_trust_handler(
            EnvelopeContext::default(),
            json!({ "agent_ura": ura, "trust_level": "elevated" }),
        )
        .expect("set");
        assert_eq!(set["trust_level"], "ELEVATED");
        assert!(set["previous"].is_null(), "first ruling has no previous");

        let again = set_trust_handler(
            EnvelopeContext {
                invocation_id: Some("inv-trust-2".into()),
                ..Default::default()
            },
            json!({ "agent_ura": ura, "trust_level": "privileged" }),
        )
        .expect("set again");
        assert_eq!(again["previous"], "ELEVATED");
        assert_eq!(again["updated_by_invocation"], "inv-trust-2");

        let get = get_trust_handler(json!({ "agent_ura": ura })).expect("get");
        assert_eq!(get["trust_level"], "PRIVILEGED");
        assert_eq!(get["source"], "device-directory");
        assert_eq!(get["updated_by_invocation"], "inv-trust-2");
    }

    #[test]
    fn unknown_level_is_refused_with_the_full_menu() {
        let _g = HomeGuard::new();
        let err = set_trust_handler(
            EnvelopeContext::default(),
            json!({ "agent_ura": agent_ura(), "trust_level": "max" }),
        )
        .expect_err("bad level must refuse");
        assert!(
            format!("{err}").contains("privileged"),
            "menu in error: {err}"
        );
    }

    #[test]
    fn non_agent_subject_is_refused_per_d8() {
        let _g = HomeGuard::new();
        let device = crate::ura::device_ura("localhost", "dev-1");
        let err = get_trust_handler(json!({ "agent_ura": device }))
            .expect_err("device URA must refuse under D8");
        assert!(format!("{err}").contains("Agent URA"), "got: {err}");
    }

    #[test]
    fn rulings_survive_directory_reload() {
        let _g = HomeGuard::new();
        let ura = agent_ura();
        set_trust_handler(
            EnvelopeContext::default(),
            json!({ "agent_ura": ura, "trust_level": "untrusted" }),
        )
        .expect("set");
        let reloaded = trust_levels::load().expect("reload");
        assert_eq!(
            reloaded.get(&ura).expect("ruling persisted").trust_level,
            "UNTRUSTED"
        );
    }

    #[test]
    fn trust_level_ranks_are_axon_pb_wire_values() {
        use easynet_axon::pb::axon::v1::TrustLevel;

        for raw in [
            "UNTRUSTED",
            "PROBATION",
            "STANDARD",
            "ELEVATED",
            "PRIVILEGED",
        ] {
            let pb = TrustLevel::from_str_name(&format!("TRUST_LEVEL_{raw}"))
                .expect("pb TrustLevel variant exists");
            assert_eq!(level_rank(raw), Some(pb as i32));
            assert_eq!(
                canonical_level(raw.to_ascii_lowercase().as_str()).unwrap(),
                raw
            );
        }
    }
}

// EasyNet CLI — shared remote-device resolution helpers
// ======================================================
//
// File: src/support/remote_device.rs
// Description: Canonical helper for CLI surfaces that forward a call
//              to an explicitly addressed remote runtime owner.
//
// Why this exists
// ---------------
// Remote invocation ingress must not repair product-directory identifiers into
// invocation targets. The canonical runtime tuple names its callee as a URA;
// directory search belongs to discovery UX before invocation planning, not to
// the signed request builder.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::{anyhow, Context};

use crate::core::identity::RuntimeIdentityUra;
use crate::core::ura::URAKind;

/// Resolve the explicit signing identity for a CLI-originated remote request.
///
/// Remote dispatch has no synthetic caller. An unpaired or incomplete CLI
/// identity is rejected before request construction because no canonical signer
/// can own that invocation tuple.
pub(crate) fn require_caller_device_ura_from_credentials() -> anyhow::Result<String> {
    let credentials = crate::daemon::persistence::config::load_credentials().map_err(|error| {
        anyhow!("remote invocation requires paired device credentials: {error}")
    })?;
    caller_device_ura(&credentials)
}

pub(crate) fn caller_device_ura(
    credentials: &crate::daemon::persistence::config::Credentials,
) -> anyhow::Result<String> {
    let realm = credentials.realm.trim();
    let node_id = credentials.node_id.trim();
    if realm.is_empty() || node_id.is_empty() {
        return Err(anyhow!(
            "remote invocation requires paired device credentials with non-empty realm and node_id"
        ));
    }
    Ok(crate::core::ura::device_ura(realm, node_id))
}

/// Resolve a CLI remote target argument into a canonical Device URA.
///
/// Public remote invocation ingress is URA-only. Bare node ids and product
/// directory aliases are rejected before descriptor resolution so the caller
/// cannot accidentally build a signed tuple for a target inferred from stale
/// discovery state.
pub(crate) fn resolve_target_device_ura(node: &str) -> anyhow::Result<String> {
    let trimmed = node.trim();
    if trimmed.is_empty() {
        anyhow::bail!("remote device target must not be empty; pass a canonical Device URA");
    }
    let identity = RuntimeIdentityUra::parse(trimmed).map_err(|error| {
        anyhow!(
            "remote device target {trimmed:?} is not a canonical URA: {error}; \
             pass `easynet:///r/<realm>/device/<id>`"
        )
    })?;
    if identity.kind() != URAKind::Device {
        anyhow::bail!(
            "remote device target {trimmed:?} has kind {}; expected a canonical Device URA",
            identity.kind()
        );
    }
    Ok(identity.into_string())
}

/// Resolve a CLI device-target flag into a canonical Device URA.
///
/// Human-facing commands may keep the ergonomic `local` selector. The selector
/// is consumed at the CLI boundary and never enters daemon ability arguments.
/// Remote targets are URA-only; bare node ids are not repaired from directory
/// state because mutation admission must bind to one explicit runtime owner.
pub(crate) fn resolve_cli_device_target_ura(
    raw: Option<&str>,
    surface: &str,
) -> anyhow::Result<String> {
    let target = raw.unwrap_or("local").trim();
    if target.is_empty() {
        anyhow::bail!("{surface} target must not be empty; pass `local` or a canonical Device URA");
    }
    if target == "local" {
        return crate::daemon::identity::local_invocation::local_device_ura()
            .with_context(|| format!("resolve local {surface} target Device URA"));
    }
    resolve_target_device_ura(target).map_err(|error| {
        anyhow!("{surface} target must be a canonical Device URA; failed to resolve {target:?}: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ura_passes_through() {
        let ura = "easynet:///r/peer/device/node-1";
        let resolved = resolve_target_device_ura(ura).expect("canonical URA");
        assert_eq!(resolved, ura);
    }

    #[test]
    fn bare_node_id_is_rejected_before_directory_lookup() {
        let err = resolve_target_device_ura("node-2")
            .expect_err("bare node id must not be repaired from directory state");
        let msg = err.to_string();
        assert!(
            msg.contains("not a canonical URA"),
            "unexpected error: {msg}"
        );
        assert!(
            msg.contains("easynet:///r/<realm>/device/<id>"),
            "error must explain the canonical recovery path, got: {msg}"
        );
    }

    #[test]
    fn authority_ura_is_not_a_device_target() {
        let err = resolve_target_device_ura("easynet:///r/peer/authority")
            .expect_err("device-specific ingress must reject an Authority URA");
        assert!(
            err.to_string().contains("expected a canonical Device URA"),
            "unexpected target-kind error: {err}"
        );
    }

    #[test]
    fn empty_target_rejects_before_remote_dispatch() {
        let err = resolve_target_device_ura("   ").expect_err("empty target must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("must not be empty"),
            "empty target failure must remain local, got: {msg}"
        );
    }

    #[test]
    fn cli_device_target_rejects_bare_node_id_before_directory_lookup() {
        let err = resolve_cli_device_target_ura(Some("node-2"), "ability deploy")
            .expect_err("bare node id must not be repaired from directory state");
        let msg = err.to_string();
        assert!(
            msg.contains("canonical URA"),
            "unexpected CLI target error: {msg}"
        );
    }
}

// EasyNet CLI — shared remote-device resolution helpers
// ======================================================
//
// File: src/support/remote_device.rs
// Description: Canonical helper for CLI surfaces that accept either a
//              device URA or a bare node UUID and need to forward a
//              call to the right remote device.
//
// Why this exists
// ---------------
// Several CLI surfaces historically open-coded the same routing rule:
//
//   1. If the user passes a canonical `easynet:///r/<realm>/device/<id>`
//      URA, validate and use it directly.
//   2. If the user passes a bare UUID, first ask the local daemon's
//      federated directory (`federation.discover`) whether that node is
//      currently known under a DIFFERENT realm.
//   3. Only if the directory cannot answer, fall back to wrapping the
//      UUID in the caller's local realm.
//
// Before this helper existed, `device show` had the fixed two-stage
// lookup while `auth abilities`, `ability list/show`, and
// `ability exec` still used the old "always wrap in local realm"
// branch. That drift is exactly how a cross-hub UUID regressed in one
// CLI surface after being fixed in another.
//
// The helper centralises the routing rule so every caller keeps the
// same semantics and future bug fixes land in one place.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::anyhow;
use serde_json::Value;

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

/// Resolve a CLI `node` argument into a canonical device URA.
///
/// Resolution order:
/// 1. Canonical URA passes through after strict validation.
/// 2. Bare UUID hits the local daemon's federated directory first so
///    cross-hub devices preserve their real realm.
/// 3. Only if the directory cannot answer do we fall back to wrapping
///    in the local realm from `credentials.json`.
pub(crate) fn resolve_target_device_ura(node: &str) -> anyhow::Result<String> {
    let local_tenant = crate::daemon::persistence::config::load_credentials()
        .ok()
        .and_then(|creds| {
            let tenant = creds.realm.trim();
            if tenant.is_empty() {
                None
            } else {
                Some(tenant.to_string())
            }
        });
    resolve_target_device_ura_with_lookup(
        node,
        local_tenant.as_deref(),
        lookup_node_ura_in_directory,
    )
}

fn resolve_target_device_ura_with_lookup<F>(
    node: &str,
    local_tenant: Option<&str>,
    lookup: F,
) -> anyhow::Result<String>
where
    F: FnOnce(&str) -> Option<String>,
{
    let trimmed = node.trim();
    if crate::core::ura::parse_ura(trimmed).is_ok() {
        return crate::daemon::invocation::routing::remote_invoke::parse_node_ura(trimmed);
    }
    if let Some(ura) = lookup(trimmed) {
        return Ok(ura);
    }
    if let Some(local_tenant) = local_tenant.filter(|tenant| !tenant.is_empty()) {
        return Ok(crate::core::ura::device_ura(local_tenant, trimmed));
    }
    Err(anyhow!(
        "cannot resolve node {trimmed:?}: federation.discover returned no match and \
         no local realm is wired (pair this device first or pass a canonical \
         `easynet:///r/<realm>/device/<id>` URA)"
    ))
}

/// Walk the local daemon's federated directory for a `DirectoryEntry`
/// whose `node_id` equals `node`. Returns the entry's canonical
/// device URA. Best-effort only: discover failures must not block the
/// legacy local-realm fallback.
fn lookup_node_ura_in_directory(node: &str) -> Option<String> {
    let entries =
        crate::daemon::invocation::routing::remote_invoke::invoke_federation_discover(None).ok()?;
    for entry in entries {
        if entry.get("node_id").and_then(Value::as_str) == Some(node) {
            if let Some(ura) = entry.get("agent_ura").and_then(Value::as_str) {
                return Some(ura.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ura_passes_through() {
        let ura = "easynet:///r/peer/device/node-1";
        let resolved = resolve_target_device_ura_with_lookup(ura, Some("local"), |_| None)
            .expect("canonical URA");
        assert_eq!(resolved, ura);
    }

    #[test]
    fn directory_hit_beats_local_realm_fallback() {
        let resolved = resolve_target_device_ura_with_lookup("node-1", Some("realm-a"), |_| {
            Some("easynet:///r/realm-b/device/node-1".to_string())
        })
        .expect("directory hit");
        assert_eq!(resolved, "easynet:///r/realm-b/device/node-1");
    }

    #[test]
    fn local_realm_fallback_is_used_when_directory_misses() {
        let resolved = resolve_target_device_ura_with_lookup("node-2", Some("realm-a"), |_| None)
            .expect("fallback");
        assert_eq!(resolved, "easynet:///r/realm-a/device/node-2");
    }

    #[test]
    fn missing_directory_and_realm_surfaces_actionable_error() {
        let err = resolve_target_device_ura_with_lookup("node-3", None, |_| None)
            .expect_err("missing realm must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("pair this device first"),
            "error must explain the recovery path, got: {msg}"
        );
    }
}

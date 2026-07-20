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
// Several CLI surfaces historically open-coded target resolution:
//
//   1. If the user passes a canonical `easynet:///r/<realm>/device/<id>`
//      URA, validate and use it directly.
//   2. If the user passes a bare UUID, ask the local daemon's federated
//      directory (`federation.discover`) for the node's canonical device URA.
//
// Before this helper existed, `device show` had the fixed two-stage
// lookup while `auth abilities`, `ability list/show`, and
// `ability exec` could drift on how much repair they performed at the edge.
// The resolver now refuses to mint device URAs from bare node IDs: a directory
// miss is unresolved state, not permission to infer a local-realm owner.
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
/// 3. Directory miss fails closed and asks the caller to pass a canonical
///    device URA or refresh federation state.
pub(crate) fn resolve_target_device_ura(node: &str) -> anyhow::Result<String> {
    resolve_target_device_ura_with_lookup(node, lookup_node_ura_in_directory)
}

fn resolve_target_device_ura_with_lookup<F>(node: &str, lookup: F) -> anyhow::Result<String>
where
    F: FnOnce(&str) -> anyhow::Result<Option<String>>,
{
    let trimmed = node.trim();
    if trimmed.is_empty() {
        anyhow::bail!("cannot resolve an empty remote device node argument");
    }
    if crate::core::ura::parse_ura(trimmed).is_ok() {
        return crate::daemon::invocation::routing::remote_invoke::parse_node_ura(trimmed);
    }
    if let Some(ura) = lookup(trimmed)? {
        return Ok(ura);
    }
    Err(anyhow!(
        "cannot resolve node {trimmed:?}: federation.discover returned no matching device; \
         refresh federation state or pass a canonical `easynet:///r/<realm>/device/<id>` URA"
    ))
}

/// Walk the local daemon's federated directory for a `DirectoryEntry`
/// whose `node_id` equals `node`. Returns the entry's canonical
/// device URA.
fn lookup_node_ura_in_directory(node: &str) -> anyhow::Result<Option<String>> {
    let entries =
        crate::daemon::invocation::routing::remote_invoke::invoke_federation_discover(None)?;
    for entry in entries {
        if entry.get("node_id").and_then(Value::as_str) == Some(node) {
            if let Some(ura) = entry.get("agent_ura").and_then(Value::as_str) {
                return Ok(Some(ura.to_string()));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ura_passes_through() {
        let ura = "easynet:///r/peer/device/node-1";
        let resolved =
            resolve_target_device_ura_with_lookup(ura, |_| Ok(None)).expect("canonical URA");
        assert_eq!(resolved, ura);
    }

    #[test]
    fn directory_hit_resolves_bare_node() {
        let resolved = resolve_target_device_ura_with_lookup("node-1", |_| {
            Ok(Some("easynet:///r/realm-b/device/node-1".to_string()))
        })
        .expect("directory hit");
        assert_eq!(resolved, "easynet:///r/realm-b/device/node-1");
    }

    #[test]
    fn directory_miss_does_not_infer_local_realm_device() {
        let err = resolve_target_device_ura_with_lookup("node-2", |_| Ok(None))
            .expect_err("directory miss must fail closed");
        let msg = err.to_string();
        assert!(
            msg.contains("federation.discover returned no matching device"),
            "unexpected error: {msg}"
        );
        assert!(
            msg.contains("canonical `easynet:///r/<realm>/device/<id>`"),
            "error must explain the canonical recovery path, got: {msg}"
        );
    }

    #[test]
    fn directory_failure_propagates_without_local_realm_repair() {
        let err = resolve_target_device_ura_with_lookup("node-3", |_| {
            Err(anyhow::anyhow!("daemon unavailable"))
        })
        .expect_err("directory failure must propagate");
        let msg = err.to_string();
        assert!(
            msg.contains("daemon unavailable"),
            "directory failure must remain visible, got: {msg}"
        );
    }
}

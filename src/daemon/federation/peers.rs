// EasyNet CLI - daemon federation peers cell (PR-N1 commit 10/N)
// =================================================================
//
// File: src/daemon/federation/peers.rs
// Description: A reload-friendly cell holding the daemon's current
//              operator-curated `realm → hub_endpoint` map. Built once
//              at boot from `DaemonConfig::federated_peers`,
//              shared by `DaemonInvocationService::dispatch_federation_
//              forward_invoke`'s cross-realm routing arm.
//
// Why this cell exists
// --------------------
// PR-N1 commit 6/N (boot wiring) snapshotted `DaemonConfig::
// federated_peers` once at boot. That meant operators editing
// `[daemon.federated_peers]` had to restart the daemon for the
// dispatcher to see the new entries — the same SIGHUP gap commit
// 9/N closed for `RealmTrustAnchor` via `SharedTrustAnchor`.
//
// An earlier review explicitly deferred this cell because there
// was no `DaemonConfigCell` infrastructure at the time; this
// commit ships the missing piece. Operators editing
// `~/.easynet/daemon-config.toml` + `kill -HUP <daemon_pid>` now
// see the new `federated_peers` map within ~50ms — same cadence
// as the trust-anchor reload.
//
// Why a separate cell rather than reusing SharedTrustAnchor's
// generic shape: `RealmTrustAnchor` is a complex struct with
// HashMap + invariant checks; `federated_peers` is a tiny
// `BTreeMap<String, String>`. A dedicated cell keeps the
// concerns separated and the SIGHUP reload task focused (one
// path failure does not propagate to the other).
//
// Lock poisoning
// --------------
// Same recovery as `SharedTrustAnchor`: `RwLock::read()` /
// `write()` returning a poison error is recovered via
// `into_inner()`. The map's only invariant is "valid TOML
// parsed at load time"; a poisoned read returns whatever the
// last successful write published.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Shared, reload-friendly cell over the daemon's current
/// `realm → hub_endpoint` map. Cheap to clone (one outer `Arc` bump).
#[derive(Debug, Clone)]
pub struct SharedFederatedPeers {
    inner: Arc<RwLock<Arc<BTreeMap<String, String>>>>,
}

impl SharedFederatedPeers {
    /// Build a cell wrapping `peers`. Production callers pass the
    /// boot-time `DaemonConfig::federated_peers().clone()` result;
    /// tests build small fixtures.
    #[must_use]
    pub fn new(peers: BTreeMap<String, String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Arc::new(peers))),
        }
    }

    /// Snapshot the current peers map as an `Arc`. The caller
    /// holds a stable view even if a concurrent writer publishes
    /// a new map mid-operation; later dispatches see the new one
    /// on their next `snapshot()` call.
    pub fn snapshot(&self) -> Arc<BTreeMap<String, String>> {
        let guard = match self.inner.read() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        Arc::clone(&guard)
    }

    /// Atomically replace the held map with `next`. Lock is held
    /// only for the assignment — the new map's structure is the
    /// caller's responsibility (TOML parse runs on the SIGHUP
    /// reload task before this call).
    pub fn replace(&self, next: BTreeMap<String, String>) {
        let mut guard = match self.inner.write() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = Arc::new(next);
    }
}

impl Default for SharedFederatedPeers {
    fn default() -> Self {
        Self::new(BTreeMap::new())
    }
}

impl From<BTreeMap<String, String>> for SharedFederatedPeers {
    fn from(peers: BTreeMap<String, String>) -> Self {
        Self::new(peers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_returns_current_peers() {
        let mut initial = BTreeMap::new();
        initial.insert("realm-a".to_string(), "https://a:50443".to_string());
        let cell = SharedFederatedPeers::new(initial);
        let snap = cell.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(
            snap.get("realm-a").map(String::as_str),
            Some("https://a:50443")
        );
    }

    #[test]
    fn replace_publishes_new_peers_atomically() {
        let cell = SharedFederatedPeers::default();
        assert!(cell.snapshot().is_empty());

        let mut next = BTreeMap::new();
        next.insert("realm-b".to_string(), "https://b:50443".to_string());
        cell.replace(next);

        let snap = cell.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(snap.contains_key("realm-b"));
    }

    #[test]
    fn snapshots_taken_before_replace_remain_stable() {
        let mut initial = BTreeMap::new();
        initial.insert("old".to_string(), "https://old:1".to_string());
        let cell = SharedFederatedPeers::new(initial);

        let pre = cell.snapshot();
        let mut next = BTreeMap::new();
        next.insert("new".to_string(), "https://new:2".to_string());
        cell.replace(next);

        // Pre-replace snapshot still sees the old map.
        assert!(pre.contains_key("old"));
        assert!(!pre.contains_key("new"));

        // Post-replace snapshot sees the new map.
        let post = cell.snapshot();
        assert!(post.contains_key("new"));
        assert!(!post.contains_key("old"));
    }

    #[test]
    fn clone_shares_underlying_cell() {
        let cell_a = SharedFederatedPeers::default();
        let cell_b = cell_a.clone();

        let mut next = BTreeMap::new();
        next.insert("shared".to_string(), "https://shared:3".to_string());
        cell_a.replace(next);

        // Clone sees the publish through the shared cell.
        assert!(cell_b.snapshot().contains_key("shared"));
    }
}

// EasyNet CLI — Tenant-Scoped Disk Paths
// =======================================
//
// File: src/persistence/tenant_paths.rs
// Description: Single helper that resolves every tenant-scoped on-
//              disk path for the daemon's business state (runs /
//              schedules / discuss-rooms / loops). All readers and
//              writers must route through this function so v2's
//              multi-tenant landing is additive — no magic
//              `.join("default")` strings scattered across the
//              codebase.
//
// Why v1 ships the helper even though there's only one tenant
// ------------------------------------------------------------
// Plan v10.1 recovers from v9's decision to skip the tenant slot:
// a multi-tenant future will need to partition on-disk state, and
// scattering "just use default" across the codebase now means v2's
// refactor will be a hundred-file patch. Introducing the helper in
// v1 reduces that patch to "change one function body" — the same
// shape that makes ABI version negotiation additive.
//
// Directory layout
// ----------------
// All paths under `~/.easynet/tenants/<tenant>/<kind>/`:
//
//   runs/          — agent session runs (existing
//                    `~/.easynet/workspaces/<agent>/runs/` migrates
//                    here in v2; v1 leaves the legacy path alone)
//   schedules/     — cron entries (PR-SCHED)
//   discuss-rooms/ — room membership + transcripts (PR-DISCUSS)
//   loops/         — loop instance status (PR-LOOP)
//
// v1 hard-codes the tenant id to "default" via
// `TenantId::default_v1()`. v2 will thread a real id through the
// IPC handshake.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;

use crate::persistence::config::state_dir;
use crate::runtime::domain::TenantId;

/// Well-known kinds of tenant-scoped storage. Every reader and
/// writer routes through `path_for_tenant` with one of these; no
/// magic strings leak into caller code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantKind {
    Runs,
    Schedules,
    DiscussRooms,
    Loops,
}

impl TenantKind {
    /// Exhaustive list of storage kinds this module owns. Callers that need
    /// to iterate every tenant-scoped directory use this instead of retyping
    /// the enum cases and drifting from `as_dir()`.
    pub const ALL: [Self; 4] = [Self::Runs, Self::Schedules, Self::DiscussRooms, Self::Loops];

    fn as_dir(self) -> &'static str {
        match self {
            Self::Runs => "runs",
            Self::Schedules => "schedules",
            Self::DiscussRooms => "discuss-rooms",
            Self::Loops => "loops",
        }
    }
}

/// Resolve the directory for `(tenant, kind)` under
/// `~/.easynet/tenants/<tenant>/<kind>/`. Does not create the
/// directory; callers that need the dir to exist call `ensure()`.
pub fn path_for_tenant(tenant: &TenantId, kind: TenantKind) -> PathBuf {
    debug_assert!(TenantKind::ALL.contains(&kind));
    state_dir()
        .join("tenants")
        .join(tenant.as_str())
        .join(kind.as_dir())
}

/// Create the directory (and all parents) if missing. Idempotent.
pub fn ensure(tenant: &TenantId, kind: TenantKind) -> std::io::Result<PathBuf> {
    let p = path_for_tenant(tenant, kind);
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tenant_path_lands_under_tenants_default() {
        // The magic string "default" appears exactly once, in
        // TenantId::default_v1(). This test pins that decision:
        // if someone later thinks "why not just omit the tenant
        // directory level when there's only one tenant?", the
        // test catches the regression — a v2 migration script
        // would fail to find the data.
        let p = path_for_tenant(&TenantId::default_v1(), TenantKind::Runs);
        let s = p.to_string_lossy();
        assert!(s.contains("tenants"), "expected `tenants/` segment in {s}");
        assert!(s.contains("default"), "expected `default/` segment in {s}");
        assert!(s.ends_with("runs"), "expected trailing `runs`, got {s}");
    }

    #[test]
    fn every_kind_produces_distinct_path() {
        // Regression guard for a copy-paste bug where two kinds
        // map to the same directory. Would silently corrupt one
        // store with another's writes.
        let t = TenantId::default_v1();
        let paths: Vec<_> = TenantKind::ALL
            .iter()
            .map(|k| path_for_tenant(&t, *k))
            .collect();
        for i in 0..paths.len() {
            for j in (i + 1)..paths.len() {
                assert_ne!(paths[i], paths[j], "kinds {i} and {j} map to the same path");
            }
        }
    }
}

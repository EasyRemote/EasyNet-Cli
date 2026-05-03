// EasyNet CLI — Pages reference system: process-wide publish state
// ================================================================
//
// File: src/runtime/agents/pages/state.rs
// Description: in-memory state of currently-published projects.
//              One `ProjectHandle` per `(user, project)` tuple,
//              keyed in a DashMap so the publish ability and
//              the fetch ability never need to coordinate locks.
//              The handle owns the `OwnedFd` of the published
//              folder for the lifetime of the publish — the
//              read sandbox uses that fd as its kernel-enforced
//              root.
//
// Phase: v0 (RFC-006-B v0.6, INV-2 transitional). Daemon-restart
//        persistence is post-MVP — when the daemon restarts, the
//        map is empty and projects must be re-published.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use dashmap::DashMap;
use once_cell::sync::Lazy;

/// Visibility marker. v0 supports PUBLIC only; PRIVATE/SCOPED
/// reject at the publish boundary with a clear error and are
/// reserved for a later phase (RFC-006-B §post-MVP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    // Private,    // post-MVP
    // Scoped,     // post-MVP
}

impl Visibility {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "public" | "" => Ok(Self::Public),
            "private" | "scoped" => {
                anyhow::bail!(
                    "visibility '{s}' is not yet supported in this MVP; only 'public' is accepted"
                )
            }
            other => anyhow::bail!("unknown visibility: {other}"),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
        }
    }
}

/// Per-publish in-memory state. Owns the folder fd (pinned for
/// `openat2`) and the canonical root (only used to format human
/// output / receipts; the fd is the security boundary).
///
/// The fd is dropped when the handle is removed from
/// `PUBLISHED_PROJECTS` (DashMap drops on `remove`), at which
/// point the kernel reclaims the directory reference and any
/// subsequent fetch fails at registry lookup.
pub struct ProjectHandle {
    pub user: String,
    pub project_id: String,
    pub folder_fd: OwnedFd,
    pub canonical_root: PathBuf,
    pub visibility: Visibility,
    pub file_size_cap: u64,
    pub started_at: SystemTime,
}

/// Process-wide publish registry. DashMap so concurrent fetches
/// over different projects never block each other; key is
/// `(user, project_id)` so two users may pick the same
/// project_id without colliding.
///
/// `Arc<ProjectHandle>` so a fetch handler can clone-out a
/// borrowed handle from the map and proceed without holding the
/// shard lock for the duration of the syscall.
pub static PUBLISHED_PROJECTS: Lazy<DashMap<(String, String), Arc<ProjectHandle>>> =
    Lazy::new(DashMap::new);

/// 100 MiB per file. Out-of-MVP knob; over-cap fetches return a
/// 502 from the Hub. Set as a const so the test matrix can
/// reference it without re-deriving.
pub const DEFAULT_FILE_SIZE_CAP: u64 = 100 * 1024 * 1024;

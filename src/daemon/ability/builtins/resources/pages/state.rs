// EasyNet CLI — Pages reference system: process-wide publish state
// ================================================================
//
// File: src/daemon/ability/builtins/resources/pages/state.rs
// Description: in-memory state of currently-published projects plus
//              the minimal on-disk snapshot used to repopulate the
//              map after a daemon restart.
//              One `ProjectHandle` per `(user, project)` tuple,
//              keyed in a DashMap so the publish ability and
//              the fetch ability never need to coordinate locks.
//              The handle owns the `OwnedFd` of the published
//              folder for the lifetime of the publish — the
//              read sandbox uses that fd as its kernel-enforced
//              root.
//
// Phase: v0.1 (RFC-006-B v0.6, INV-2 transitional). Publish state is
//             snapshotted under `~/.easynet/` so daemon restart no
//             longer discards the user's pages inventory.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use super::sandbox::PublishedFolderHandle;

/// PageVisibility marker. v0 supports PUBLIC only; PRIVATE/SCOPED
/// reject at the publish boundary with a clear error and are
/// reserved for a later phase (RFC-006-B §post-MVP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageVisibility {
    Public,
    // Private,    // post-MVP
    // Scoped,     // post-MVP
}

impl PageVisibility {
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
    pub folder_handle: PublishedFolderHandle,
    pub canonical_root: PathBuf,
    pub visibility: PageVisibility,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PersistedProjectRecord {
    user: String,
    project_id: String,
    folder: PathBuf,
    visibility: String,
    file_size_cap: u64,
    started_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct PersistedProjectRegistry {
    projects: Vec<PersistedProjectRecord>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RestoreSummary {
    pub restored: usize,
    pub skipped: usize,
}

fn registry_path_for_user(user: &str) -> PathBuf {
    let suffix: String = user
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    crate::daemon::persistence::config::state_dir().join(format!("pages-published-{suffix}.json"))
}

fn system_time_to_epoch_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn epoch_ms_to_system_time(ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms)
}

fn snapshot_registry_for_user(user: &str) -> PersistedProjectRegistry {
    let mut projects: Vec<PersistedProjectRecord> = PUBLISHED_PROJECTS
        .iter()
        .filter_map(|entry| {
            let handle = entry.value();
            if handle.user != user {
                return None;
            }
            Some(PersistedProjectRecord {
                user: handle.user.clone(),
                project_id: handle.project_id.clone(),
                folder: handle.canonical_root.clone(),
                visibility: handle.visibility.as_str().to_string(),
                file_size_cap: handle.file_size_cap,
                started_at_ms: system_time_to_epoch_ms(handle.started_at),
            })
        })
        .collect();
    projects.sort_by(|a, b| {
        a.user
            .cmp(&b.user)
            .then_with(|| a.project_id.cmp(&b.project_id))
    });
    PersistedProjectRegistry { projects }
}

pub(crate) fn persist_registry_for_user(user: &str) -> anyhow::Result<()> {
    let path = registry_path_for_user(user);
    let snapshot = snapshot_registry_for_user(user);
    if snapshot.projects.is_empty() {
        if path.exists() {
            fs::remove_file(&path)?;
        }
        return Ok(());
    }
    let dir = crate::daemon::persistence::config::state_dir();
    fs::create_dir_all(&dir)?;
    let bytes = serde_json::to_vec_pretty(&snapshot)?;
    crate::daemon::persistence::config::atomic_write(&path, &bytes)?;
    Ok(())
}

pub(crate) fn restore_published_projects(user: &str) -> anyhow::Result<RestoreSummary> {
    let path = registry_path_for_user(user);
    if !path.exists() {
        return Ok(RestoreSummary::default());
    }
    let registry: PersistedProjectRegistry = serde_json::from_slice(&fs::read(&path)?)?;
    let mut summary = RestoreSummary::default();
    let mut cleaned_snapshot_needed = false;

    for record in registry.projects {
        let key = (record.user.clone(), record.project_id.clone());
        if record.user != user {
            cleaned_snapshot_needed = true;
            summary.skipped += 1;
            continue;
        }
        if PUBLISHED_PROJECTS.contains_key(&key) {
            continue;
        }
        let visibility = match PageVisibility::parse(&record.visibility) {
            Ok(v) => v,
            Err(_) => {
                cleaned_snapshot_needed = true;
                summary.skipped += 1;
                continue;
            }
        };
        let canonical_root = match fs::canonicalize(&record.folder) {
            Ok(path) => path,
            Err(_) => {
                cleaned_snapshot_needed = true;
                summary.skipped += 1;
                continue;
            }
        };
        let folder_handle = match super::sandbox::open_directory(&canonical_root) {
            Ok(handle) => handle,
            Err(_) => {
                cleaned_snapshot_needed = true;
                summary.skipped += 1;
                continue;
            }
        };
        let handle = Arc::new(ProjectHandle {
            user: record.user.clone(),
            project_id: record.project_id.clone(),
            folder_handle,
            canonical_root,
            visibility,
            file_size_cap: record.file_size_cap,
            started_at: epoch_ms_to_system_time(record.started_at_ms),
        });
        PUBLISHED_PROJECTS.insert(key, handle);
        summary.restored += 1;
    }

    if cleaned_snapshot_needed {
        persist_registry_for_user(user)?;
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::dispatch::{AbilityAuthorityContext, AxonAbilityCatalog};
    use serde_json::json;

    fn pages_registry(realm: &str, user: &str) -> Arc<AxonAbilityCatalog> {
        let device_ura = crate::core::ura::device_ura(realm, "pages-test-device");
        let pages_agent = super::super::management_agent_ura(realm, user);
        let authority_context = AbilityAuthorityContext::for_device_authority_root(device_ura)
            .expect("Pages test Device authority")
            .with_declared_agent_authority_root(pages_agent)
            .expect("Pages test Agent authority");
        Arc::new(AxonAbilityCatalog::new_with_runtime_and_authority_context(
            easynet_axon::invocation::LocalRuntime::new(),
            authority_context,
        ))
    }

    fn clear_registry_for_user(user: &str) {
        let keys: Vec<_> = PUBLISHED_PROJECTS
            .iter()
            .filter_map(|entry| {
                let key = entry.key();
                (key.0 == user).then(|| key.clone())
            })
            .collect();
        for key in keys {
            PUBLISHED_PROJECTS.remove(&key);
        }
    }

    #[test]
    fn publish_snapshot_restores_project_after_map_clear() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();

        let user = "pages-restore-alice";
        clear_registry_for_user(user);
        let project_id = "restored-site";
        let folder = tempfile::tempdir().expect("tempdir");
        fs::write(folder.path().join("index.html"), "<h1>restored</h1>").unwrap();

        crate::daemon::ability::builtins::resources::pages::publish::handle_publish(
            user,
            8787,
            "easynet.run",
            pages_registry("easynet.run", user),
            json!({
                "folder": folder.path().display().to_string(),
                "project_id": project_id,
                "visibility": "public",
            }),
        )
        .unwrap();
        clear_registry_for_user(user);

        let summary = restore_published_projects(user).unwrap();
        assert_eq!(summary.restored, 1);
        assert_eq!(summary.skipped, 0);
        assert!(PUBLISHED_PROJECTS.contains_key(&(user.to_string(), project_id.to_string())));

        let fetched = crate::daemon::ability::builtins::resources::pages::fetch::handle_fetch(
            user,
            project_id,
            json!({ "path": "/index.html" }),
        )
        .unwrap();
        assert_eq!(fetched["content_type"], "text/html; charset=utf-8");

        crate::daemon::ability::builtins::resources::pages::list_get_unpublish::handle_unpublish(
            user,
            json!({ "project_id": project_id }),
        )
        .unwrap();
        clear_registry_for_user(user);
    }

    #[test]
    fn restore_skips_missing_folder_and_cleans_snapshot() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();

        let user = "pages-missing-alice";
        clear_registry_for_user(user);
        let project_id = "missing-site";
        let folder = tempfile::tempdir().expect("tempdir");
        fs::write(folder.path().join("index.html"), "<h1>gone</h1>").unwrap();

        crate::daemon::ability::builtins::resources::pages::publish::handle_publish(
            user,
            8787,
            "easynet.run",
            pages_registry("easynet.run", user),
            json!({
                "folder": folder.path().display().to_string(),
                "project_id": project_id,
                "visibility": "public",
            }),
        )
        .unwrap();
        clear_registry_for_user(user);
        drop(folder);

        let summary = restore_published_projects(user).unwrap();
        assert_eq!(summary.restored, 0);
        assert_eq!(summary.skipped, 1);
        assert!(!PUBLISHED_PROJECTS.contains_key(&(user.to_string(), project_id.to_string())));
        assert!(
            !registry_path_for_user(user).exists(),
            "empty cleaned snapshot should be removed"
        );

        clear_registry_for_user(user);
    }
}

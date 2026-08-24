// EasyNet CLI — remote desktop session recovery store
// ===================================================
//
// File: plugins/remote-desktop/src/session_recovery.rs
// Description: Durable RemoteApp session recovery snapshot contract.
//
// Protocol Responsibility:
// - None. Axon owns Invocation, admission, and receipt semantics. This module
//   stores daemon-local RemoteApp session lifecycle projections so the plugin
//   can recover its own state after process restart.
//
// Implementation Approach:
// - Keep the durable format explicit, versioned, and fail-closed.
// - Persist one bounded JSON snapshot per session using atomic replace.
// - Store domain sub-projections as JSON values until full rehydration wires
//   typed constructors for target binding, consent, video, and input policy.
//
// Usage Contract:
// - Public session views must never read directly from this store. Handlers
//   rehydrate into the plugin-owned session model first.
// - Corrupt or mismatched snapshots are ignored/reported by recovery code; they
//   must not create partial live sessions.
//
// Architectural Position:
// - Remote-desktop plugin persistence layer. It is not an Axon receipt store,
//   frontend database, or generic daemon session registry.

#![allow(dead_code)]

use std::fs;
use std::io;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::daemon::persistence::config::{self, WritePermissions};
use crate::daemon::persistence::file_lock::ExclusiveFileLock;
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopRecoverySnapshot {
    schema_version: u32,
    session_id: String,
    session_token: String,
    creator_caller_ura: String,
    selected_resource_ura: String,
    subject_display_name: String,
    target_binding: Value,
    #[serde(default)]
    target_tracking: Option<Value>,
    consent: Value,
    mode: String,
    transport_preferences: Vec<String>,
    video: Value,
    input_policy: Value,
    created_at_ms: u64,
    updated_at_ms: u64,
    lease_expires_at_ms: u64,
    lifecycle_state: String,
    #[serde(default)]
    transport_epoch_high_watermark: u64,
    #[serde(default)]
    input_runtime_block_reason: Option<String>,
    terminal_receipt: Option<Value>,
    events: Vec<Value>,
}

impl RemoteDesktopRecoverySnapshot {
    pub(in crate::daemon::plugins::remote_desktop) fn from_session(
        session: &RemoteDesktopSession,
    ) -> anyhow::Result<Self> {
        let mut snapshot = Self::new(
            session.session_id().to_string(),
            session.session_token_for_recovery_snapshot().to_string(),
            session.creator_caller_ura().to_string(),
            session.subject_ura().to_string(),
            session.subject_display_name().to_string(),
            session.target_binding().to_value(),
            session.consent_state().to_value(),
            session.mode().to_string(),
            session.transport_preferences().to_vec(),
            session.video().to_value(),
            session.input_policy().to_value(),
            session.created_at_ms(),
            session.updated_at_ms(),
            session.lease_expires_at_ms(),
            session.state().json_name().to_string(),
            if session.is_terminal() {
                None
            } else {
                session
                    .input_runtime_block_reason()
                    .map(ToString::to_string)
            },
            session.terminal_receipt(),
            session.events(),
        )?;
        snapshot.target_tracking = Some(session.target_tracking_recovery_value());
        snapshot.transport_epoch_high_watermark = session.transport_epoch_high_watermark();
        snapshot.validate()?;
        Ok(snapshot)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        session_id: String,
        session_token: String,
        creator_caller_ura: String,
        selected_resource_ura: String,
        subject_display_name: String,
        target_binding: Value,
        consent: Value,
        mode: String,
        transport_preferences: Vec<String>,
        video: Value,
        input_policy: Value,
        created_at_ms: u64,
        updated_at_ms: u64,
        lease_expires_at_ms: u64,
        lifecycle_state: String,
        input_runtime_block_reason: Option<String>,
        terminal_receipt: Option<Value>,
        events: Vec<Value>,
    ) -> anyhow::Result<Self> {
        let snapshot = Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            session_id,
            session_token,
            creator_caller_ura,
            selected_resource_ura,
            subject_display_name,
            target_binding,
            target_tracking: None,
            consent,
            mode,
            transport_preferences,
            video,
            input_policy,
            created_at_ms,
            updated_at_ms,
            lease_expires_at_ms,
            lifecycle_state,
            transport_epoch_high_watermark: 0,
            input_runtime_block_reason,
            terminal_receipt,
            events,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn validate(&self) -> anyhow::Result<()> {
        if self.schema_version != SNAPSHOT_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported RemoteApp recovery snapshot schema {}; expected {}",
                self.schema_version,
                SNAPSHOT_SCHEMA_VERSION
            );
        }
        require_non_empty("session_id", &self.session_id)?;
        require_non_empty("session_token", &self.session_token)?;
        require_non_empty("creator_caller_ura", &self.creator_caller_ura)?;
        require_ura("selected_resource_ura", &self.selected_resource_ura)?;
        require_non_empty("subject_display_name", &self.subject_display_name)?;
        require_object("target_binding", &self.target_binding)?;
        if let Some(target_tracking) = &self.target_tracking {
            require_object("target_tracking", target_tracking)?;
        }
        require_object("consent", &self.consent)?;
        require_non_empty("mode", &self.mode)?;
        require_non_empty("lifecycle_state", &self.lifecycle_state)?;
        require_object("video", &self.video)?;
        require_object("input_policy", &self.input_policy)?;
        if self.updated_at_ms < self.created_at_ms {
            anyhow::bail!("updated_at_ms must be >= created_at_ms");
        }
        if self.lease_expires_at_ms < self.created_at_ms {
            anyhow::bail!("lease_expires_at_ms must be >= created_at_ms");
        }
        if let Some(terminal_receipt) = &self.terminal_receipt {
            require_object("terminal_receipt", terminal_receipt)?;
        }
        if let Some(reason) = &self.input_runtime_block_reason {
            require_non_empty("input_runtime_block_reason", reason)?;
        }
        Ok(())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(in crate::daemon::plugins::remote_desktop) fn session_token(&self) -> &str {
        &self.session_token
    }

    pub(in crate::daemon::plugins::remote_desktop) fn creator_caller_ura(&self) -> &str {
        &self.creator_caller_ura
    }

    pub(in crate::daemon::plugins::remote_desktop) fn selected_resource_ura(&self) -> &str {
        &self.selected_resource_ura
    }

    pub(in crate::daemon::plugins::remote_desktop) fn subject_display_name(&self) -> &str {
        &self.subject_display_name
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_binding(&self) -> &Value {
        &self.target_binding
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_tracking(&self) -> Option<&Value> {
        self.target_tracking.as_ref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn consent(&self) -> &Value {
        &self.consent
    }

    pub(in crate::daemon::plugins::remote_desktop) fn mode(&self) -> &str {
        &self.mode
    }

    pub(in crate::daemon::plugins::remote_desktop) fn transport_preferences(&self) -> &[String] {
        &self.transport_preferences
    }

    pub(in crate::daemon::plugins::remote_desktop) fn video(&self) -> &Value {
        &self.video
    }

    pub(in crate::daemon::plugins::remote_desktop) fn input_policy(&self) -> &Value {
        &self.input_policy
    }

    pub(in crate::daemon::plugins::remote_desktop) fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub(in crate::daemon::plugins::remote_desktop) fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }

    pub(in crate::daemon::plugins::remote_desktop) fn lease_expires_at_ms(&self) -> u64 {
        self.lease_expires_at_ms
    }

    pub(in crate::daemon::plugins::remote_desktop) fn lifecycle_state(&self) -> &str {
        &self.lifecycle_state
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn transport_epoch_high_watermark(
        &self,
    ) -> u64 {
        self.transport_epoch_high_watermark
    }

    pub(in crate::daemon::plugins::remote_desktop) fn input_runtime_block_reason(
        &self,
    ) -> Option<&str> {
        self.input_runtime_block_reason.as_deref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn terminal_receipt(&self) -> Option<Value> {
        self.terminal_receipt.clone()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn events(&self) -> Vec<Value> {
        self.events.clone()
    }

    fn terminal(&self) -> bool {
        self.terminal_receipt.is_some()
            || matches!(self.lifecycle_state.as_str(), "closed" | "failed")
    }

    fn last_event_sequence(&self) -> u64 {
        self.events
            .iter()
            .filter_map(|event| event.get("sequence").and_then(Value::as_u64))
            .max()
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopRecoveryStore {
    root: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopRecoveryLoadReport {
    snapshots: Vec<RemoteDesktopRecoverySnapshot>,
    rejected: Vec<RemoteDesktopRecoveryLoadRejection>,
}

impl RemoteDesktopRecoveryLoadReport {
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        snapshots: Vec<RemoteDesktopRecoverySnapshot>,
        rejected: Vec<RemoteDesktopRecoveryLoadRejection>,
    ) -> Self {
        Self {
            snapshots,
            rejected,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn into_snapshots(
        self,
    ) -> Vec<RemoteDesktopRecoverySnapshot> {
        self.snapshots
    }

    pub(in crate::daemon::plugins::remote_desktop) fn rejected(
        &self,
    ) -> &[RemoteDesktopRecoveryLoadRejection] {
        &self.rejected
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopRecoveryLoadRejection {
    path: PathBuf,
    reason: String,
}

impl RemoteDesktopRecoveryLoadRejection {
    fn new(path: PathBuf, reason: String) -> Self {
        Self { path, reason }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn path(&self) -> &PathBuf {
        &self.path
    }

    pub(in crate::daemon::plugins::remote_desktop) fn reason(&self) -> &str {
        &self.reason
    }
}

impl RemoteDesktopRecoveryStore {
    pub(in crate::daemon::plugins::remote_desktop) fn daemon_default() -> Self {
        Self { root: None }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn new(root: PathBuf) -> Self {
        Self { root: Some(root) }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn save(
        &self,
        snapshot: &RemoteDesktopRecoverySnapshot,
    ) -> anyhow::Result<PathBuf> {
        snapshot.validate()?;
        let root = self.root();
        fs::create_dir_all(&root)?;
        harden_recovery_dir(&root)?;
        let path = self.snapshot_path(snapshot.session_id())?;
        let _lock = ExclusiveFileLock::acquire_for_data_path(&path)?;
        match load_snapshot_path(&path) {
            Ok(existing) if !recovery_snapshot_should_replace(&existing, snapshot)? => {
                return Ok(path);
            }
            Ok(_) => {}
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) => {}
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "refusing to overwrite unreadable RemoteApp recovery snapshot {}: {error}",
                    path.display()
                ));
            }
        }
        let body = serde_json::to_vec_pretty(snapshot)?;
        config::atomic_write_with_permissions(&path, &body, WritePermissions::OwnerReadWrite)?;
        Ok(path)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn load(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Option<RemoteDesktopRecoverySnapshot>> {
        let path = self.snapshot_path(session_id)?;
        match fs::read(&path) {
            Ok(body) => {
                let snapshot: RemoteDesktopRecoverySnapshot = serde_json::from_slice(&body)?;
                snapshot.validate()?;
                Ok(Some(snapshot))
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn load_all(
        &self,
    ) -> anyhow::Result<RemoteDesktopRecoveryLoadReport> {
        let root = self.root();
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(RemoteDesktopRecoveryLoadReport::new(Vec::new(), Vec::new()));
            }
            Err(err) => return Err(err.into()),
        };
        let mut snapshots = Vec::new();
        let mut rejected = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            match load_snapshot_path(&path) {
                Ok(snapshot) => snapshots.push(snapshot),
                Err(err) => rejected.push(RemoteDesktopRecoveryLoadRejection::new(
                    path,
                    err.to_string(),
                )),
            }
        }
        snapshots.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        rejected.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(RemoteDesktopRecoveryLoadReport::new(snapshots, rejected))
    }

    fn snapshot_path(&self, session_id: &str) -> anyhow::Result<PathBuf> {
        validate_session_id_for_path(session_id)?;
        Ok(self.root().join(format!("{session_id}.json")))
    }

    fn root(&self) -> PathBuf {
        self.root
            .clone()
            .unwrap_or_else(|| config::state_dir().join("remote-desktop").join("sessions"))
    }
}

/// Decide a per-session durable commit while the snapshot lock is held.
///
/// Terminal state is absorbing: once published, no delayed active snapshot
/// may revive the session. Within the same terminal class, event sequence is
/// the aggregate revision and `updated_at_ms` breaks ties for lease-only
/// mutations. Equal revisions with different bodies fail closed because they
/// reveal a mutation path that did not advance either ordering signal.
fn recovery_snapshot_should_replace(
    existing: &RemoteDesktopRecoverySnapshot,
    incoming: &RemoteDesktopRecoverySnapshot,
) -> anyhow::Result<bool> {
    if existing.session_id != incoming.session_id {
        anyhow::bail!("RemoteApp recovery snapshot commit compared different session ids");
    }
    if existing.session_token != incoming.session_token {
        if incoming.created_at_ms < existing.created_at_ms {
            return Ok(false);
        }
        if incoming.created_at_ms > existing.created_at_ms {
            return Ok(true);
        }
        anyhow::bail!(
            "conflicting RemoteApp session incarnations share session_id={} created_at_ms={}",
            incoming.session_id,
            incoming.created_at_ms
        );
    }
    match (existing.terminal(), incoming.terminal()) {
        (true, false) => return Ok(false),
        (false, true) => return Ok(true),
        _ => {}
    }
    let existing_order = (existing.last_event_sequence(), existing.updated_at_ms);
    let incoming_order = (incoming.last_event_sequence(), incoming.updated_at_ms);
    if incoming_order < existing_order {
        return Ok(false);
    }
    if incoming_order > existing_order {
        return Ok(true);
    }
    if existing == incoming {
        return Ok(false);
    }
    anyhow::bail!(
        "conflicting RemoteApp recovery snapshots share session revision sequence={} updated_at_ms={}",
        incoming_order.0,
        incoming_order.1
    )
}

fn load_snapshot_path(path: &PathBuf) -> anyhow::Result<RemoteDesktopRecoverySnapshot> {
    let body = fs::read(path)?;
    let snapshot: RemoteDesktopRecoverySnapshot = serde_json::from_slice(&body)?;
    snapshot.validate()?;
    Ok(snapshot)
}

#[cfg(unix)]
fn harden_recovery_dir(path: &PathBuf) -> anyhow::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn harden_recovery_dir(_path: &PathBuf) -> anyhow::Result<()> {
    Ok(())
}

fn validate_session_id_for_path(session_id: &str) -> anyhow::Result<()> {
    require_non_empty("session_id", session_id)?;
    if session_id
        .chars()
        .any(|ch| ch == '/' || ch == '\\' || ch.is_control() || ch.is_whitespace())
    {
        anyhow::bail!("session_id is not safe for a recovery snapshot path");
    }
    Ok(())
}

fn require_non_empty(field: &'static str, value: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{field} must be non-empty");
    }
    Ok(())
}

fn require_ura(field: &'static str, value: &str) -> anyhow::Result<()> {
    require_non_empty(field, value)?;
    if !value.starts_with("easynet:///r/") {
        anyhow::bail!("{field} must be a canonical EasyNet URA");
    }
    Ok(())
}

fn require_object(field: &'static str, value: &Value) -> anyhow::Result<()> {
    if !value.is_object() {
        anyhow::bail!("{field} must be a JSON object");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use serde_json::json;

    use super::{RemoteDesktopRecoverySnapshot, RemoteDesktopRecoveryStore};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn snapshot() -> RemoteDesktopRecoverySnapshot {
        RemoteDesktopRecoverySnapshot::new(
            "rd-recovery-test".to_string(),
            "test-session-token".to_string(),
            "easynet:///r/localhost/user/u1".to_string(),
            "easynet:///r/localhost/resource/device.dev/streams/window.1".to_string(),
            "Recovered window".to_string(),
            json!({"binding_id": "tb_1", "target_kind": "window"}),
            json!({
                "policy": "local_user_consent",
                "approval_receipt": {
                    "receipt_ura": "easynet:///r/localhost/resource/device.dev/invocation/i/history/receipt/4",
                    "receipt_hash": "0".repeat(64)
                }
            }),
            "view_only".to_string(),
            vec!["webrtc".to_string()],
            json!({"max_fps": 30}),
            json!({"keyboard_enabled": false, "pointer_enabled": false}),
            100,
            120,
            60_000,
            "active".to_string(),
            None,
            None,
            vec![json!({"event_type": "SESSION_CREATED", "sequence": 1})],
        )
        .expect("valid snapshot")
    }

    fn terminal_snapshot() -> RemoteDesktopRecoverySnapshot {
        let mut snapshot = snapshot();
        snapshot.updated_at_ms = 140;
        snapshot.lifecycle_state = "closed".to_string();
        snapshot.terminal_receipt = Some(json!({
            "receipt_type": "remoteapp.session.terminal.v1",
            "session_id": snapshot.session_id,
            "terminal": true,
        }));
        snapshot.events.push(json!({
            "event_type": "SESSION_CLOSED",
            "sequence": 2,
            "terminal": true,
        }));
        snapshot
    }

    #[test]
    fn recovery_store_round_trips_valid_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RemoteDesktopRecoveryStore::new(temp.path().to_path_buf());
        let snapshot = snapshot();

        let path = store.save(&snapshot).expect("save snapshot");
        assert!(path.ends_with("rd-recovery-test.json"));

        let loaded = store
            .load(snapshot.session_id())
            .expect("load snapshot")
            .expect("snapshot exists");
        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn recovery_store_terminal_snapshot_is_absorbing_against_delayed_active_write() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RemoteDesktopRecoveryStore::new(temp.path().to_path_buf());
        let active = snapshot();
        let terminal = terminal_snapshot();

        store.save(&terminal).expect("terminal snapshot saves");
        store
            .save(&active)
            .expect("delayed active snapshot is safely ignored");

        let loaded = store
            .load(active.session_id())
            .expect("load terminal snapshot")
            .expect("terminal snapshot exists");
        assert_eq!(loaded, terminal);
    }

    #[test]
    fn recovery_store_concurrent_terminal_and_active_commits_converge_to_terminal() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let barrier = Arc::new(Barrier::new(3));
        let active = snapshot();
        let terminal = terminal_snapshot();

        let active_writer = {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            let active = active.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.save(&active)
            })
        };
        let terminal_writer = {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            let terminal = terminal.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.save(&terminal)
            })
        };
        barrier.wait();
        active_writer
            .join()
            .expect("active writer joins")
            .expect("active commit completes");
        terminal_writer
            .join()
            .expect("terminal writer joins")
            .expect("terminal commit completes");

        let loaded = store
            .load(active.session_id())
            .expect("load converged snapshot")
            .expect("snapshot exists");
        assert_eq!(loaded, terminal);
    }

    #[test]
    fn recovery_store_allows_newer_session_incarnation_after_terminal_row() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RemoteDesktopRecoveryStore::new(temp.path().to_path_buf());
        let terminal = terminal_snapshot();
        let mut next = snapshot();
        next.session_token = "next-session-token".to_string();
        next.created_at_ms = 1_000;
        next.updated_at_ms = 1_020;
        next.lease_expires_at_ms = 61_000;

        store.save(&terminal).expect("old terminal snapshot saves");
        store
            .save(&next)
            .expect("new incarnation replaces old terminal");
        store
            .save(&terminal)
            .expect("delayed old-incarnation terminal is ignored");

        let loaded = store
            .load(next.session_id())
            .expect("load new incarnation")
            .expect("new incarnation exists");
        assert_eq!(loaded, next);
    }

    #[test]
    fn recovery_snapshot_round_trips_runtime_input_block_reason() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RemoteDesktopRecoveryStore::new(temp.path().to_path_buf());
        let mut snapshot = snapshot();
        snapshot.input_runtime_block_reason = Some("accessibility_permission_denied".to_string());

        store.save(&snapshot).expect("save snapshot");

        let loaded = store
            .load(snapshot.session_id())
            .expect("load snapshot")
            .expect("snapshot exists");
        assert_eq!(
            loaded.input_runtime_block_reason(),
            Some("accessibility_permission_denied")
        );
        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn recovery_snapshot_keeps_legacy_rows_without_runtime_input_block_reason_loadable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("rd-recovery-test.json");
        let mut body = serde_json::to_value(snapshot()).expect("snapshot serializes");
        body.as_object_mut()
            .expect("snapshot body object")
            .remove("input_runtime_block_reason");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&body).expect("serialize legacy snapshot"),
        )
        .expect("write legacy snapshot");
        let store = RemoteDesktopRecoveryStore::new(temp.path().to_path_buf());

        let loaded = store
            .load("rd-recovery-test")
            .expect("legacy snapshot loads")
            .expect("snapshot exists");

        assert_eq!(loaded.input_runtime_block_reason(), None);
    }

    #[test]
    fn recovery_store_fails_closed_for_corrupt_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("rd-corrupt.json");
        let mut corrupt = snapshot();
        corrupt.schema_version = 999;
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&corrupt).expect("serialize corrupt snapshot"),
        )
        .expect("write corrupt snapshot");
        let store = RemoteDesktopRecoveryStore::new(temp.path().to_path_buf());

        let err = store
            .load("rd-corrupt")
            .expect_err("corrupt snapshot must not load");
        assert!(
            err.to_string()
                .contains("unsupported RemoteApp recovery snapshot schema"),
            "{err}"
        );
    }

    #[test]
    fn recovery_store_refuses_to_overwrite_corrupt_existing_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("rd-recovery-test.json");
        let corrupt = b"{not json";
        std::fs::write(&path, corrupt).expect("write corrupt snapshot");
        let store = RemoteDesktopRecoveryStore::new(temp.path().to_path_buf());

        let error = store
            .save(&snapshot())
            .expect_err("corrupt existing state must fail closed");

        assert!(
            error
                .to_string()
                .contains("refusing to overwrite unreadable RemoteApp recovery snapshot"),
            "{error}"
        );
        assert_eq!(
            std::fs::read(path).expect("read unchanged corrupt row"),
            corrupt,
            "a failed validation must not replace the existing durable row"
        );
    }

    #[test]
    fn recovery_store_load_all_reports_corrupt_snapshots_without_dropping_valid_rows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RemoteDesktopRecoveryStore::new(temp.path().to_path_buf());
        let snapshot = snapshot();
        store.save(&snapshot).expect("save valid snapshot");
        std::fs::write(temp.path().join("rd-corrupt.json"), b"{not json")
            .expect("write corrupt snapshot");

        let report = store.load_all().expect("load batch recovery report");
        assert_eq!(report.rejected().len(), 1);
        assert!(report.rejected()[0].path().ends_with("rd-corrupt.json"));
        assert!(
            !report.rejected()[0].reason().trim().is_empty(),
            "rejection reason must be observable"
        );
        let snapshots = report.into_snapshots();
        assert_eq!(snapshots, vec![snapshot]);
    }

    #[cfg(unix)]
    #[test]
    fn recovery_store_saves_private_snapshot_permissions() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RemoteDesktopRecoveryStore::new(temp.path().to_path_buf());
        let snapshot = snapshot();

        let path = store.save(&snapshot).expect("save snapshot");
        let dir_mode = std::fs::metadata(temp.path())
            .expect("recovery dir metadata")
            .permissions()
            .mode()
            & 0o777;
        let file_mode = std::fs::metadata(path)
            .expect("snapshot metadata")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }

    #[test]
    fn recovery_store_rejects_path_unsafe_session_ids() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = RemoteDesktopRecoveryStore::new(temp.path().to_path_buf());

        let err = store
            .load("../rd-escape")
            .expect_err("path-unsafe session id must fail");
        assert!(
            err.to_string()
                .contains("session_id is not safe for a recovery snapshot path"),
            "{err}"
        );
    }

    #[test]
    fn recovery_snapshot_requires_a_resource_subject() {
        let err = RemoteDesktopRecoverySnapshot::new(
            "rd-recovery-test".to_string(),
            "test-session-token".to_string(),
            "easynet:///r/localhost/user/u1".to_string(),
            "not-a-ura".to_string(),
            "Recovered window".to_string(),
            json!({}),
            json!({}),
            "view_only".to_string(),
            vec!["webrtc".to_string()],
            json!({}),
            json!({}),
            100,
            100,
            101,
            "active".to_string(),
            None,
            None,
            vec![],
        )
        .expect_err("invalid subject must fail");
        assert!(
            err.to_string()
                .contains("selected_resource_ura must be a canonical EasyNet URA"),
            "{err}"
        );
    }
}

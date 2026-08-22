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

use serde::{Deserialize, Serialize};
use serde_json::Value;

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopRecoverySnapshot {
    schema_version: u32,
    session_id: String,
    creator_caller_ura: String,
    selected_resource_ura: String,
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
    terminal_receipt: Option<Value>,
    events: Vec<Value>,
}

impl RemoteDesktopRecoverySnapshot {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        session_id: String,
        creator_caller_ura: String,
        selected_resource_ura: String,
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
        terminal_receipt: Option<Value>,
        events: Vec<Value>,
    ) -> anyhow::Result<Self> {
        let snapshot = Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            session_id,
            creator_caller_ura,
            selected_resource_ura,
            target_binding,
            consent,
            mode,
            transport_preferences,
            video,
            input_policy,
            created_at_ms,
            updated_at_ms,
            lease_expires_at_ms,
            lifecycle_state,
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
        require_non_empty("creator_caller_ura", &self.creator_caller_ura)?;
        require_ura("selected_resource_ura", &self.selected_resource_ura)?;
        require_object("target_binding", &self.target_binding)?;
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
        Ok(())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn session_id(&self) -> &str {
        &self.session_id
    }
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopRecoveryStore {
    root: PathBuf,
}

impl RemoteDesktopRecoveryStore {
    pub(in crate::daemon::plugins::remote_desktop) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn save(
        &self,
        snapshot: &RemoteDesktopRecoverySnapshot,
    ) -> anyhow::Result<PathBuf> {
        snapshot.validate()?;
        fs::create_dir_all(&self.root)?;
        let path = self.snapshot_path(snapshot.session_id())?;
        let tmp_path = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(snapshot)?;
        fs::write(&tmp_path, body)?;
        fs::rename(&tmp_path, &path)?;
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

    fn snapshot_path(&self, session_id: &str) -> anyhow::Result<PathBuf> {
        validate_session_id_for_path(session_id)?;
        Ok(self.root.join(format!("{session_id}.json")))
    }
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
    use serde_json::json;

    use super::{RemoteDesktopRecoverySnapshot, RemoteDesktopRecoveryStore};

    fn snapshot() -> RemoteDesktopRecoverySnapshot {
        RemoteDesktopRecoverySnapshot::new(
            "rd-recovery-test".to_string(),
            "easynet:///r/localhost/user/u1".to_string(),
            "easynet:///r/localhost/resource/device.dev/streams/window.1".to_string(),
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
            vec![json!({"event_type": "SESSION_CREATED", "sequence": 1})],
        )
        .expect("valid snapshot")
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
            "easynet:///r/localhost/user/u1".to_string(),
            "not-a-ura".to_string(),
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

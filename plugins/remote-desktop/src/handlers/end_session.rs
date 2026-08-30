// EasyNet CLI — remote desktop end-session handler
// ================================================

use std::sync::Arc;

use serde_json::{json, Value};

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_END_SESSION, REASON_INVALID_ARGUMENT,
};
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::request::require_str;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session_access::ensure_session_control_identity;
use crate::daemon::plugins::remote_desktop::session_lifecycle::{
    begin_session_transport_settlement, commit_prepared_closing_checkpoint,
    settle_session_transports_and_finish, RetiredSessionTransports,
};
use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoverySnapshot;
use crate::daemon::plugins::remote_desktop::transport::TransportSettlementStatus;

/// Handle `remote_desktop.end_session`.
pub(in crate::daemon::plugins::remote_desktop) fn handle(
    plugin: Arc<RemoteDesktopPlugin>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let session_id = require_str(&args, "session_id", ABILITY_END_SESSION)?.to_string();
    let reason = match args.get("reason") {
        None => "caller_ended".to_string(),
        Some(Value::String(reason)) if !reason.trim().is_empty() => reason.trim().to_string(),
        Some(_) => anyhow::bail!(
            "{ABILITY_END_SESSION}: `reason` must be a non-empty string; reason={REASON_INVALID_ARGUMENT}"
        ),
    };
    let sessions = plugin.session_store();
    let operation_lock = sessions.target_operation_lock(&session_id);
    let operation = match operation_lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let (closing_intent, existing) = sessions.with_sessions(|sessions| -> anyhow::Result<_> {
        let session =
            sessions
                .get_mut(&session_id)
                .ok_or_else(|| RemoteDesktopError::SessionNotFound {
                    ability: ABILITY_END_SESSION,
                    session_id: session_id.clone(),
                })?;
        ensure_session_control_identity(ABILITY_END_SESSION, &env, &args, session)?;
        if session.is_terminal() || session.is_terminating() {
            let mut view = plugin.session_view(session);
            if let Some(map) = view.as_object_mut() {
                map.insert("already_ended".into(), json!(session.is_terminal()));
                map.insert("already_ending".into(), json!(session.is_terminating()));
            }
            let recovery_snapshot = RemoteDesktopRecoverySnapshot::from_session(session)?;
            return Ok((None, Some((recovery_snapshot, view))));
        }
        let closing_intent =
            RemoteDesktopRecoverySnapshot::prepare_closing_intent(session, &reason)?;
        Ok((Some(closing_intent), None))
    })?;
    if let Some((recovery_snapshot, view)) = existing {
        drop(operation);
        plugin.persist_recovery_snapshot(&recovery_snapshot)?;
        return Ok(view);
    }
    let closing_intent = closing_intent.expect("non-terminal session prepares Closing intent");
    // This is the write-ahead linearization point. Failure leaves the live
    // aggregate and every host transport untouched, so the caller may retry.
    let checkpoint =
        commit_prepared_closing_checkpoint(plugin.recovery_store().as_ref(), &closing_intent)?;
    let (transports, closing_snapshot) =
        plugin
            .session_store()
            .with_sessions(|sessions| -> anyhow::Result<_> {
                let session = sessions.get_mut(&session_id).ok_or_else(|| {
                    RemoteDesktopError::SessionNotFound {
                        ability: ABILITY_END_SESSION,
                        session_id: session_id.clone(),
                    }
                })?;
                assert!(
                    session.begin_close(&reason),
                    "non-terminal RemoteApp session must enter Closing"
                );
                let closing_snapshot = RemoteDesktopRecoverySnapshot::from_session(session)?;
                let transports =
                    begin_session_transport_settlement(&plugin, &session_id, session, checkpoint)
                        .unwrap_or_else(RetiredSessionTransports::empty);
                Ok((transports, closing_snapshot))
            })?;
    // The write-ahead intent is sufficient for crash safety; this richer row
    // advances event/epoch projections used by recovery and observability.
    if let Err(error) = plugin.persist_recovery_snapshot(&closing_snapshot) {
        eprintln!(
            "[remote-desktop] durable Closing intent exists for {session_id}, but the richer Closing projection remains pending: {error}"
        );
    }
    drop(operation);
    if settle_session_transports_and_finish(
        plugin.transport_manager().settlement_queue(),
        plugin.session_store(),
        plugin.recovery_store(),
        plugin.relay_lease_provider(),
        session_id.clone(),
        transports,
    ) != TransportSettlementStatus::Settled
    {
        anyhow::bail!(
            "{ABILITY_END_SESSION}: bounded transport settlement or durable terminal commit was not confirmed; session remains Closing"
        );
    }
    plugin.session_store().with_sessions(|sessions| {
        let session =
            sessions
                .get(&session_id)
                .ok_or_else(|| RemoteDesktopError::SessionNotFound {
                    ability: ABILITY_END_SESSION,
                    session_id: session_id.clone(),
                })?;
        Ok(plugin.session_view(session))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::sync::watch;

    use crate::daemon::ability::builtins::resources::media::screen_snapshot::SyntheticScreenBackend;
    use crate::daemon::persistence::resources::{self, ResourcesFile};
    use crate::daemon::plugins::remote_desktop::relay_lease::{
        RemoteDesktopRelayLease, RemoteDesktopRelayLeaseAvailability, RemoteDesktopRelayLeaseInit,
        RemoteDesktopRelayLeaseProvider, EASYNET_RELAY_PROVIDER,
    };
    use crate::daemon::plugins::remote_desktop::session_recovery::RemoteDesktopRecoveryStore;
    use crate::daemon::plugins::remote_desktop::test_support::{
        create_test_session, env_for, reset_store, seed_display, test_lock, test_plugin,
        test_runtime_limits, TestRemoteAppTargetBindingVerifier,
    };
    use crate::daemon::plugins::remote_desktop::transport::PreviewTaskGroupCompletion;

    #[derive(Default)]
    struct RecordingRelayLeaseProvider {
        acquires: AtomicUsize,
        releases: AtomicUsize,
        released_lease_ids: Mutex<Vec<String>>,
    }

    impl RemoteDesktopRelayLeaseProvider for RecordingRelayLeaseProvider {
        fn acquire(
            &self,
            session_id: &str,
            resource_ura: &str,
        ) -> anyhow::Result<RemoteDesktopRelayLeaseAvailability> {
            let now = crate::daemon::plugins::remote_desktop::session::now_ms();
            let acquire_sequence = self.acquires.fetch_add(1, Ordering::SeqCst) + 1;
            let refresh_after_ms = if acquire_sequence == 1 {
                now + 25
            } else {
                now + 60_000
            };
            Ok(RemoteDesktopRelayLeaseAvailability::Active(
                RemoteDesktopRelayLease::from_init(
                    session_id,
                    resource_ura,
                    RemoteDesktopRelayLeaseInit {
                        provider: EASYNET_RELAY_PROVIDER.to_string(),
                        lease_id: format!("lease-product-lifecycle-{acquire_sequence}"),
                        session_id: session_id.to_string(),
                        device_ura: "easynet:///r/acme/device/01DEV".to_string(),
                        resource_ura: resource_ura.to_string(),
                        urls: vec!["turn:relay.example.test:3478?transport=udp".to_string()],
                        username: "ephemeral-user".to_string(),
                        credential: "ephemeral-secret".to_string(),
                        issued_at_ms: now,
                        refresh_after_ms,
                        expires_at_ms: now + 120_000,
                    },
                )?,
            ))
        }

        fn release(&self, lease: &RemoteDesktopRelayLease) -> anyhow::Result<()> {
            self.releases.fetch_add(1, Ordering::SeqCst);
            self.released_lease_ids
                .lock()
                .expect("released lease ids lock")
                .push(lease.lease_id().to_string());
            Ok(())
        }
    }

    #[test]
    fn hub_relay_lease_reaches_both_ice_views_and_releases_after_terminal_commit() {
        let _lock = test_lock();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let relay = Arc::new(RecordingRelayLeaseProvider::default());
        let plugin = RemoteDesktopPlugin::with_relay_lease_provider_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            relay.clone(),
        );
        reset_store(&plugin);
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-relay-lifecycle");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let created = create_test_session(
            Arc::clone(&plugin),
            env.clone(),
            json!({"session_id": "rd-relay-lifecycle", "mode": "view_only"}),
        )
        .expect("session with Hub relay lease creates");
        assert_eq!(
            created["transport"]["client_ice_servers"][0]["credential"],
            json!("ephemeral-secret")
        );
        assert_eq!(
            created["transport"]["easynet_relay"]["lease_id"],
            json!("lease-product-lifecycle-1")
        );
        let refresh_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let refreshed = plugin.session_store().with_sessions(|sessions| {
                sessions
                    .get("rd-relay-lifecycle")
                    .and_then(|session| session.active_relay_lease())
                    .is_some_and(|lease| lease.lease_id() == "lease-product-lifecycle-2")
            });
            if refreshed {
                break;
            }
            assert!(
                std::time::Instant::now() < refresh_deadline,
                "relay lease monitor did not install the refreshed lease"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(relay.acquires.load(Ordering::SeqCst), 2);
        assert_eq!(relay.releases.load(Ordering::SeqCst), 0);
        let token = created["session_token"].as_str().unwrap();
        let ended = handle(
            Arc::clone(&plugin),
            env,
            json!({
                "session_id": "rd-relay-lifecycle",
                "session_token": token,
                "reason": "test_end",
            }),
        )
        .expect("terminal settlement succeeds");
        assert_eq!(ended["state"], json!("closed"));
        assert_eq!(relay.releases.load(Ordering::SeqCst), 1);
        assert_eq!(
            relay
                .released_lease_ids
                .lock()
                .expect("released lease ids lock")
                .as_slice(),
            ["lease-product-lifecycle-2"]
        );
        assert_eq!(
            ended["transport"]["easynet_relay"]["state"],
            json!("unavailable")
        );
        let persisted = serde_json::to_string(
            &plugin
                .recovery_store()
                .load("rd-relay-lifecycle")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(!persisted.contains("ephemeral-secret"));
        assert!(!persisted.contains("ephemeral-user"));
    }

    #[test]
    fn end_session_persists_terminal_recovery_snapshot() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-recovery-end-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let created = create_test_session(
            Arc::clone(&plugin),
            env.clone(),
            json!({"session_id": "rd-recovery-end", "mode": "view_only"}),
        )
        .expect("test session creates");
        let token = created["session_token"]
            .as_str()
            .expect("create_session returns session token")
            .to_string();

        let invalid_reason = handle(
            Arc::clone(&plugin),
            env.clone(),
            json!({
                "session_id": "rd-recovery-end",
                "session_token": token.clone(),
                "reason": "   ",
            }),
        )
        .expect_err("blank terminal reason must fail before mutating lifecycle");
        assert!(invalid_reason.to_string().contains(REASON_INVALID_ARGUMENT));

        let ended = handle(
            Arc::clone(&plugin),
            env,
            json!({
                "session_id": "rd-recovery-end",
                "session_token": token,
                "reason": "test_end",
            }),
        )
        .expect("end_session succeeds");
        assert_eq!(ended["state"], json!("closed"));
        assert_eq!(ended["end_reason"], json!("test_end"));

        let snapshot = plugin
            .recovery_store()
            .load("rd-recovery-end")
            .expect("recovery snapshot load succeeds")
            .expect("end_session must write a terminal recovery snapshot");
        let snapshot = serde_json::to_value(snapshot).expect("snapshot serializes");
        assert_eq!(snapshot["session_id"], json!("rd-recovery-end"));
        assert_eq!(snapshot["lifecycle_state"], json!("closed"));
        assert!(snapshot["terminal_receipt"].is_object());
        assert!(snapshot["events"].as_array().unwrap().iter().any(|event| {
            event["event_type"] == json!("SESSION_CLOSED")
                && event["payload"]["reason"] == json!("test_end")
        }));
    }

    #[test]
    fn end_session_commits_closed_only_after_preview_task_group_completes() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-preview-settlement-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let created = create_test_session(
            Arc::clone(&plugin),
            env.clone(),
            json!({"session_id": "rd-preview-settlement", "mode": "view_only"}),
        )
        .expect("test session creates");
        let token = created["session_token"]
            .as_str()
            .expect("create_session returns session token")
            .to_string();

        let (stop_tx, mut stop_rx) = watch::channel(false);
        let preview_epoch = plugin.session_store().with_sessions(|sessions| {
            sessions
                .get_mut("rd-preview-settlement")
                .expect("test session exists")
                .attach_preview_transport(stop_tx.clone())
                .expect("live session accepts preview")
                .0
        });
        let (worker_done_tx, worker_done_rx) = mpsc::channel();
        plugin.transport_manager().activate_preview(
            "rd-preview-settlement".to_string(),
            preview_epoch,
            stop_tx,
            worker_done_rx,
        );

        let worker_plugin = Arc::clone(&plugin);
        let end = std::thread::spawn(move || {
            handle(
                worker_plugin,
                env,
                json!({
                    "session_id": "rd-preview-settlement",
                    "session_token": token,
                    "reason": "test_preview_settlement",
                }),
            )
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        while !*stop_rx.borrow_and_update() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(
            *stop_rx.borrow_and_update(),
            "end_session publishes preview stop"
        );
        plugin.session_store().with_sessions(|sessions| {
            let session = sessions
                .get("rd-preview-settlement")
                .expect("session remains observable while settling");
            assert!(session.is_terminating());
            assert!(!session.is_terminal());
        });

        worker_done_tx
            .send(PreviewTaskGroupCompletion)
            .expect("preview task group reports completion");
        let ended = end
            .join()
            .expect("end_session worker exits")
            .expect("end_session succeeds after settlement");
        assert_eq!(ended["state"], json!("closed"));
        assert_eq!(ended["end_reason"], json!("test_preview_settlement"));
    }

    #[test]
    fn end_session_retains_closing_when_preview_completion_receipt_is_lost() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-preview-disconnect-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let created = create_test_session(
            Arc::clone(&plugin),
            env.clone(),
            json!({"session_id": "rd-preview-disconnect", "mode": "view_only"}),
        )
        .expect("test session creates");
        let token = created["session_token"]
            .as_str()
            .expect("create_session returns session token")
            .to_string();

        let (stop_tx, _stop_rx) = watch::channel(false);
        let preview_epoch = plugin.session_store().with_sessions(|sessions| {
            sessions
                .get_mut("rd-preview-disconnect")
                .expect("test session exists")
                .attach_preview_transport(stop_tx.clone())
                .expect("live session accepts preview")
                .0
        });
        let (worker_done_tx, worker_done_rx) = mpsc::channel();
        plugin.transport_manager().activate_preview(
            "rd-preview-disconnect".to_string(),
            preview_epoch,
            stop_tx,
            worker_done_rx,
        );
        drop(worker_done_tx);

        let error = handle(
            Arc::clone(&plugin),
            env,
            json!({
                "session_id": "rd-preview-disconnect",
                "session_token": token,
                "reason": "test_preview_disconnect",
            }),
        )
        .expect_err("missing completion receipt must fail closed");
        assert!(error.to_string().contains("session remains Closing"));
        plugin.session_store().with_sessions(|sessions| {
            let session = sessions
                .get("rd-preview-disconnect")
                .expect("Closing session remains observable");
            assert!(session.is_terminating());
            assert!(!session.is_terminal());
        });
        let snapshot = plugin
            .recovery_store()
            .load("rd-preview-disconnect")
            .expect("Closing snapshot loads")
            .expect("Closing intent is durable");
        assert_eq!(snapshot.lifecycle_state(), "closing");
    }

    #[test]
    fn end_session_checkpoint_failure_does_not_start_host_teardown() {
        let _lock = test_lock();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("temporary recovery root");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let plugin = RemoteDesktopPlugin::with_recovery_store_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            Arc::clone(&recovery),
        );
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-checkpoint-failure-display");
        resources::save(&file).expect("test resource saves");
        let env = env_for(&ura);
        let created = create_test_session(
            Arc::clone(&plugin),
            env.clone(),
            json!({"session_id": "rd-checkpoint-failure", "mode": "view_only"}),
        )
        .expect("test session creates");
        let token = created["session_token"]
            .as_str()
            .expect("session token")
            .to_string();
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let preview_epoch = plugin.session_store().with_sessions(|rows| {
            rows.get_mut("rd-checkpoint-failure")
                .expect("session exists")
                .install_preview_transport_for_test(stop_tx.clone())
        });
        let (done_tx, done_rx) = mpsc::channel();
        plugin.transport_manager().activate_preview(
            "rd-checkpoint-failure".to_string(),
            preview_epoch,
            stop_tx,
            done_rx,
        );
        recovery.set_fail_saves_for_test(true);

        let error = handle(
            Arc::clone(&plugin),
            env,
            json!({
                "session_id": "rd-checkpoint-failure",
                "session_token": token,
                "reason": "caller_ended",
            }),
        )
        .expect_err("Closing checkpoint failure must reject termination");

        assert!(error
            .to_string()
            .contains("injected RemoteApp recovery save failure"));
        assert!(
            !*stop_rx.borrow_and_update(),
            "preview stop must not be sent"
        );
        plugin.session_store().with_sessions(|rows| {
            let session = rows.get("rd-checkpoint-failure").expect("session remains");
            assert!(!session.is_terminating());
            assert!(!session.is_terminal());
            assert!(session.preview_attached());
        });
        recovery.set_fail_saves_for_test(false);
        done_tx
            .send(PreviewTaskGroupCompletion)
            .expect("preview worker completion sends");
        reset_store(&plugin);
    }
}

use std::time::Duration;

/// Best-effort REST backstop before each `session.open` dial.
///
/// Hub restarts can lose the in-memory trust view before the device's
/// next gRPC reconnect. The backend already exposes
/// `/api/v1/devices/verify-credential` as the idempotent path that
/// replays `identity.register_pubkey` into the hub daemon. Calling
/// it here keeps the reconnect loop self-healing: if the Hub forgot this
/// device, the trust entry is restored before `federation.join` and
/// `federation.advertise_abilities` run. Failures are advisory; the
/// subsequent gRPC prelude remains the authoritative session gate.
pub(super) async fn warm_device_credential_for_session(caller_ura: &str) {
    let caller_ura = caller_ura.to_string();
    let outcome = tokio::task::spawn_blocking(move || verify_device_credential_once(&caller_ura))
        .await
        .unwrap_or_else(|err| CredentialWarmupOutcome::Failed {
            api_base: String::new(),
            reason: format!("credential warmup task join failed: {err}"),
        });

    match outcome {
        CredentialWarmupOutcome::Verified { api_base } => {
            crate::op_event!(
                component = session,
                kind = credential_verify_warmup_ok,
                api_base = api_base,
                message = "device credential verified before session.open dial",
            );
        }
        CredentialWarmupOutcome::Skipped { reason } => {
            crate::op_event!(
                component = session,
                kind = credential_verify_warmup_skipped,
                reason = reason,
            );
        }
        CredentialWarmupOutcome::Failed { api_base, reason } => {
            crate::op_event!(
                component = session,
                kind = credential_verify_warmup_failed,
                api_base = api_base,
                reason = reason,
                message =
                    "continuing to gRPC session prelude; Hub will return the authoritative status",
            );
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CredentialWarmupOutcome {
    Verified { api_base: String },
    Skipped { reason: String },
    Failed { api_base: String, reason: String },
}

fn verify_device_credential_once(caller_ura: &str) -> CredentialWarmupOutcome {
    // Test isolation: the warmup runs unconditionally at the top of
    // dial_and_run_session_with_idle_timeout, so every dial test would
    // otherwise read the developer's real ~/.easynet/credentials.json and
    // fire a blocking 5s ureq POST that races other tests' loopback hubs
    // (flaky Elapsed / corrupt-header failures under the parallel suite).
    // Skip the live read+POST in test builds; the HTTP wire shape itself
    // is covered by credential_warmup_posts_current_device_credential,
    // which drives verify_device_credential_for_credentials directly.
    if cfg!(test) {
        return CredentialWarmupOutcome::Skipped {
            reason: "credential warmup skipped under cargo test".to_string(),
        };
    }
    let creds = match crate::daemon::persistence::config::load_credentials() {
        Ok(creds) => creds,
        Err(err) => {
            return CredentialWarmupOutcome::Skipped {
                reason: format!("credentials unavailable: {err}"),
            };
        }
    };
    verify_device_credential_for_credentials(caller_ura, creds)
}

pub(super) fn verify_device_credential_for_credentials(
    caller_ura: &str,
    creds: crate::daemon::persistence::config::Credentials,
) -> CredentialWarmupOutcome {
    let expected_caller = crate::core::ura::device_ura(&creds.realm, &creds.node_id);
    if expected_caller != caller_ura {
        return CredentialWarmupOutcome::Skipped {
            reason: format!(
                "credentials caller {expected_caller} does not match session caller {caller_ura}"
            ),
        };
    }

    let api_base = creds.api_base();
    let url = format!("{api_base}/api/v1/devices/verify-credential");
    let response = ureq::post(&url)
        .timeout(Duration::from_secs(5))
        .send_json(serde_json::json!({
            "node_id": creds.node_id,
            "credential_token": creds.credential_token,
        }));

    match response {
        Ok(resp) if (200..300).contains(&resp.status()) => {
            CredentialWarmupOutcome::Verified { api_base }
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.into_string().unwrap_or_default();
            CredentialWarmupOutcome::Failed {
                api_base,
                reason: format!("HTTP {status}: {body}"),
            }
        }
        Err(ureq::Error::Status(status, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            CredentialWarmupOutcome::Failed {
                api_base,
                reason: format!("HTTP {status}: {body}"),
            }
        }
        Err(err) => CredentialWarmupOutcome::Failed {
            api_base,
            reason: err.to_string(),
        },
    }
}

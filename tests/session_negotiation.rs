// T2.1 step 2a — frame-0 carrier negotiation facts (DEC-F004)
// ============================================================
// Integration-level pins for the PresenceRegistry negotiation API.
// Lives here (not only in the lib's inline tests) so the contract is
// verified through the public face — and stays runnable even when
// unrelated inline-test modules are mid-refactor.

#![cfg(feature = "axon-pb")]

use easynet_cli::daemon::invocation::bidi::state::presence::{PresenceRegistry, SessionContract};

fn registry() -> PresenceRegistry {
    PresenceRegistry::new()
}

#[tokio::test]
async fn negotiated_insert_remembers_contract_and_surfaces_prior_nonce() {
    let reg = registry();
    let canonical_version = SessionContract::canonical().version;
    let (tx1, _rx1) = tokio::sync::mpsc::channel(1);
    let first = reg
        .insert_negotiated(
            "easynet:///r/t/device/d1".into(),
            tx1,
            SessionContract {
                version: canonical_version,
                claimant_boot_nonce: vec![1; 16],
            },
        )
        .expect("canonical presence key");
    assert!(first.displaced.is_none());
    assert!(first.displaced_claimant_nonce.is_none());
    assert_eq!(
        reg.lookup_dispatch_session("easynet:///r/t/device/d1")
            .map(|session| session.contract_version),
        Some(canonical_version)
    );

    // A different claimant displacing the slot surfaces the prior
    // fingerprint so the accept path can classify the conflict
    // (same-device restart vs two processes fighting over one URA).
    let (tx2, _rx2) = tokio::sync::mpsc::channel(1);
    let second = reg
        .insert_negotiated(
            "easynet:///r/t/device/d1".into(),
            tx2,
            SessionContract {
                version: canonical_version + 1,
                claimant_boot_nonce: vec![2; 16],
            },
        )
        .expect("canonical presence key");
    assert!(second.displaced.is_some());
    assert_eq!(second.displaced_claimant_nonce, Some(vec![1; 16]));
    assert_eq!(
        reg.lookup_dispatch_session("easynet:///r/t/device/d1")
            .map(|session| session.contract_version),
        Some(canonical_version + 1)
    );
}

#[tokio::test]
async fn insert_tracked_registers_canonical_contract() {
    let reg = registry();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let r = reg
        .insert_tracked("easynet:///r/t/device/d2".into(), tx)
        .expect("canonical presence key");
    assert!(r.displaced_claimant_nonce.is_none());
    assert_eq!(
        reg.lookup_dispatch_session("easynet:///r/t/device/d2")
            .map(|session| session.contract_version),
        Some(SessionContract::canonical().version)
    );
}

#[tokio::test]
async fn no_live_session_means_no_contract() {
    assert_eq!(
        registry()
            .lookup_dispatch_session("easynet:///r/t/device/ghost")
            .map(|session| session.contract_version),
        None
    );
}

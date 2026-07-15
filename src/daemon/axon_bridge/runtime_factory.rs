//! Boot-time factory for the shared `LocalRuntime` instance.
//!
//! Centralises the "build Axon's runtime + install KeyResolver +
//! install LedgerSink" recipe so that:
//!
//!   * production boot (`daemon::invocation::start_daemon_invocation_transport`)
//!     gets the runtime wired the same way every time, and
//!   * integration tests can call the same factory with a tempdir
//!     ledger + a stub trust anchor without duplicating the
//!     plumbing.
//!
//! The runtime returned here owns no `AbilityFn` yet — Phase 3
//! registers those. Phase 2 only ensures the runtime exists and is
//! reachable from the dispatch service.

use std::sync::Arc;
#[cfg(test)]
use std::{collections::HashMap, sync::Mutex};

#[cfg(test)]
use easynet_axon::invocation::{
    sha256, AgentIdentity, Ed25519ReceiptSigningAuthority, ReceiptSigningAuthority,
    ReceiptSigningAuthorityProvider,
};
use easynet_axon::invocation::{
    AxiomBinding, AxonError, InvocationLedger, KeyResolver, LedgerSink, LocalRuntime,
};

use crate::daemon::identity::local_invocation::LocalSystemKeyResolver;
use crate::daemon::identity::receipt_signing::load_runtime_signing_authority_providers;
pub use crate::daemon::identity::receipt_signing::ProductionReceiptAuthorityConfig;

/// Build the daemon production runtime from persistent owner-bound key-service
/// capabilities. No private key enters the CLI process.
pub fn build_production_local_runtime(
    config: ProductionReceiptAuthorityConfig,
) -> Result<Arc<LocalRuntime>, AxonError> {
    let providers = load_runtime_signing_authority_providers(config)?;
    Ok(LocalRuntime::new_with_signing_authority_providers(
        Some(providers.invocation),
        providers.receipt,
    ))
}

/// Construct an explicit local-fast `Arc<LocalRuntime>` for daemon tests.
///
/// Test wiring includes:
///
/// - the caller-key resolver supplied by the services layer, wrapped with the
///   daemon-local `_system.local` resolver branch, so external signed calls
///   and daemon-internal signed calls both use Axon's public admission path;
/// - the ledger sink backed by `InvocationLedger` (so every
///   terminal invocation persists into `<ledger_dir>/invocations.redb`
///   without the dispatch arm needing to manually build a record).
///
/// Production boot must use [`build_production_local_runtime`].
#[cfg(test)]
#[must_use]
pub fn build_local_runtime(
    key_resolver: Option<Arc<dyn KeyResolver>>,
    ledger: Option<Arc<InvocationLedger>>,
) -> Arc<LocalRuntime> {
    let runtime = build_ephemeral_test_runtime();
    configure_local_runtime(&runtime, key_resolver, ledger);
    runtime
}

#[cfg(test)]
#[must_use]
pub(crate) fn build_ephemeral_test_runtime() -> Arc<LocalRuntime> {
    LocalRuntime::new_with_receipt_signing_authority_provider(
        ephemeral_test_receipt_signing_authority_provider(),
    )
}

#[cfg(test)]
pub(crate) fn ephemeral_test_receipt_signing_authority_provider(
) -> Arc<dyn ReceiptSigningAuthorityProvider> {
    Arc::new(EphemeralTestReceiptSigningAuthorityProvider::default())
}

#[cfg(test)]
#[derive(Default)]
struct EphemeralTestReceiptSigningAuthorityProvider {
    authorities: Mutex<HashMap<AgentIdentity, Arc<dyn ReceiptSigningAuthority>>>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl ReceiptSigningAuthorityProvider for EphemeralTestReceiptSigningAuthorityProvider {
    async fn resolve(
        &self,
        callee: &AgentIdentity,
    ) -> Result<Arc<dyn ReceiptSigningAuthority>, AxonError> {
        let mut authorities = self
            .authorities
            .lock()
            .map_err(|_| AxonError::internal("ephemeral_test_receipt_authority_lock_poisoned"))?;
        if let Some(authority) = authorities.get(callee) {
            return Ok(Arc::clone(authority));
        }
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&sha256(callee.ura.as_bytes()));
        let authority: Arc<dyn ReceiptSigningAuthority> =
            Arc::new(Ed25519ReceiptSigningAuthority::self_signed(
                callee.clone(),
                signing_key,
                "cli-test-runtime",
            ));
        authorities.insert(callee.clone(), Arc::clone(&authority));
        Ok(authority)
    }

    fn resolve_signer_key(
        &self,
        signer_ura: &str,
    ) -> Result<Option<ed25519_dalek::VerifyingKey>, AxonError> {
        let authorities = self
            .authorities
            .lock()
            .map_err(|_| AxonError::internal("ephemeral_test_receipt_authority_lock_poisoned"))?;
        Ok(authorities
            .values()
            .find(|authority| authority.signer_identity().ura == signer_ura)
            .map(|authority| authority.verifying_key()))
    }
}

/// Install daemon-specific admission and ledger adapters onto an
/// already-created runtime. Daemon boot uses this when ability
/// registration has to happen before `invocation_transport` finishes loading
/// the transport config and trust anchor.
pub fn configure_local_runtime(
    runtime: &Arc<LocalRuntime>,
    key_resolver: Option<Arc<dyn KeyResolver>>,
    ledger: Option<Arc<InvocationLedger>>,
) {
    runtime.set_admission_key_resolver(Arc::new(
        LocalSystemKeyResolver::new(key_resolver)
            .with_receipt_signing_runtime(Arc::downgrade(runtime)),
    ));
    if let Some(ledger) = ledger {
        runtime.set_ledger_sink(
            LedgerSink::new(ledger)
                .with_invocation_ura(ledger_invocation_ura)
                .with_ability_ura(ledger_route_ura),
        );
    }
}

fn ledger_invocation_ura(invocation_id: &str, binding: &AxiomBinding) -> String {
    easynet_axon::ura::invocation_record_ura_for_binding(
        &binding.subject.ura,
        &binding.callee.ura,
        &binding.caller.ura,
        invocation_id,
    )
    .unwrap_or_else(|| {
        easynet_axon::ura::invocation_history_resource_ura(
            "_system",
            "hub.invocations",
            invocation_id,
        )
    })
}

fn ledger_route_ura(ability_name: &str, binding: &AxiomBinding) -> String {
    // RFC-005: a route names the same canonical `/ability/` URA the owner
    // publishes. Axon's ledger sink passes the daemon registry key here
    // (`liangbing.chat`, `fs.read`, ...), while public Ability URAs
    // store the owner-local name (`chat`, `fs.read`, ...). Project through
    // the CLI URA boundary object before calling Axon's canonical builder;
    // do not duplicate URA grammar in this adapter.
    let callee_public_name =
        crate::core::ura::owner_local_ability_name(&binding.callee.ura, ability_name);
    let caller_public_name =
        crate::core::ura::owner_local_ability_name(&binding.caller.ura, ability_name);

    easynet_axon::ura::published_route_ura(&binding.callee.ura, &callee_public_name)
        .or_else(|| {
            easynet_axon::ura::published_route_ura(&binding.caller.ura, &caller_public_name)
        })
        .unwrap_or_else(|| {
            easynet_axon::ura::hub_ability_ura("_system", &format!("system.{ability_name}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use easynet_axon::invocation::axiom::AuthorityBinding;
    use easynet_axon::invocation::{AgentIdentity, CausalContext, SubjectIdentity, UraProfile};

    #[test]
    fn build_local_runtime_returns_handle_with_admission_and_sink_optional() {
        // Smoke: factory accepts (anchor, None) without panicking and
        // returns a runtime. The actual end-to-end "ledger writes
        // happen on terminal" semantics are pinned by Axon's own
        // tests + Phase 0's tests/axon_runtime_smoke.rs; here we
        // just need to know the wiring assembles.
        let rt = build_local_runtime(None, None);
        // Arc strong count > 0 — proves the runtime was built.
        assert!(Arc::strong_count(&rt) >= 1);
    }

    #[test]
    fn ledger_resolvers_use_axon_canonical_ura_helpers() {
        let caller = AgentIdentity::new(
            "easynet:///r/localhost/user/dev",
            UraProfile::EasynetStrictV2,
        );
        let callee = AgentIdentity::new(
            "easynet:///r/localhost/agent/dev.liangbing",
            UraProfile::EasynetStrictV2,
        );
        let subject = SubjectIdentity::new(
            "easynet:///r/localhost/user/dev",
            UraProfile::EasynetStrictV2,
        );
        let binding = AxiomBinding {
            caller: caller.clone(),
            callee,
            subject,
            invocation_nonce: [0u8; 16],
            causal: CausalContext::None,
            payload_digest: [0u8; 32],
            callee_signature: None,
            signer_binding: None,
            host_attestation: Vec::new(),
            ability_binding: "liangbing.chat".to_string(),
            authority_binding: AuthorityBinding::Self_ {
                principal_ura: caller.ura.clone(),
            },
        };

        assert_eq!(
            ledger_route_ura("liangbing.chat", &binding),
            "easynet:///r/localhost/ability/dev.liangbing.chat"
        );
        assert_eq!(
            ledger_invocation_ura("inv_123", &binding),
            "easynet:///r/localhost/resource/dev/invocation/inv_123/history"
        );

        let fallback_caller = AgentIdentity::new(
            "easynet:///r/localhost/user/dev",
            UraProfile::EasynetStrictV2,
        );
        let fallback_binding = AxiomBinding {
            caller: fallback_caller.clone(),
            callee: AgentIdentity::new(
                crate::core::ura::hub_ura("localhost"),
                UraProfile::EasynetStrictV2,
            ),
            subject: SubjectIdentity::new(
                "easynet:///r/localhost/user/dev",
                UraProfile::EasynetStrictV2,
            ),
            invocation_nonce: [0u8; 16],
            causal: CausalContext::None,
            payload_digest: [0u8; 32],
            callee_signature: None,
            signer_binding: None,
            host_attestation: Vec::new(),
            ability_binding: "chat".to_string(),
            authority_binding: AuthorityBinding::Self_ {
                principal_ura: fallback_caller.ura.clone(),
            },
        };
        // RFC-005 removed the device ability *resource* route; the
        // last-resort fallback (neither binding URA publishes the route)
        // now names a hub-owned system ability URA.
        assert_eq!(
            ledger_route_ura("chat", &fallback_binding),
            "easynet:///r/_system/ability/hub.system.chat"
        );
    }
}

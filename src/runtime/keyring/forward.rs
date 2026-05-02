// EasyNet CLI — Federation forward-invoke hook (RFC-002 §5.2)
// =============================================================
//
// Process-global registration slot for the forward-invoke routing
// hook used by `invoke_ability::dispatch` when a target URA is
// known to the keyring's peer table but not to the local agent
// registry. Tonight's wiring keeps the contract minimal:
//
//   trait CliForwardInvoker {
//       fn invoke(&self, target_uri: &str, ability: &str, args: Value)
//           -> anyhow::Result<Value>;
//       fn knows_target(&self, target_uri: &str) -> bool;
//   }
//
// At daemon boot the federation transport (bridge + tenant + realm)
// is bundled into a concrete impl and `set_forward_invoker` is called
// once. invoke_ability checks `is_federation_target` against the
// hook before bailing with target_not_registered.
//
// Tests can install a fake invoker (see EXP-3 integration tests).
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use serde_json::Value;
use std::sync::{Arc, OnceLock};

/// Hook used by `invoke_ability::dispatch` to route an invoke to a
/// remote agent. Implementations are responsible for verifying
/// authorization (peer table membership / access policy) before
/// dispatching; the dispatch layer treats a returned Ok(value) as
/// the unwrapped result body and returns it to the caller verbatim.
pub trait CliForwardInvoker: Send + Sync {
    /// Returns true when this invoker can route to the named target.
    /// Used so `dispatch` can fall back to `target_not_registered`
    /// without attempting a futile network call.
    fn knows_target(&self, target_uri: &str) -> bool;

    /// Issue the forward invoke. Returns the unwrapped result body
    /// (the same Value the local handler would have returned). On
    /// hub-side typed failure (AXON_TARGET_OFFLINE etc.) returns
    /// Err with the typed code in the message string.
    fn invoke(&self, target_uri: &str, ability: &str, args: Value) -> anyhow::Result<Value>;
}

static FORWARD_INVOKER: OnceLock<Arc<dyn CliForwardInvoker>> = OnceLock::new();

/// Install the process-global forward invoker. Called at daemon boot
/// once federation transport is ready. Subsequent calls are
/// no-ops — the slot is set-once.
pub fn set_forward_invoker(invoker: Arc<dyn CliForwardInvoker>) -> bool {
    FORWARD_INVOKER.set(invoker).is_ok()
}

/// Borrow the registered forward invoker, if any.
pub fn forward_invoker() -> Option<Arc<dyn CliForwardInvoker>> {
    FORWARD_INVOKER.get().cloned()
}

/// Heuristic: detect URAs of the federation shape. Accepts both
/// the URA-conformant `easynet:///r/{prv,org}/reg/agent.<id>` and
/// the legacy `easynet:///r/<tenant>/agent/<id>` shape. Bare names
/// (no scheme) are treated as local.
pub fn is_federation_target(target: &str) -> bool {
    target.starts_with("easynet:///r/")
}

/// Test routing slot. Tests install a closure here; the static
/// `TestSinkInvoker` (registered at first test that touches it) calls
/// into the slot. This works around the OnceLock set-once nature so
/// any number of tests can share routing without contending for the
/// global.
///
/// **Why not cfg(test)**: integration tests in `tests/` link the lib
/// in non-test config and need to drive these helpers, so we expose
/// them publicly. The guard against production misuse is the
/// `set_test_router` name itself plus the fact that no production
/// boot path calls it. A future RFC can split this into a dedicated
/// `test-support` feature when we want a stricter compile-time gate.
pub struct TestSinkInvoker;

type TestRouter = Box<dyn Fn(&str, &str, Value) -> anyhow::Result<Value> + Send + Sync>;

type TestKnower = Box<dyn Fn(&str) -> bool + Send + Sync>;

static TEST_ROUTER: OnceLock<std::sync::Mutex<Option<TestRouter>>> = OnceLock::new();
static TEST_KNOWER: OnceLock<std::sync::Mutex<Option<TestKnower>>> = OnceLock::new();

impl CliForwardInvoker for TestSinkInvoker {
    fn knows_target(&self, target_uri: &str) -> bool {
        let lock = TEST_KNOWER.get_or_init(|| std::sync::Mutex::new(None));
        match lock.lock().unwrap().as_ref() {
            Some(f) => f(target_uri),
            None => false,
        }
    }
    fn invoke(&self, target_uri: &str, ability: &str, args: Value) -> anyhow::Result<Value> {
        let lock = TEST_ROUTER.get_or_init(|| std::sync::Mutex::new(None));
        let g = lock.lock().unwrap();
        match g.as_ref() {
            Some(f) => f(target_uri, ability, args),
            None => Err(anyhow::anyhow!("test router not installed")),
        }
    }
}

pub fn install_test_sink_once() {
    let _ = FORWARD_INVOKER.set(Arc::new(TestSinkInvoker));
}

pub fn set_test_router<F>(router: F)
where
    F: Fn(&str, &str, Value) -> anyhow::Result<Value> + Send + Sync + 'static,
{
    install_test_sink_once();
    let lock = TEST_ROUTER.get_or_init(|| std::sync::Mutex::new(None));
    *lock.lock().unwrap() = Some(Box::new(router));
}

pub fn set_test_knower<F>(knower: F)
where
    F: Fn(&str) -> bool + Send + Sync + 'static,
{
    install_test_sink_once();
    let lock = TEST_KNOWER.get_or_init(|| std::sync::Mutex::new(None));
    *lock.lock().unwrap() = Some(Box::new(knower));
}

pub fn clear_test_routing() {
    if let Some(l) = TEST_ROUTER.get() {
        *l.lock().unwrap() = None;
    }
    if let Some(l) = TEST_KNOWER.get() {
        *l.lock().unwrap() = None;
    }
}

/// Serialize tests that mutate the global forward routing. Cargo
/// runs tests in parallel by default; without this guard, two tests
/// touching the router would race and produce flaky failures.
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_federation_uras() {
        assert!(is_federation_target("easynet:///r/prv/reg/agent.foo"));
        assert!(is_federation_target(
            "easynet:///r/silan.localhost/agent/01HXYZ"
        ));
        assert!(!is_federation_target("foo.bar"));
        assert!(!is_federation_target("local-only"));
        assert!(!is_federation_target(""));
    }
}

// EasyNet CLI — Control-plane Accept Loop
// ========================================
//
// File: src/services/control/server.rs
// Description: Ties transport + ability_proxy together. `run()` is
//              the daemon's call site: bind the listener, write
//              `control.json`, accept connections forever, hand
//              each one to a per-connection task.
//
// v1 status — skeleton
// --------------------
// `run()` is a sync signature today so daemon bin code can call it
// without a tokio runtime. The real accept-loop (tokio-based, per
// connection spawn + LengthDelimitedCodec read loop) lands in a
// follow-up PR-DAEMON commit once `transport.rs` and
// `discovery.rs` are wired. Until then the function returns an
// explicit skeleton error, matching the pattern used across this
// layer so "easynet self control start" fails loudly rather than
// pretending to work.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use crate::runtime::kernel_api::KernelApi;
use crate::services::control::ability_proxy::AbilityProxy;

/// Run the Control-plane accept loop with the given Kernel.
///
/// v1: sync signature, skeleton body. Real implementation lands in
/// a follow-up PR-DAEMON commit after transport.rs + discovery.rs
/// are wired. The signature is already tokio-ready: a `#[tokio::main]`
/// daemon bin wrapping this function will drop in trivially when
/// the body becomes async.
pub fn run(_kernel: Arc<dyn KernelApi>) -> anyhow::Result<()> {
    anyhow::bail!(
        "control-plane server is a skeleton in v1 of PR-DAEMON; \
         transport.rs + discovery.rs follow-up commits land the real \
         accept loop"
    )
}

/// Construct an AbilityProxy wrapping the given Kernel. Exists as a
/// named helper so a future test harness can exercise the proxy
/// without going through the full accept loop.
pub fn make_proxy(kernel: Arc<dyn KernelApi>) -> AbilityProxy {
    AbilityProxy::new(kernel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::gateway::NoopGateway;
    use crate::runtime::kernel::Kernel;

    #[test]
    fn run_returns_explicit_skeleton_error() {
        // The v1 accept-loop returns a clear "not wired" error, not
        // a misleading "address in use" or similar. A production
        // operator seeing this message knows immediately that it is
        // a version-mismatch between lib and daemon, not a local
        // environment issue.
        let kernel = Arc::new(Kernel::new(Arc::new(NoopGateway::new())));
        let err = run(kernel).unwrap_err();
        assert!(format!("{err}").contains("skeleton"));
    }

    #[test]
    fn make_proxy_wraps_kernel_handle() {
        // Smoke: the helper exists and returns an AbilityProxy bound
        // to the supplied Kernel. Use it from tests that want to
        // exercise proxy behaviour without spinning up the server.
        let kernel: Arc<dyn KernelApi> =
            Arc::new(Kernel::new(Arc::new(NoopGateway::new())));
        let proxy = make_proxy(Arc::clone(&kernel));
        // Arc::ptr_eq confirms the proxy holds exactly the handle
        // we supplied (not a clone-from-scratch).
        assert!(Arc::ptr_eq(proxy.kernel(), &kernel));
    }
}

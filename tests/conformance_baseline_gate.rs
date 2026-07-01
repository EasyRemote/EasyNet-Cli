//! Baseline conformance gate (SPEC §9.1 item 7, §13.4)
//! ====================================================
//!
//! Spec §9.1 item 7 requires Hub mode and Device mode to *pass baseline
//! conformance* — a missing baseline ability must be a build/CI failure,
//! not a silent runtime gap. The typed model lives in
//! `runtime::ability::conformance`; this integration test is the
//! deterministic gate that exercises it against the **real** daemon
//! registry built by `runtime::agents::build_registry()`.
//!
//! Why an integration test (and not only the in-lib `#[cfg(test)]`
//! checks): linking the crate's public API from `tests/` proves the
//! conformance surface is reachable as a real product contract, and the
//! gate runs in CI independently of the in-lib unit tests.
//!
//! Surfaces covered here:
//!   * `LocalRegistry`   — checked against the real built catalog.
//!   * `AxonRuntimeAdmin` — checked against the daemon's installed
//!     runtime-admin surface (`RuntimeAdminConformance::from_daemon_surface`).
//!   * `DaemonInvocation` — checked against the production route tables
//!     exported beside the tonic `Invoke` / `InvokeStream` match arms.

use easynet_cli::runtime::ability::conformance::{
    DaemonInvocationSurface, DeviceBaseline, HubBaseline, RegistryConformance,
    RuntimeAdminConformance,
};

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Pins `HOME` to a throwaway dir for the duration of the test so the
/// registry builder's home-reading code paths cannot pick up the running
/// user's real config under parallel `cargo test`. Mirrors the
/// established `TestHomeGuard` pattern in `tests/pages_unit.rs`
/// (the crate's own `HomeGuard` is `pub(crate)`, not reachable here).
struct HomeGuard {
    _lock: MutexGuard<'static, ()>,
    temp_dir: PathBuf,
    prev_home: Option<String>,
}

impl HomeGuard {
    fn new() -> Self {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let lock = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp_dir = std::env::temp_dir().join(format!(
            "easynet-conformance-gate-home-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &temp_dir);
        Self {
            _lock: lock,
            temp_dir,
            prev_home,
        }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.prev_home.take() {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

/// Device mode must register every Device baseline `LocalRegistry`
/// ability with the correct call mode.
#[test]
fn device_mode_registry_satisfies_device_baseline() {
    let _home = HomeGuard::new();
    let registry = easynet_cli::runtime::agents::build_registry();
    let report =
        RegistryConformance::new(&registry).check("device", &DeviceBaseline::required_abilities());
    assert!(report.is_conformant(), "{}", report.panic_message());
}

/// Hub mode shares the same local registry; its `LocalRegistry` baseline
/// rows (e.g. `meta.list_abilities` introspection) must also be present.
#[test]
fn hub_mode_registry_satisfies_hub_local_registry_slice() {
    let _home = HomeGuard::new();
    let registry = easynet_cli::runtime::agents::build_registry();
    let report =
        RegistryConformance::new(&registry).check("hub", HubBaseline::required_abilities());
    assert!(report.is_conformant(), "{}", report.panic_message());
}

/// The daemon runtime-admin surface (`session.open`, `runtime.invoke_remote`
/// bidi carriers + `runtime.bootstrap_self_identity`) must be installed.
/// The installed set is derived from the production dispatcher constant via
/// `from_daemon_surface`, so this gate cannot pass on a hand-mirrored list.
#[test]
fn daemon_runtime_admin_surface_satisfies_hub_baseline() {
    let report = RuntimeAdminConformance::from_daemon_surface()
        .check("hub", HubBaseline::required_abilities());
    assert!(report.is_conformant(), "{}", report.panic_message());
}

/// Hub mode's daemon-owned Invocation routes must be present in the actual
/// production route surface, not a test-local mirror.
#[test]
fn daemon_invocation_surface_satisfies_hub_baseline() {
    let report = DaemonInvocationSurface::from_daemon_surface()
        .check("hub", HubBaseline::required_abilities());
    assert!(report.is_conformant(), "{}", report.panic_message());
}

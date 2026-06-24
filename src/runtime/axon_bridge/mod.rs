//! Axon SDK bridge layer.
//!
//! CLI used to ship a full parallel implementation of the
//! "callee-side" invocation pipeline:
//!
//!   * its own dispatch surface (`AxonAbilityCatalog`),
//!   * its own transport policy gate (`AdmissionFacade`),
//!   * its own in-memory receipt store (`SharedReceiptStore`,
//!     deleted in Phase 5a — `LedgerSink` is now the canonical
//!     persistence surface for runtime outcomes).
//!
//! Axon's `easynet_axon::invocation` module already defines the
//! canonical implementations of every one of those concerns —
//! `LocalRuntime`, `invoke_externally_signed_*`, `LedgerSink`,
//! `KeyResolver`. The bridge layer here is the gradual handoff:
//! each submodule contains one small adapter that lets the rest of
//! the CLI consume an Axon type while existing services continue to
//! own the data they're responsible for. Service-owned adapters
//! such as `RealmTrustAnchor` -> `KeyResolver` live in `services`;
//! this module depends only on Axon SDK types and runtime-side
//! ability dispatch glue.
//!
//! Phase mapping (matches the task list at the top of the migration):
//!
//!   * Phase 1 — services adapt `RealmTrustAnchor` → `KeyResolver`.
//!   * Phase 2 — `runtime_factory`: build & wire `Arc<LocalRuntime>`
//!     + `LedgerSink` at daemon boot.
//!   * Phase 3 — registration sites write directly to the shared
//!     `LocalRuntime`; there is no legacy registry mirror.
//!   * Phase 4 — `dispatch_shim`: `<self>.invoke_remote` /
//!     `federation.forward_invoke` dispatch arms route through the
//!     `local_runtime_request` factory and Axon's public
//!     descriptor-bound request APIs instead of CLI's bespoke
//!     dispatch. The wire shim itself depends on tonic-generated Axon
//!     proto types and is therefore feature-gated with `axon-pb`.
//!   * Phase 5 — the CLI-side parallel implementations get deleted.

pub(crate) mod descriptor_ref;
#[cfg(feature = "axon-pb")]
pub mod dispatch_shim;
pub mod hot_agent_registrar;
pub(crate) mod local_runtime_request;
pub mod runtime_factory;
#[cfg(feature = "axon-pb")]
pub(crate) mod wire_descriptor;

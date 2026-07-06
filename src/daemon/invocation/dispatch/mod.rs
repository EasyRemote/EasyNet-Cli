//! Invocation RPC service shell and local/runtime dispatch implementations.

#[cfg(feature = "axon-pb")]
pub mod client;
pub mod daemon_invocation_service;
pub(crate) mod deps;
pub(crate) mod descriptor_binding;
pub mod federation_wrappers;
pub mod invocation_wire;
pub mod local_runtime_invoker;
pub mod local_session_dispatcher;
mod request;
pub(crate) mod unary_dispatcher;

pub use request::{
    CallerSignatureMaterial, DaemonInvocation, DaemonInvocationBuilder, InvocationDraft,
    InvocationTuple, KeyringLocalDaemonInvocationSigner, LocalDaemonInvocationSigner,
    PrepareOptions, PreparedInvocation, SignedInvocation, SignerPolicy, SignerPolicyMode,
    SigningMaterial,
};

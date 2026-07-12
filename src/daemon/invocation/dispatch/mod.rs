//! Invocation RPC service shell and local/runtime dispatch implementations.

#[cfg(feature = "axon-pb")]
pub mod client;
#[cfg(feature = "axon-pb")]
pub mod daemon_invocation_service;
#[cfg(feature = "axon-pb")]
pub(crate) mod deps;
#[cfg(feature = "axon-pb")]
pub(crate) mod descriptor_binding;
#[cfg(feature = "axon-pb")]
pub mod federation_wrappers;
#[cfg(feature = "axon-pb")]
pub mod invocation_wire;
pub mod local_runtime_invoker;
#[cfg(feature = "axon-pb")]
pub mod local_session_dispatcher;
#[cfg(feature = "axon-pb")]
mod request;
#[cfg(feature = "axon-pb")]
pub(crate) mod unary_dispatcher;

#[cfg(feature = "axon-pb")]
pub use request::{
    CallerSignatureMaterial, DaemonInvocation, DaemonInvocationBuilder, InvocationDraft,
    InvocationTuple, KeyServiceLocalDaemonInvocationSigner, LocalDaemonInvocationSigner,
    PrepareOptions, PreparedInvocation, SignedInvocation, SignerPolicy, SignerPolicyMode,
    SigningMaterial,
};

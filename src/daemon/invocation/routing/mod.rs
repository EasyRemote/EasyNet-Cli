//! Daemon-local target resolution and explicit federation routing helpers.

#[cfg(feature = "axon-pb")]
pub mod hub_resolver;
#[cfg(feature = "axon-pb")]
pub(crate) mod remote_invoke;
#[cfg(feature = "axon-pb")]
pub mod route_resolver;
pub mod target;

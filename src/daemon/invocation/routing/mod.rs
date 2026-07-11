//! Daemon-local target resolution and explicit federation routing helpers.

pub mod hub_resolver;
#[cfg(feature = "axon-pb")]
pub(crate) mod remote_invoke;
pub mod route_resolver;
pub mod target;

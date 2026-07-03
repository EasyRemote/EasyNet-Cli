//! Daemon-local target resolution and explicit federation routing helpers.

#[cfg(feature = "axon-pb")]
pub(crate) mod federation_invoke;
pub mod hub_resolver;
pub mod route_resolver;
pub mod target;

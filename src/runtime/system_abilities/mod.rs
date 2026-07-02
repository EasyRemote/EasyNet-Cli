//! Daemon-owned system ability handlers grouped by product module.
//!
//! Product grouping here is source organization only. The control-plane model
//! remains in `runtime::ability`, descriptor/catalog projection remains in
//! `daemon::ability::catalog`, and Axon glue remains in
//! `runtime::axon_bridge`.

pub mod agents;
pub mod automation;
pub mod device_control;
pub mod governance;
pub mod integrations;
#[cfg(test)]
mod real_invoke_tests;
pub mod resources;

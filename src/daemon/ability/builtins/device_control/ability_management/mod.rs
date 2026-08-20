//! Ability deployment management state machine.
//!
//! `ops` owns the public ability.deploy/uninstall handlers, `registrar` owns
//! runtime binding and replay, `store` owns the durable installed-ability
//! catalog, and `publish` owns ability.publish/unpublish for agent manifests.

pub mod ops;
pub mod publish;
pub mod registrar;
pub mod store;

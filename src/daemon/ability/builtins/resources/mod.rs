//! Resource-plane system ability handlers.
//!
//! Resources are implementation/runtime assets exposed through governed
//! abilities: skills, context, daemon files store, media resource projection,
//! pages, and voice call state. They do not define Ability ontology.

pub mod context;
pub mod files_store;
pub mod list;
pub mod media;
pub mod pages;
pub mod refresh_remote_targets;
pub mod skills;
pub mod voice;
pub mod voice_contract;
pub mod watch_remote_targets;

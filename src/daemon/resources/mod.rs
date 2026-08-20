//! Daemon-owned resource reference domains.
//!
//! This module contains EasyNet-Cli daemon resource policy, not Axon
//! protocol canonicalization. Axon owns Invocation and receipt semantics;
//! the daemon owns local resource mapping such as filesystem virtual roots.

pub mod context;
pub mod files;
pub mod media;
pub mod pages;
pub mod projection;
pub mod skills;

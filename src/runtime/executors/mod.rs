//! Reusable implementation executors for manifest-bound abilities.
//!
//! Executors are implementation machinery. They do not register public
//! abilities and do not own descriptor identity.

pub mod eal;
pub mod host_stream;
pub mod http;
pub mod shell;
pub mod template;

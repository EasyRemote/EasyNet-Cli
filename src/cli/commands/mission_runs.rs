//! Public CLI facade for daemon-owned mission orchestration.
//!
//! Mission execution, lifecycle state, and persistence belong to the
//! runtime. Keeping this facade preserves the existing CLI library surface
//! while ensuring dependency flow remains `cli -> daemon`.

pub use crate::daemon::execution::mission::orchestration::*;

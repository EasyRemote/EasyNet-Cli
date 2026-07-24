//! Public CLI facade for daemon-owned mission orchestration.
//!
//! Mission execution, lifecycle state, and persistence belong to the
//! runtime. This facade exposes only the CLI read/cancel projection used by
//! command handlers and watch surfaces; the daemon execution service stays
//! owned by `daemon::execution::mission::orchestration`.

pub use crate::daemon::execution::mission::orchestration::{
    cancel_run, find_run, list_runs, root_dir, CancelOutcome, MissionRunDir, MissionRunMeta,
    MissionRunStatus, MissionRunSummary,
};

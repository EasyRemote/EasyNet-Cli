//! Automation and composition system ability handlers.
//!
//! These handlers expose scheduled work, loops, missions, discussions, and
//! long-running judge/worker orchestration. Long-lived state remains in
//! `daemon::execution`; this module owns the governed ability surfaces.

pub mod discuss;
pub mod loop_ability;
pub mod mission;
pub mod orchestration;
pub mod schedule;
pub mod think;

//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/boot/identity_fact.rs
//! Description: boot-private identity fact classifiers.
//!
//! Protocol Responsibility:
//! - Keeps daemon boot lifecycle and process attach validation from collapsing
//!   absent identity facts into string defaults.
//!
//! Implementation Approach:
//! - Uses small value-state enums that classify optional wire/discovery facts
//!   before caller-specific error projection.
//!
//! Usage Contract:
//! - Boot modules may use these classifiers internally; they are not public
//!   daemon SDK or Axon protocol types.
//!
//! Architectural Position:
//! - `daemon::boot` identity validation support for lifecycle/process attach
//!   state machines.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeviceNodeIdFact<'a> {
    Present(&'a str),
    Missing,
    Blank,
}

impl<'a> DeviceNodeIdFact<'a> {
    pub(super) fn from_optional(value: Option<&'a str>) -> Self {
        match value {
            Some(value) if value.trim().is_empty() => Self::Blank,
            Some(value) => Self::Present(value),
            None => Self::Missing,
        }
    }

    pub(super) fn present_value(self) -> Option<&'a str> {
        match self {
            Self::Present(value) => Some(value),
            Self::Missing | Self::Blank => None,
        }
    }

    pub(super) fn mismatch_value(self) -> String {
        match self {
            Self::Present(value) => value.to_string(),
            Self::Missing => "<missing>".to_string(),
            Self::Blank => "<blank>".to_string(),
        }
    }
}

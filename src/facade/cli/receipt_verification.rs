// EasyNet CLI - receipt verification projection
// =============================================
//
// File: src/facade/cli/receipt_verification.rs
// Description: CLI-local receipt verification state shared by invocation
//              surfaces. This module names what the CLI has actually proven;
//              it is not an Axon receipt verifier and must not report a
//              positive or negative verification result unless a verifier ran.

use std::fmt;

/// CLI-local receipt-chain verification state.
///
/// Invariant 1: `NotPerformed` means this process did not perform offline
/// verification. It is not equivalent to "verification failed".
///
/// Invariant 2: ledger-projected verification remains a separate field because
/// it describes what the daemon persisted, not what this CLI process proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliReceiptChainVerification {
    NotPerformed,
}

impl CliReceiptChainVerification {
    /// State emitted by current CLI surfaces until a real verifier is wired.
    pub const fn not_performed() -> Self {
        Self::NotPerformed
    }

    /// Stable operator-facing label used in table/TUI renderers.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotPerformed => "not_performed",
        }
    }
}

impl fmt::Display for CliReceiptChainVerification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

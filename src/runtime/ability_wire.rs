// EasyNet CLI - ability wire profiles
// ====================================
//
// File: src/runtime/ability_wire.rs
// Description: Canonical runtime mapping from ability name to bidi wire codec.
//
// Protocol Responsibility:
// - Names the local bidi wire adapter used by daemon gRPC and `<self>.session`.
// - Keeps service dispatch from hard-coding every adapter branch.
//
// Architectural Position:
// - Runtime metadata boundary. Services ask this module what wire profile an
//   ability uses; they do not own per-ability wire policy.

/// Bidi wire codec used when an ability crosses the daemon/Axon session bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbilityBidiWireKind {
    /// Terminal PTY stream. Binary chunks carry terminal bytes; supported
    /// control frames map to terminal control messages such as resize.
    Pty,
    /// File-transfer stream. Binary chunks and JSON control frames follow the
    /// daemon's file-transfer envelope contract.
    FileTransfer,
    /// JSON control-frame stream. Input and output payloads are structured JSON
    /// values owned by the ability implementation.
    JsonFrames,
}

/// Return the declared bidi wire profile for a locally hosted ability.
pub fn bidi_wire_kind_for(ability: &str) -> Option<AbilityBidiWireKind> {
    if ability == crate::runtime::agents::pty_attach_ability::ABILITY_PTY_SESSION_ATTACH {
        return Some(AbilityBidiWireKind::Pty);
    }
    if ability == crate::runtime::agents::file_transfer_ability::ABILITY_FILE_TRANSFER {
        return Some(AbilityBidiWireKind::FileTransfer);
    }
    None
}

/// Return true when the runtime has a daemon/session wire adapter for `ability`.
pub fn is_bidi_wire_ability(ability: &str) -> bool {
    bidi_wire_kind_for(ability).is_some()
}

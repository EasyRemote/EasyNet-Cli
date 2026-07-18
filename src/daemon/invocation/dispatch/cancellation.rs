//! Canonical Invocation lifecycle cancellation authority.
//!
//! Cancellation is an ordinary descriptor-bound `invocation.cancel`
//! Invocation. This registry is only the daemon-local lifecycle index used by
//! that ability after Axon admission; transport metadata has no control
//! semantics.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use axon_sdk::invocation::{
    AxonError, DescriptorBoundEnvelope, FinalizedInvocation, InvocationHandle,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const DEFAULT_CANCEL_REASON: &str = "user_request";
const MAX_CANCEL_REASON_BYTES: usize = 1_024;
const LIFECYCLE_HASH_BYTES: usize = 32;
const MAX_TRACKED_INVOCATIONS: usize = 4_096;

pub const ABILITY_INVOCATION_CANCEL: &str =
    crate::daemon::ability::names::governance::INVOCATION_CANCEL;

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationCancelCommand {
    pub target_lifecycle_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_invocation_id: Option<String>,
    pub reason: String,
}

impl InvocationCancelCommand {
    pub fn new(
        target_lifecycle_hash: impl Into<String>,
        target_invocation_id: Option<String>,
        reason: impl Into<String>,
    ) -> Result<Self, InvocationCancellationError> {
        let target_lifecycle_hash = normalize_lifecycle_hash(target_lifecycle_hash.into())?;
        let target_invocation_id = target_invocation_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let reason = normalize_reason(reason.into())?;
        Ok(Self {
            target_lifecycle_hash,
            target_invocation_id,
            reason,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct InvocationCancelResult {
    pub target_lifecycle_hash: String,
    pub target_invocation_id: String,
    pub accepted: bool,
    pub already_terminal: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct LifecycleAuthority {
    caller_ura: String,
    authority_ura: String,
}

#[derive(Clone, Default)]
pub struct InvocationCancellationRegistry {
    inner: Arc<Mutex<RegistryState>>,
}

/// Registration token binding one admitted descriptor-bound envelope to its
/// Axon lifecycle handle. Transport adapters retain this token until canonical
/// finalization, then transition the registry entry to its replayable terminal
/// state. Local carrier close never performs this transition.
#[derive(Clone)]
pub(crate) struct RegisteredInvocationLifecycle {
    registry: InvocationCancellationRegistry,
    key: String,
    handle: InvocationHandle,
}

impl RegisteredInvocationLifecycle {
    pub(crate) fn register(
        registry: InvocationCancellationRegistry,
        envelope: &DescriptorBoundEnvelope,
        handle: InvocationHandle,
    ) -> Result<Self, InvocationCancellationError> {
        let key = registry.register(envelope, handle.clone())?;
        Ok(Self {
            registry,
            key,
            handle,
        })
    }

    fn mark_terminal(&self) {
        self.registry.mark_terminal(&self.key, self.handle.clone());
    }

    pub(crate) async fn finalized(&self) -> Result<FinalizedInvocation, AxonError> {
        let finalized = self.handle.finalized().await?;
        self.mark_terminal();
        Ok(finalized)
    }

    pub(crate) async fn cancel_and_finalize(
        &self,
        reason: impl Into<String>,
    ) -> Result<FinalizedInvocation, AxonError> {
        self.handle.cancel(reason).await?;
        self.finalized().await
    }
}

#[derive(Default)]
struct RegistryState {
    entries: HashMap<String, RegistryEntry>,
    terminal_order: VecDeque<String>,
}

impl RegistryState {
    fn retain_terminal_key(&mut self, key: &str) {
        if !self.terminal_order.iter().any(|retained| retained == key) {
            self.terminal_order.push_back(key.to_string());
        }
    }

    fn reserve_entry_slot(&mut self) -> bool {
        while self.entries.len() >= MAX_TRACKED_INVOCATIONS {
            let Some(expired) = self.terminal_order.pop_front() else {
                return false;
            };
            if matches!(
                self.entries.get(&expired),
                Some(RegistryEntry::Terminal { .. })
            ) {
                self.entries.remove(&expired);
            }
        }
        true
    }
}

enum RegistryEntry {
    Active {
        authority: LifecycleAuthority,
        handle: InvocationHandle,
    },
    Terminal {
        authority: LifecycleAuthority,
        handle: InvocationHandle,
    },
}

impl RegistryEntry {
    fn authority(&self) -> &LifecycleAuthority {
        match self {
            Self::Active { authority, .. } | Self::Terminal { authority, .. } => authority,
        }
    }

    fn handle(&self) -> &InvocationHandle {
        match self {
            Self::Active { handle, .. } | Self::Terminal { handle, .. } => handle,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InvocationCancellationError {
    #[error("invalid target lifecycle hash")]
    InvalidLifecycleHash,
    #[error("invocation cancel reason exceeds {MAX_CANCEL_REASON_BYTES} bytes")]
    CancelReasonTooLong,
    #[error("duplicate active invocation lifecycle `{0}`")]
    DuplicateActive(String),
    #[error("invocation cancellation registry capacity exhausted")]
    CapacityExhausted,
    #[error("cancel target lifecycle `{0}` is not registered")]
    TargetNotFound(String),
    #[error("cancel target invocation id does not match the registered lifecycle")]
    TargetInvocationMismatch,
    #[error("cancel caller does not own the target lifecycle")]
    OwnershipDenied,
    #[error("cancel command was routed to a different lifecycle authority")]
    AuthorityMismatch,
    #[error(transparent)]
    Axon(#[from] AxonError),
}

impl InvocationCancellationRegistry {
    fn lock(&self) -> MutexGuard<'_, RegistryState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn register(
        &self,
        envelope: &DescriptorBoundEnvelope,
        handle: InvocationHandle,
    ) -> Result<String, InvocationCancellationError> {
        let key = invocation_lifecycle_hash(envelope);
        let invocation = envelope.envelope();
        let authority = LifecycleAuthority {
            caller_ura: invocation.caller.ura.clone(),
            authority_ura: invocation.callee.ura.clone(),
        };
        let mut state = self.lock();
        if state.entries.contains_key(&key) {
            return Err(InvocationCancellationError::DuplicateActive(key));
        }
        if !state.reserve_entry_slot() {
            return Err(InvocationCancellationError::CapacityExhausted);
        }
        state
            .entries
            .insert(key.clone(), RegistryEntry::Active { authority, handle });
        Ok(key)
    }

    fn mark_terminal(&self, key: &str, handle: InvocationHandle) {
        let mut state = self.lock();
        let Some(entry) = state.entries.remove(key) else {
            return;
        };
        if entry.handle().invocation_id() != handle.invocation_id() {
            state.entries.insert(key.to_string(), entry);
            return;
        }
        state.entries.insert(
            key.to_string(),
            RegistryEntry::Terminal {
                authority: entry.authority().clone(),
                handle,
            },
        );
        state.retain_terminal_key(key);
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, key: &str) -> bool {
        self.lock().entries.contains_key(key)
    }

    #[cfg(test)]
    pub(crate) fn contains_invocation_id(&self, invocation_id: &str) -> bool {
        self.lock()
            .entries
            .values()
            .any(|entry| entry.handle().invocation_id() == invocation_id)
    }

    /// Execute one admitted cancel command. Completion means the cancellation
    /// signal reached the target Axon handle; it does not claim that the target
    /// has reached a terminal state.
    pub async fn request_cancel(
        &self,
        command: InvocationCancelCommand,
        command_caller_ura: &str,
        command_authority_ura: &str,
    ) -> Result<InvocationCancelResult, InvocationCancellationError> {
        let (handle, already_terminal) = {
            let state = self.lock();
            let entry = state
                .entries
                .get(&command.target_lifecycle_hash)
                .ok_or_else(|| {
                    InvocationCancellationError::TargetNotFound(
                        command.target_lifecycle_hash.clone(),
                    )
                })?;
            let authority = entry.authority();
            if command_authority_ura != authority.authority_ura {
                return Err(InvocationCancellationError::AuthorityMismatch);
            }
            if command_caller_ura != authority.caller_ura
                && command_caller_ura != authority.authority_ura
            {
                return Err(InvocationCancellationError::OwnershipDenied);
            }
            if command
                .target_invocation_id
                .as_deref()
                .is_some_and(|expected| expected != entry.handle().invocation_id())
            {
                return Err(InvocationCancellationError::TargetInvocationMismatch);
            }
            (
                entry.handle().clone(),
                matches!(entry, RegistryEntry::Terminal { .. }),
            )
        };

        if !already_terminal {
            handle.cancel(command.reason).await?;
        }
        Ok(InvocationCancelResult {
            target_lifecycle_hash: command.target_lifecycle_hash,
            target_invocation_id: handle.invocation_id().to_string(),
            accepted: true,
            already_terminal,
        })
    }
}

pub fn invocation_lifecycle_hash(envelope: &DescriptorBoundEnvelope) -> String {
    hex::encode(Sha256::digest(envelope.canonical_bytes()))
}

fn normalize_lifecycle_hash(value: String) -> Result<String, InvocationCancellationError> {
    let value = value.trim().to_ascii_lowercase();
    let decoded =
        hex::decode(&value).map_err(|_| InvocationCancellationError::InvalidLifecycleHash)?;
    if decoded.len() != LIFECYCLE_HASH_BYTES {
        return Err(InvocationCancellationError::InvalidLifecycleHash);
    }
    Ok(value)
}

fn normalize_reason(value: String) -> Result<String, InvocationCancellationError> {
    let value = value.trim();
    if value.len() > MAX_CANCEL_REASON_BYTES {
        return Err(InvocationCancellationError::CancelReasonTooLong);
    }
    Ok(if value.is_empty() {
        DEFAULT_CANCEL_REASON.to_string()
    } else {
        value.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_normalizes_hash_and_reason() {
        let command = InvocationCancelCommand::new("AB".repeat(32), None, "  operator stop  ")
            .expect("valid command");
        assert_eq!(command.target_lifecycle_hash, "ab".repeat(32));
        assert_eq!(command.reason, "operator stop");
    }

    #[test]
    fn terminal_retention_order_is_idempotent() {
        let mut state = RegistryState::default();
        for _ in 0..8 {
            state.retain_terminal_key("a");
        }
        assert_eq!(state.terminal_order.len(), 1);
    }
}

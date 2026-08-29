//! ABI v9 payload-lease ownership and resource accounting.
//!
//! This module changes only the lifetime of immutable payload bytes. It does
//! not own Invocation ordering, receipts, terminal state, or cancellation.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};

use bytes::Bytes;
use rand::{rngs::OsRng, RngCore};
use tokio::sync::OwnedSemaphorePermit;

use super::{ClientSessionBinding, InvocationStreamId, RuntimeBufferLeaseId};

pub(super) const STREAM_V9_MAX_OUTSTANDING_LEASES: usize = 64;
pub(super) const STREAM_V9_MAX_OUTSTANDING_BYTES: usize = 256 * 1024 * 1024;

struct BufferLeaseRegistry {
    state: Mutex<BufferLeaseRegistryState>,
    changed: Condvar,
}

#[derive(Default)]
struct BufferLeaseRegistryState {
    leases: HashMap<RuntimeBufferLeaseId, BufferLeaseEntry>,
    streams: HashMap<InvocationStreamId, StreamLeaseAccounting>,
}

struct BufferLeaseEntry {
    owner: ClientSessionBinding,
    stream_id: InvocationStreamId,
    payload: Bytes,
    _queued_and_leased_byte_budget: Option<OwnedSemaphorePermit>,
    references: u32,
}

struct StreamLeaseAccounting {
    owner: ClientSessionBinding,
    accepting: bool,
    outstanding_leases: usize,
    outstanding_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BufferLeaseAllocationError {
    StreamClosed,
    PayloadTooLarge { bytes: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BufferLeaseAccessError {
    NotFound,
    OwnerMismatch,
    ReferenceOverflow,
}

fn registry() -> &'static BufferLeaseRegistry {
    static REGISTRY: OnceLock<BufferLeaseRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| BufferLeaseRegistry {
        state: Mutex::new(BufferLeaseRegistryState::default()),
        changed: Condvar::new(),
    })
}

fn lock_state(registry: &BufferLeaseRegistry) -> MutexGuard<'_, BufferLeaseRegistryState> {
    registry
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) fn register_stream(owner: ClientSessionBinding, stream_id: InvocationStreamId) {
    let registry = registry();
    let previous = lock_state(registry).streams.insert(
        stream_id,
        StreamLeaseAccounting {
            owner,
            accepting: true,
            outstanding_leases: 0,
            outstanding_bytes: 0,
        },
    );
    debug_assert!(previous.is_none(), "stream ids must not be reused");
}

pub(super) fn close_stream(owner: ClientSessionBinding, stream_id: InvocationStreamId) {
    let registry = registry();
    let mut state = lock_state(registry);
    let remove = match state.streams.get_mut(&stream_id) {
        Some(stream) if stream.owner == owner => {
            stream.accepting = false;
            stream.outstanding_leases == 0
        }
        _ => false,
    };
    if remove {
        state.streams.remove(&stream_id);
    }
    registry.changed.notify_all();
}

fn next_lease_id(
    entries: &HashMap<RuntimeBufferLeaseId, BufferLeaseEntry>,
) -> RuntimeBufferLeaseId {
    loop {
        let candidate = OsRng.next_u64();
        if candidate != 0 && !entries.contains_key(&candidate) {
            return candidate;
        }
    }
}

#[cfg(test)]
pub(super) fn allocate(
    owner: ClientSessionBinding,
    stream_id: InvocationStreamId,
    payload: Bytes,
) -> Result<RuntimeBufferLeaseId, BufferLeaseAllocationError> {
    allocate_with_budget(owner, stream_id, payload, None)
}

pub(super) fn allocate_with_budget(
    owner: ClientSessionBinding,
    stream_id: InvocationStreamId,
    payload: Bytes,
    queued_and_leased_byte_budget: Option<OwnedSemaphorePermit>,
) -> Result<RuntimeBufferLeaseId, BufferLeaseAllocationError> {
    if payload.len() > STREAM_V9_MAX_OUTSTANDING_BYTES {
        return Err(BufferLeaseAllocationError::PayloadTooLarge {
            bytes: payload.len(),
        });
    }

    let registry = registry();
    let mut state = lock_state(registry);
    loop {
        let Some(stream) = state.streams.get(&stream_id) else {
            return Err(BufferLeaseAllocationError::StreamClosed);
        };
        if stream.owner != owner || !stream.accepting {
            return Err(BufferLeaseAllocationError::StreamClosed);
        }
        if payload.is_empty() {
            return Ok(0);
        }
        let byte_capacity_available = stream
            .outstanding_bytes
            .checked_add(payload.len())
            .is_some_and(|bytes| bytes <= STREAM_V9_MAX_OUTSTANDING_BYTES);
        if stream.outstanding_leases < STREAM_V9_MAX_OUTSTANDING_LEASES && byte_capacity_available {
            break;
        }
        state = registry
            .changed
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }

    let lease_id = next_lease_id(&state.leases);
    state.leases.insert(
        lease_id,
        BufferLeaseEntry {
            owner,
            stream_id,
            payload: payload.clone(),
            _queued_and_leased_byte_budget: queued_and_leased_byte_budget,
            references: 1,
        },
    );
    let stream = state
        .streams
        .get_mut(&stream_id)
        .expect("lease stream remains registered while registry lock is held");
    stream.outstanding_leases += 1;
    stream.outstanding_bytes += payload.len();
    Ok(lease_id)
}

pub(super) fn retain(
    owner: ClientSessionBinding,
    lease_id: RuntimeBufferLeaseId,
) -> Result<(), BufferLeaseAccessError> {
    let registry = registry();
    let mut state = lock_state(registry);
    let Some(entry) = state.leases.get_mut(&lease_id) else {
        return Err(BufferLeaseAccessError::NotFound);
    };
    if entry.owner != owner {
        return Err(BufferLeaseAccessError::OwnerMismatch);
    }
    entry.references = entry
        .references
        .checked_add(1)
        .ok_or(BufferLeaseAccessError::ReferenceOverflow)?;
    Ok(())
}

pub(super) fn release(
    owner: ClientSessionBinding,
    lease_id: RuntimeBufferLeaseId,
) -> Result<(), BufferLeaseAccessError> {
    let registry = registry();
    let mut state = lock_state(registry);
    let Some(entry) = state.leases.get_mut(&lease_id) else {
        return Err(BufferLeaseAccessError::NotFound);
    };
    if entry.owner != owner {
        return Err(BufferLeaseAccessError::OwnerMismatch);
    }
    if entry.references > 1 {
        entry.references -= 1;
        return Ok(());
    }

    let entry = state
        .leases
        .remove(&lease_id)
        .expect("lease exists while registry lock is held");
    let remove_stream = match state.streams.get_mut(&entry.stream_id) {
        Some(stream) => {
            stream.outstanding_leases = stream.outstanding_leases.saturating_sub(1);
            stream.outstanding_bytes = stream.outstanding_bytes.saturating_sub(entry.payload.len());
            !stream.accepting && stream.outstanding_leases == 0
        }
        None => false,
    };
    if remove_stream {
        state.streams.remove(&entry.stream_id);
    }
    registry.changed.notify_all();
    Ok(())
}

pub(super) fn purge_owner(owner: ClientSessionBinding) {
    let registry = registry();
    let mut state = lock_state(registry);
    state.leases.retain(|_, entry| entry.owner != owner);
    state.streams.retain(|_, stream| stream.owner != owner);
    registry.changed.notify_all();
}

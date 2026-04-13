// EasyNet CLI — DendriteBridge Connection Pool
// =============================================
//
// File: src/shared/bridge_pool.rs
// Description: High-performance, lock-free connection pool for DendriteBridge instances.
//
// Problem:
//   DendriteBridge is !Send/!Sync (FFI handle). Each thread in a parallel
//   phase needs its own bridge. Creating a new gRPC connection per step pays
//   TCP+gRPC handshake cost every time.
//
// Solution (gen-2):
//   Lock-free SegQueue from crossbeam replaces Mutex<Vec>. Bridges are moved
//   in/out atomically without contention. Pool size adapts to available CPU
//   cores. AtomicUsize tracks current size for bounded growth.
//
// Thread Safety:
//   The pool itself is Send+Sync (crossbeam SegQueue + atomics). Individual
//   DendriteBridge instances are moved into threads, never shared.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_queue::SegQueue;
use easynet_axon::dendrite_bridge::DendriteBridge;

/// Compute adaptive pool size based on available CPU cores.
///
/// Returns `max(num_cpus, 4)` capped at 64. This ensures the pool is large
/// enough for typical parallel phases without over-allocating on large machines.
pub fn adaptive_pool_size() -> usize {
    num_cpus::get().max(4).min(64)
}

/// A lock-free pool of pre-connected DendriteBridge instances.
///
/// Uses crossbeam's `SegQueue` for wait-free push/pop operations.
/// Bridges are checked out (moved to caller), used, and returned.
/// The pool grows on demand if all bridges are checked out.
pub struct BridgePool {
    endpoint: String,
    timeout_ms: u64,
    queue: SegQueue<DendriteBridge>,
    /// Current number of bridges in the queue (approximate, used for bounded return).
    size: AtomicUsize,
    /// Maximum pool size — bridges beyond this are dropped on return.
    max_size: usize,
}

// Safety: BridgePool is Send+Sync because:
// - SegQueue<DendriteBridge> is internally synchronized (lock-free CAS operations).
// - Bridges are moved into/out of the queue, never shared across threads.
// - Each checked-out bridge is used by exactly one thread at a time.
// - DendriteBridge is !Send only because the FFI handle *might* not be
//   thread-safe, but we never use a bridge from two threads simultaneously.
//   The pool transfers ownership: checkout() → use → drop(BridgeGuard) → checkin().
unsafe impl Send for BridgePool {}
unsafe impl Sync for BridgePool {}

impl BridgePool {
    /// Create a new pool and eagerly connect `initial_size` bridges.
    ///
    /// Bridges that fail to connect during init are silently skipped;
    /// they will be created on demand when checked out.
    pub fn new(endpoint: &str, timeout_ms: u64, initial_size: usize) -> Self {
        let queue = SegQueue::new();
        let mut connected = 0usize;
        for _ in 0..initial_size {
            if let Ok(br) = DendriteBridge::connect(endpoint, timeout_ms) {
                queue.push(br);
                connected += 1;
            }
        }
        Self {
            endpoint: endpoint.to_string(),
            timeout_ms,
            queue,
            size: AtomicUsize::new(connected),
            // 2× initial for headroom; at least 16 to absorb burst phases.
            max_size: (initial_size * 2).max(16),
        }
    }

    /// Create a pool with adaptive sizing based on available CPU cores.
    pub fn with_adaptive_size(endpoint: &str, timeout_ms: u64) -> Self {
        Self::new(endpoint, timeout_ms, adaptive_pool_size())
    }

    /// Check out a bridge from the pool.
    ///
    /// Returns an existing bridge if available (lock-free pop),
    /// otherwise creates a new one on demand.
    pub fn checkout(&self) -> Result<BridgeGuard<'_>, String> {
        let bridge = match self.queue.pop() {
            Some(br) => {
                self.size.fetch_sub(1, Ordering::Relaxed);
                br
            }
            None => DendriteBridge::connect(&self.endpoint, self.timeout_ms)
                .map_err(|e| format!("bridge connect: {e}"))?,
        };
        Ok(BridgeGuard {
            bridge: Some(bridge),
            pool: self,
        })
    }

    /// Return a bridge to the pool (called by BridgeGuard::drop).
    fn checkin(&self, bridge: DendriteBridge) {
        let current = self.size.load(Ordering::Relaxed);
        if current < self.max_size {
            self.queue.push(bridge);
            self.size.fetch_add(1, Ordering::Relaxed);
        }
        // else: drop the bridge (pool at capacity)
    }

    /// Endpoint accessor.
    #[allow(dead_code)]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Timeout accessor.
    #[allow(dead_code)]
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// Current approximate number of idle bridges in the pool.
    #[allow(dead_code)]
    pub fn idle_count(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }

    /// Maximum pool size.
    #[allow(dead_code)]
    pub fn max_size(&self) -> usize {
        self.max_size
    }
}

/// RAII guard that returns the bridge to the pool on drop.
///
/// This type is `!Send` because `DendriteBridge` is `!Send`, which is
/// correct — the guard must be used and dropped on the same thread.
pub struct BridgeGuard<'a> {
    bridge: Option<DendriteBridge>,
    pool: &'a BridgePool,
}

impl<'a> BridgeGuard<'a> {
    /// Access the underlying bridge.
    pub fn bridge(&self) -> &DendriteBridge {
        self.bridge.as_ref().expect("bridge taken after drop")
    }
}

impl Drop for BridgeGuard<'_> {
    fn drop(&mut self) {
        if let Some(br) = self.bridge.take() {
            self.pool.checkin(br);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_pool_size_is_reasonable() {
        let size = adaptive_pool_size();
        assert!(size >= 4, "adaptive pool size should be at least 4, got {size}");
        assert!(size <= 64, "adaptive pool size should be at most 64, got {size}");
    }

    #[test]
    fn pool_new_with_zero_initial_size() {
        let pool = BridgePool::new("localhost:50051", 5000, 0);
        assert_eq!(pool.max_size, 16); // max(0*2, 16)
        assert_eq!(pool.idle_count(), 0);
    }

    #[test]
    fn pool_max_size_at_least_16() {
        let pool = BridgePool::new("localhost:50051", 5000, 4);
        assert_eq!(pool.max_size, 16); // max(4*2=8, 16)
    }

    #[test]
    fn pool_max_size_scales_with_initial() {
        let pool = BridgePool::new("localhost:50051", 5000, 32);
        assert_eq!(pool.max_size, 64); // 32*2 = 64 > 16
    }

    #[test]
    fn adaptive_pool_uses_cpu_count() {
        let pool = BridgePool::with_adaptive_size("localhost:50051", 5000);
        let expected_max = (adaptive_pool_size() * 2).max(16);
        assert_eq!(pool.max_size, expected_max);
    }
}

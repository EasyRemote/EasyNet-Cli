// EasyNet CLI — remote desktop session lease
// ==========================================
//
// File: src/plugins/builtin/remote_desktop/session_lease.rs
// Description: Timestamp and lease-deadline model for remote desktop sessions.

/// Timestamp and lease deadline owned by a remote desktop session.
///
/// Invariant 1: `created_at_ms` never changes after construction.
/// Invariant 2: `updated_at_ms` moves only through `touch` and `refresh`.
/// Invariant 3: `expires_at_ms` is computed by saturating addition so large
/// lease TTLs cannot panic or wrap the session deadline.
#[derive(Debug, Clone)]
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopLease {
    created_at_ms: u64,
    updated_at_ms: u64,
    expires_at_ms: u64,
}

impl RemoteDesktopLease {
    /// Build the initial session lease from a single creation timestamp.
    pub(in crate::plugins::builtin::remote_desktop) fn new(now_ms: u64, ttl_ms: u64) -> Self {
        Self {
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(ttl_ms),
        }
    }

    /// Creation timestamp in Unix milliseconds.
    pub(in crate::plugins::builtin::remote_desktop) fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    /// Last mutation timestamp in Unix milliseconds.
    pub(in crate::plugins::builtin::remote_desktop) fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }

    /// Current lease deadline in Unix milliseconds.
    pub(in crate::plugins::builtin::remote_desktop) fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    /// Whether the lease has elapsed at `now_ms`.
    pub(in crate::plugins::builtin::remote_desktop) fn is_expired_at(&self, now_ms: u64) -> bool {
        now_ms > self.expires_at_ms
    }

    /// Mark a non-lease session mutation at `now_ms`.
    pub(in crate::plugins::builtin::remote_desktop) fn touch(&mut self, now_ms: u64) {
        self.updated_at_ms = now_ms;
    }

    /// Refresh the lease and return the new expiry timestamp.
    pub(in crate::plugins::builtin::remote_desktop) fn refresh(
        &mut self,
        now_ms: u64,
        ttl_ms: u64,
    ) -> u64 {
        self.updated_at_ms = now_ms;
        self.expires_at_ms = now_ms.saturating_add(ttl_ms);
        self.expires_at_ms
    }

    /// Override the deadline in tests without exposing production mutation.
    #[cfg(test)]
    pub(in crate::plugins::builtin::remote_desktop) fn set_expires_at_for_test(
        &mut self,
        expires_at_ms: u64,
    ) {
        self.expires_at_ms = expires_at_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_desktop_lease_refresh_uses_saturating_deadline() {
        let mut lease = RemoteDesktopLease::new(10, 5);

        let expires_at = lease.refresh(u64::MAX - 1, 10);

        assert_eq!(lease.created_at_ms(), 10);
        assert_eq!(lease.updated_at_ms(), u64::MAX - 1);
        assert_eq!(expires_at, u64::MAX);
        assert_eq!(lease.expires_at_ms(), u64::MAX);
    }

    #[test]
    fn remote_desktop_lease_expiry_is_strictly_after_deadline() {
        let lease = RemoteDesktopLease::new(100, 50);

        assert!(!lease.is_expired_at(150));
        assert!(lease.is_expired_at(151));
    }
}

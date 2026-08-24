// EasyNet CLI — remote desktop transport manager
// ===============================================
//
// File: plugins/remote-desktop/src/transport/manager.rs
// Description: Epoch-scoped endpoint and owned media-task lifecycle.
//
// Protocol Responsibility:
// - None. Axon does not own product media endpoints.
//
// Implementation Approach:
// - Allocate monotonic epochs, expose cloneable endpoint access, and retain the
//   only stop/completion handles in a managed endpoint.
//
// Usage Contract:
// - Callbacks and candidate application must compare the endpoint epoch before
//   mutating session state. Replacement retires and settles the old generation.
//
// Architectural Position:
// - Remote-desktop plugin transport-resource owner.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::runtime::Handle;
use tokio::sync::watch;
use webrtc::peer_connection::PeerConnection;

use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;

#[derive(Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct DirectWebRtcEndpoint {
    pub(in crate::daemon::plugins::remote_desktop) epoch: TransportEpoch,
    pub(in crate::daemon::plugins::remote_desktop) peer_connection: Arc<dyn PeerConnection>,
}

struct ManagedDirectWebRtcEndpoint {
    access: DirectWebRtcEndpoint,
    stop_tx: watch::Sender<bool>,
    completion: Option<thread::JoinHandle<()>>,
}

impl ManagedDirectWebRtcEndpoint {
    fn retire(mut self) {
        let _ = self.stop_tx.send(true);
        if let Some(completion) = self.completion.take() {
            let _ = thread::Builder::new()
                .name("easynet-rd-endpoint-reaper".into())
                .spawn(move || {
                    let _ = completion.join();
                });
        }
    }

    fn stop_and_join(mut self) {
        let _ = self.stop_tx.send(true);
        if let Some(completion) = self.completion.take() {
            let _ = completion.join();
        }
    }
}

pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopTransportManager {
    endpoints: Mutex<HashMap<String, ManagedDirectWebRtcEndpoint>>,
    next_epoch: AtomicU64,
    runtime: Mutex<Option<tokio::runtime::Runtime>>,
}

impl RemoteDesktopTransportManager {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self {
            endpoints: Mutex::new(HashMap::new()),
            // Epochs are public stale-callback fences, so a daemon restart
            // must not reset the namespace to one. Recovery snapshots tighten
            // this seed further through `observe_prior_epoch`.
            next_epoch: AtomicU64::new(process_epoch_seed()),
            runtime: Mutex::new(None),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn allocate_epoch(&self) -> TransportEpoch {
        let value = self.next_epoch.fetch_add(1, Ordering::AcqRel);
        assert_ne!(
            value,
            u64::MAX,
            "RemoteApp transport epoch namespace exhausted"
        );
        TransportEpoch::new(value)
    }

    /// Move the process-local allocator past an epoch persisted by an earlier
    /// daemon process. This is idempotent and safe to call for every recovered
    /// session before the plugin accepts new offers.
    pub(in crate::daemon::plugins::remote_desktop) fn observe_prior_epoch(&self, epoch: u64) {
        let next = epoch
            .checked_add(1)
            .expect("persisted RemoteApp transport epoch namespace exhausted");
        self.next_epoch.fetch_max(next, Ordering::AcqRel);
    }

    pub(in crate::daemon::plugins::remote_desktop) fn activate_endpoint(
        &self,
        session_id: String,
        endpoint: DirectWebRtcEndpoint,
        stop_tx: watch::Sender<bool>,
        completion: thread::JoinHandle<()>,
    ) {
        let old = self.endpoints().insert(
            session_id,
            ManagedDirectWebRtcEndpoint {
                access: endpoint,
                stop_tx,
                completion: Some(completion),
            },
        );
        if let Some(old) = old {
            old.retire();
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn endpoint(
        &self,
        session_id: &str,
    ) -> Option<DirectWebRtcEndpoint> {
        self.endpoints()
            .get(session_id)
            .map(|managed| managed.access.clone())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn stop_endpoint(&self, session_id: &str) {
        if let Some(endpoint) = self.endpoints().remove(session_id) {
            endpoint.retire();
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn stop_endpoint_if_epoch(
        &self,
        session_id: &str,
        epoch: TransportEpoch,
    ) -> bool {
        let endpoint = {
            let mut endpoints = self.endpoints();
            if endpoints
                .get(session_id)
                .is_none_or(|endpoint| endpoint.access.epoch != epoch)
            {
                return false;
            }
            endpoints.remove(session_id)
        };
        if let Some(endpoint) = endpoint {
            endpoint.retire();
            return true;
        }
        false
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn clear_endpoints(&self) {
        let endpoints = std::mem::take(&mut *self.endpoints());
        for (_, endpoint) in endpoints {
            endpoint.retire();
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn block_on<F: Future>(
        &self,
        future: F,
    ) -> anyhow::Result<F::Output> {
        Ok(self.runtime_handle()?.block_on(future))
    }

    fn endpoints(&self) -> MutexGuard<'_, HashMap<String, ManagedDirectWebRtcEndpoint>> {
        match self.endpoints.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn runtime(&self) -> MutexGuard<'_, Option<tokio::runtime::Runtime>> {
        match self.runtime.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn runtime_handle(&self) -> anyhow::Result<Handle> {
        let mut runtime = self.runtime();
        if runtime.is_none() {
            let built = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("easynet-webrtc-runtime")
                .enable_all()
                .build()
                .map_err(|err| anyhow::anyhow!("build remote desktop WebRTC runtime: {err}"))?;
            *runtime = Some(built);
        }
        runtime
            .as_ref()
            .map(|runtime| runtime.handle().clone())
            .ok_or_else(|| {
                anyhow::anyhow!("remote desktop WebRTC runtime unavailable after initialization")
            })
    }
}

fn process_epoch_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros().min(u64::MAX as u128 - 1) as u64)
        .unwrap_or(1)
        .max(1)
}

impl Drop for RemoteDesktopTransportManager {
    fn drop(&mut self) {
        let endpoints = match self.endpoints.get_mut() {
            Ok(endpoints) => std::mem::take(endpoints),
            Err(poisoned) => std::mem::take(poisoned.into_inner()),
        };
        for (_, endpoint) in endpoints {
            endpoint.stop_and_join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocator_is_monotonic_and_advances_past_recovered_epochs() {
        let manager = RemoteDesktopTransportManager::new();
        let first = manager.allocate_epoch();
        let second = manager.allocate_epoch();
        assert!(second > first);

        let recovered = second.value().saturating_add(10_000);
        manager.observe_prior_epoch(recovered);
        let resumed = manager.allocate_epoch();
        assert!(resumed.value() > recovered);
    }
}

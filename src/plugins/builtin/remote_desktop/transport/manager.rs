// EasyNet CLI — remote desktop transport manager
// ===============================================
//
// File: src/plugins/builtin/remote_desktop/transport/manager.rs
// Description: Runtime and endpoint-handle ownership for remote desktop media.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use tokio::runtime::Handle;
use tokio::sync::watch;
use webrtc::peer_connection::PeerConnection;

/// Handle for one direct WebRTC endpoint owned by the plugin.
///
/// Invariant 1: `stop_tx` is the only cooperative shutdown signal for the
/// endpoint's media loop.
/// Invariant 2: `peer_connection` remains reachable while ICE candidates can
/// be trickled into the endpoint.
#[derive(Clone)]
pub(in crate::plugins::builtin::remote_desktop) struct DirectWebRtcEndpoint {
    pub(in crate::plugins::builtin::remote_desktop) stop_tx: watch::Sender<bool>,
    pub(in crate::plugins::builtin::remote_desktop) peer_connection: Arc<dyn PeerConnection>,
}

/// Owns long-lived transport handles and the async runtime used by the plugin.
///
/// Invariant 1: direct WebRTC endpoint handles are keyed by session id and are
/// removed before a replacement endpoint is inserted.
/// Invariant 2: callers clone a Tokio [`Handle`] before running or spawning
/// work, so long-lived media futures never hold the manager mutex and cannot
/// starve ICE trickle, session setup, or teardown operations.
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopTransportManager {
    endpoints: Mutex<HashMap<String, DirectWebRtcEndpoint>>,
    runtime: Mutex<Option<tokio::runtime::Runtime>>,
}

impl RemoteDesktopTransportManager {
    /// Construct WITHOUT building the Tokio runtime.
    ///
    /// Invariant (load-bearing): registration must not start the media
    /// engine. Reflection/snapshot surfaces (`published_abilities()`,
    /// MCP reflective refresh, descriptor generation) rebuild the full
    /// plugin registry — each rebuild constructs a manager whose
    /// registered closures keep it alive. When `new()` eagerly built a
    /// 2-worker runtime, every such rebuild leaked one runtime: 120
    /// `easynet-webrtc-runtime` threads + 183 kqueues exhausted the
    /// daemon's RLIMIT_NOFILE (os error 24) and took down sockets,
    /// spawns, and manifest reads fleet-wide (2026-06-10). The runtime
    /// is now built on first real media use (`runtime_handle`).
    pub(in crate::plugins::builtin::remote_desktop) fn new() -> Self {
        Self {
            endpoints: Mutex::new(HashMap::new()),
            runtime: Mutex::new(None),
        }
    }

    pub(in crate::plugins::builtin::remote_desktop) fn replace_endpoint(
        &self,
        session_id: String,
        endpoint: DirectWebRtcEndpoint,
    ) -> Option<DirectWebRtcEndpoint> {
        self.endpoints().insert(session_id, endpoint)
    }

    pub(in crate::plugins::builtin::remote_desktop) fn endpoint(
        &self,
        session_id: &str,
    ) -> Option<DirectWebRtcEndpoint> {
        self.endpoints().get(session_id).cloned()
    }

    fn remove_endpoint(&self, session_id: &str) -> Option<DirectWebRtcEndpoint> {
        self.endpoints().remove(session_id)
    }

    /// Stop and remove one direct WebRTC endpoint if it is currently live.
    ///
    /// This owns endpoint-handle teardown at the transport boundary. Session
    /// state transitions happen in the session module; callers use this only
    /// to release live media transport resources.
    pub(in crate::plugins::builtin::remote_desktop) fn stop_endpoint(&self, session_id: &str) {
        if let Some(endpoint) = self.remove_endpoint(session_id) {
            let _ = endpoint.stop_tx.send(true);
        }
    }

    #[cfg(test)]
    pub(in crate::plugins::builtin::remote_desktop) fn clear_endpoints(&self) {
        self.endpoints().clear();
    }

    pub(in crate::plugins::builtin::remote_desktop) fn block_on<F: Future>(
        &self,
        future: F,
    ) -> F::Output {
        self.runtime_handle().block_on(future)
    }

    pub(in crate::plugins::builtin::remote_desktop) fn spawn<F>(
        &self,
        future: F,
    ) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.runtime_handle().spawn(future)
    }

    fn endpoints(&self) -> MutexGuard<'_, HashMap<String, DirectWebRtcEndpoint>> {
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

    fn runtime_handle(&self) -> Handle {
        self.runtime()
            .get_or_insert_with(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .thread_name("easynet-webrtc-runtime")
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("create EasyNet WebRTC runtime")
            })
            .handle()
            .clone()
    }
}

impl Drop for RemoteDesktopTransportManager {
    fn drop(&mut self) {
        let runtime = match self.runtime.get_mut() {
            Ok(slot) => slot.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        let Some(runtime) = runtime else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            let _ = thread::Builder::new()
                .name("easynet-webrtc-runtime-drop".into())
                .spawn(move || drop(runtime))
                .and_then(|handle| {
                    handle
                        .join()
                        .map_err(|_| std::io::Error::other("WebRTC runtime drop panicked"))
                });
        } else {
            drop(runtime);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc as std_mpsc;
    use std::time::Duration;

    #[test]
    fn new_does_not_build_the_runtime() {
        // Regression (2026-06-10 fd exhaustion): reflection paths
        // construct managers they never use for media; construction
        // must stay thread/fd-free.
        let manager = RemoteDesktopTransportManager::new();
        assert!(
            manager.runtime().is_none(),
            "runtime must be lazy — built on first media use, not at registration"
        );
    }

    #[test]
    fn long_block_on_does_not_serialize_later_runtime_calls() {
        let manager = Arc::new(RemoteDesktopTransportManager::new());
        let (entered_tx, entered_rx) = std_mpsc::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();

        let first_manager = Arc::clone(&manager);
        let first = thread::spawn(move || {
            first_manager.block_on(async move {
                entered_tx.send(()).expect("test receiver alive");
                let _ = release_rx.await;
            });
        });
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first block_on future entered");

        let (done_tx, done_rx) = std_mpsc::channel();
        let second_manager = Arc::clone(&manager);
        let second = thread::spawn(move || {
            let value = second_manager.block_on(async { 42_u8 });
            done_tx.send(value).expect("test receiver alive");
        });

        assert_eq!(
            done_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("second block_on must not wait for first long future"),
            42
        );

        let _ = release_tx.send(());
        first.join().expect("first runtime caller joins");
        second.join().expect("second runtime caller joins");
    }
}

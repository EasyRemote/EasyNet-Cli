//! EasyNet CLI — remote desktop lifecycle worker
//! =================================================
//!
//! File: plugins/remote-desktop/src/lifecycle_worker.rs
//! Description: Shared ownership primitive for restartable plugin worker threads.
//!
//! Protocol Responsibility:
//! - None. This module owns daemon-local thread lifecycle only.
//!
//! Implementation Approach:
//! - Pair one command sender with one join handle and replace them atomically.
//! - Join stopped workers from external threads and detach when destruction is
//!   initiated by the worker itself, which cannot legally join its own thread.
//!
//! Usage Contract:
//! - Owners provide a typed shutdown command and call `shutdown` from `Drop`.
//! - Restart is fallible and must occur outside the worker thread.
//!
//! Architectural Position:
//! - Shared remote-desktop plugin lifecycle infrastructure.

use std::io;
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};

pub(in crate::daemon::plugins::remote_desktop) struct LifecycleWorker<C> {
    tx: Option<Sender<C>>,
    join: Option<JoinHandle<()>>,
}

impl<C> LifecycleWorker<C> {
    pub(in crate::daemon::plugins::remote_desktop) const fn new() -> Self {
        Self {
            tx: None,
            join: None,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn sender(&self) -> Option<Sender<C>> {
        self.tx.clone()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn start(
        &mut self,
        spawn: impl FnOnce() -> io::Result<(Sender<C>, JoinHandle<()>)>,
    ) -> io::Result<Sender<C>> {
        self.join_for_restart()?;
        let (tx, join) = spawn()?;
        self.tx = Some(tx.clone());
        self.join = Some(join);
        Ok(tx)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn shutdown(&mut self, command: C) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(command);
        }
        let Some(join) = self.join.take() else {
            return;
        };
        if join.thread().id() == thread::current().id() {
            drop(join);
            return;
        }
        let _ = join.join();
    }

    fn join_for_restart(&mut self) -> io::Result<()> {
        self.tx.take();
        let Some(join) = self.join.take() else {
            return Ok(());
        };
        if join.thread().id() == thread::current().id() {
            drop(join);
            return Err(io::Error::other(
                "lifecycle worker cannot restart itself while it is running",
            ));
        }
        join.join()
            .map_err(|_| io::Error::other("lifecycle worker terminated with panic"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::LifecycleWorker;

    #[test]
    fn shutdown_from_worker_detaches_instead_of_self_joining() {
        let lifecycle = Arc::new(Mutex::new(LifecycleWorker::<()>::new()));
        let worker_lifecycle = Arc::clone(&lifecycle);
        let (completed_tx, completed_rx) = mpsc::channel();

        lifecycle
            .lock()
            .expect("lifecycle lock")
            .start(move || {
                let (command_tx, command_rx) = mpsc::channel();
                let join = std::thread::spawn(move || {
                    command_rx.recv().expect("shutdown trigger");
                    worker_lifecycle
                        .lock()
                        .expect("worker lifecycle lock")
                        .shutdown(());
                    completed_tx.send(()).expect("completion signal");
                });
                Ok((command_tx, join))
            })
            .expect("worker starts")
            .send(())
            .expect("shutdown trigger sends");

        completed_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("self-owned shutdown completes without a self-join panic");
    }
}

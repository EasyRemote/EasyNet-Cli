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
//! - A previously panicked generation is observable in logs but does not
//!   prevent the owner from installing a replacement generation.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviousWorkerExit {
    NotRunning,
    Completed,
    Panicked,
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
        let previous_exit = self.join_for_restart()?;
        if previous_exit == PreviousWorkerExit::Panicked {
            eprintln!("[remote-desktop] replacing a lifecycle worker that terminated with panic");
        }
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

    fn join_for_restart(&mut self) -> io::Result<PreviousWorkerExit> {
        self.tx.take();
        let Some(join) = self.join.take() else {
            return Ok(PreviousWorkerExit::NotRunning);
        };
        if join.thread().id() == thread::current().id() {
            drop(join);
            return Err(io::Error::other(
                "lifecycle worker cannot restart itself while it is running",
            ));
        }
        Ok(match join.join() {
            Ok(()) => PreviousWorkerExit::Completed,
            Err(_) => PreviousWorkerExit::Panicked,
        })
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

    #[test]
    fn start_replaces_a_worker_that_terminated_with_panic() {
        let mut lifecycle = LifecycleWorker::<()>::new();
        let first = lifecycle
            .start(|| {
                let (command_tx, command_rx) = mpsc::channel();
                let join = std::thread::spawn(move || {
                    command_rx.recv().expect("panic trigger");
                    panic!("injected worker failure");
                });
                Ok((command_tx, join))
            })
            .expect("first worker starts");
        first.send(()).expect("panic trigger sends");
        drop(first);

        let (restarted_tx, restarted_rx) = mpsc::channel();
        let second = lifecycle
            .start(|| {
                let (command_tx, command_rx) = mpsc::channel();
                let join = std::thread::spawn(move || {
                    command_rx.recv().expect("replacement trigger");
                    restarted_tx.send(()).expect("replacement reports work");
                });
                Ok((command_tx, join))
            })
            .expect("panicked worker is replaceable");
        second.send(()).expect("replacement trigger sends");
        restarted_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("replacement worker runs");
    }
}

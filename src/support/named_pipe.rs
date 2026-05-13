// EasyNet CLI — Windows named-pipe helpers
// ========================================
//
// File: src/support/named_pipe.rs
// Description: Deterministic per-user pipe naming plus the minimal
//              bind/accept/connect helpers needed by the local
//              control-plane and daemon-sidecar transports on
//              Windows.
//
// Scope
// -----
// This module is intentionally narrow:
//   - `scoped_pipe_name(label)` mints a stable pipe name from the
//     current EasyNet state directory, so two different desktop
//     users do not collide on `\\.\pipe\...`.
//   - `PipeListener` owns the "always keep one instance available"
//     dance required by Windows named pipes.
//   - `connect_with_retry` hides the `ERROR_PIPE_BUSY` / start-up
//     race retry loop so callers do not each grow their own copy.
//
// It does NOT try to virtualize Unix-vs-Windows transport behind one
// trait. Callers still choose their platform-specific branch at the
// module boundary; this file only keeps the Windows path honest and
// reusable.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#[cfg(windows)]
use std::io;
#[cfg(windows)]
use std::time::{Duration, Instant};

#[cfg(windows)]
use sha2::{Digest, Sha256};
#[cfg(windows)]
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};

#[cfg(windows)]
const ERROR_PIPE_BUSY_OS_CODE: i32 = 231;

/// Derive a deterministic pipe name scoped to the current EasyNet
/// state directory. The state-dir path already carries the user
/// boundary (`%USERPROFILE%\\.easynet`), so hashing it gives us a
/// stable "same user ⇒ same pipe, different user ⇒ different pipe"
/// name without leaking the full home path into `control.json`.
#[cfg(windows)]
pub fn scoped_pipe_name(label: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        crate::persistence::config::state_dir()
            .to_string_lossy()
            .as_bytes(),
    );
    hasher.update(b":");
    hasher.update(label.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!(r"\\.\pipe\easynet-{label}-{}", &digest[..16])
}

/// Server-side helper that always keeps one unconnected instance
/// available while handing the connected instance off to the caller.
///
/// This mirrors the pattern documented in Tokio's named-pipe docs:
/// connect the current instance, create the next one immediately,
/// then return the connected instance.
#[cfg(windows)]
#[derive(Debug)]
pub struct PipeListener {
    name: String,
    pending: NamedPipeServer,
}

#[cfg(windows)]
impl PipeListener {
    pub fn bind(name: impl Into<String>) -> io::Result<Self> {
        let name = name.into();
        let pending = create_server_instance(&name, true)?;
        Ok(Self { name, pending })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn accept(&mut self) -> io::Result<NamedPipeServer> {
        self.pending.connect().await?;
        let next = create_server_instance(&self.name, false)?;
        Ok(std::mem::replace(&mut self.pending, next))
    }
}

#[cfg(windows)]
fn create_server_instance(name: &str, first_instance: bool) -> io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    if first_instance {
        options.first_pipe_instance(true);
    }
    options.create(name)
}

/// Dial a named pipe, retrying through the two expected startup races:
/// the server has not created the first instance yet (`NotFound`) or
/// all existing instances are currently busy (`ERROR_PIPE_BUSY`).
#[cfg(windows)]
pub async fn connect_with_retry(name: &str, timeout: Duration) -> io::Result<NamedPipeClient> {
    let deadline = Instant::now() + timeout;
    loop {
        match ClientOptions::new().open(name) {
            Ok(client) => return Ok(client),
            Err(err)
                if Instant::now() < deadline
                    && (err.kind() == io::ErrorKind::NotFound
                        || err.raw_os_error() == Some(ERROR_PIPE_BUSY_OS_CODE)) =>
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(err) => return Err(err),
        }
    }
}

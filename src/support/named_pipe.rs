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
//     dance required by Windows named pipes, and rejects clients whose
//     OS token SID does not match the daemon user's SID.
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
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use std::time::{Duration, Instant};

#[cfg(windows)]
use sha2::{Digest, Sha256};
#[cfg(windows)]
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::Security::{
    CopySid, EqualSid, GetLengthSid, GetTokenInformation, RevertToSelf, TokenUser, PSID,
    TOKEN_QUERY, TOKEN_USER,
};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::ImpersonateNamedPipeClient;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
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
    peer_gate: WindowsPeerGate,
}

#[cfg(windows)]
impl PipeListener {
    pub fn bind(name: impl Into<String>) -> io::Result<Self> {
        let name = name.into();
        let peer_gate = WindowsPeerGate::for_current_process()?;
        let pending = create_server_instance(&name, true)?;
        Ok(Self {
            name,
            pending,
            peer_gate,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn accept(&mut self) -> io::Result<NamedPipeServer> {
        self.pending.connect().await?;
        let next = create_server_instance(&self.name, false)?;
        let stream = std::mem::replace(&mut self.pending, next);
        self.peer_gate.authorize_stream(&stream)?;
        Ok(stream)
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

#[cfg(windows)]
#[derive(Debug)]
struct WindowsPeerGate {
    daemon_user_sid: WindowsSid,
}

#[cfg(windows)]
impl WindowsPeerGate {
    fn for_current_process() -> io::Result<Self> {
        let token = open_current_process_token()?;
        Ok(Self {
            daemon_user_sid: token_user_sid(token.raw())?,
        })
    }

    fn authorize_stream(&self, stream: &NamedPipeServer) -> io::Result<()> {
        let peer_sid = peer_sid_from_named_pipe(stream)?;
        if self.daemon_user_sid.equals(&peer_sid) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "named-pipe peer user SID does not match daemon user SID",
            ))
        }
    }
}

#[cfg(windows)]
#[derive(Clone)]
struct WindowsSid(Vec<u8>);

#[cfg(windows)]
impl std::fmt::Debug for WindowsSid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("WindowsSid")
            .field(&format_args!("{} bytes", self.0.len()))
            .finish()
    }
}

#[cfg(windows)]
impl WindowsSid {
    fn from_psid(sid: PSID) -> io::Result<Self> {
        if sid.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows token user SID is null",
            ));
        }
        let len = unsafe { GetLengthSid(sid) };
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut bytes = vec![0u8; len as usize];
        let ok = unsafe { CopySid(len, bytes.as_mut_ptr().cast(), sid) };
        if ok == FALSE {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(bytes))
    }

    fn as_psid(&self) -> PSID {
        self.0.as_ptr() as PSID
    }

    fn equals(&self, other: &Self) -> bool {
        unsafe { EqualSid(self.as_psid(), other.as_psid()) != FALSE }
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct TokenHandle(HANDLE);

#[cfg(windows)]
impl TokenHandle {
    fn raw(&self) -> HANDLE {
        self.0
    }
}

#[cfg(windows)]
impl Drop for TokenHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn open_current_process_token() -> io::Result<TokenHandle> {
    let mut token = std::ptr::null_mut();
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(TokenHandle(token))
}

#[cfg(windows)]
fn open_current_thread_token() -> io::Result<TokenHandle> {
    let mut token = std::ptr::null_mut();
    let ok = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, FALSE, &mut token) };
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(TokenHandle(token))
}

#[cfg(windows)]
fn token_user_sid(token: HANDLE) -> io::Result<WindowsSid> {
    let mut needed = 0u32;
    unsafe {
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut buffer = vec![0u8; needed as usize];
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }

    let token_user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    WindowsSid::from_psid(token_user.User.Sid)
}

#[cfg(windows)]
fn peer_sid_from_named_pipe(stream: &NamedPipeServer) -> io::Result<WindowsSid> {
    let handle = stream.as_raw_handle() as HANDLE;
    let ok = unsafe { ImpersonateNamedPipeClient(handle) };
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }

    let token_result = open_current_thread_token();
    let sid_result = token_result.and_then(|token| token_user_sid(token.raw()));
    let revert_ok = unsafe { RevertToSelf() };
    if revert_ok == FALSE {
        return Err(io::Error::last_os_error());
    }
    sid_result
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

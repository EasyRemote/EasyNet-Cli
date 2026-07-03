//! FFI client handles and library-internal daemon IPC client.

pub mod handle;
pub(crate) mod ipc;

pub(crate) use ipc::{connect, IpcClient};

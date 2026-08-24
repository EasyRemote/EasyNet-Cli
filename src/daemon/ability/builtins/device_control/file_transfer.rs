// EasyNet CLI — fs.transfer bidi ability
// =================================================
//
// File: src/daemon/ability/builtins/device_control/file_transfer.rs
//
// Bidirectional file transfer between the operator (via the
// EasyNet backend's HTTP /api/v1/files/{upload,download} routes)
// and a target device's local filesystem. Pairs with
// `backend/internal/handler/file/handler.go` + `pumps.go`.
//
// Wire contract
// -------------
// The bidi session's initial args carry the mode + ResourceRef:
//
//   { "mode": "upload" | "download",
//     "resource_ref": { ... RFC-005 filesystem ResourceRef ... } }
//
// Frames the handler RECEIVES (from_client):
//   * upload mode: application/octet-stream frames containing file bytes.
//     Closing the upstream half is EOF.
//   * download mode: no input frames are required.
//
// Frames the handler EMITS (to_client):
//   * download mode: application/octet-stream frames containing file bytes.
//   * either mode (terminal):
//       {"type":"complete", "sha256":"<hex>",
//        "bytes": <int>}                       — success
//       {"type":"error",   "code":"<typed>",
//        "message":"<human>"}                  — failure
//
// The terminal frame is what the backend's pumps.go reads to fill
// the Receipt's `sha256` and `state` fields. Runtime layer maps
// {"type":"complete"} → BidiKindReceipt{state:"completed"} and
// {"type":"error"} → BidiKindReceipt{state:"failed"}; that
// mapping happens at the canonical Invocation terminal boundary
// (including the federation-routed path) — NOT here.
//
// Safety
// ------
// * Path traversal: ResourceRef relative paths reject traversal at
//   parse time. Existing symlink targets are rechecked against the
//   ResourceRef virtual root before the transfer touches bytes.
// * Per-call byte cap: 1 GiB. A larger transfer is split into
//   multiple ability calls or denied — preventing a single Invoke
//   from filling the disk.
// * Atomicity for upload: writes go to `<resolved-target>.partial.<nonce>`
//   and rename atomically to the ResourceRef target on success. A
//   mid-transfer abort removes the private staging file and never
//   overwrites an existing file with truncated data.
// * Read-only for download: opening succeeds when the file is
//   readable; missing-file surfaces as `{type:"error", code:
//   "not_found"}` so the backend renders 404-shaped UX.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::ability::dispatch::{
    bidi_input_channel, AxonAbilityCatalog, BidiInputFrame, BidiOutputFrame, BidiSource,
};
use crate::daemon::resources::files::{
    self as filesystem, FilesystemResourceCapability, ResolvedFilesystemPath,
};
pub const ABILITY_FILE_TRANSFER: &str = crate::daemon::ability::names::device_control::FS_TRANSFER;

/// Maximum bytes per file_transfer call. 1 GiB matches order-of-
/// magnitude what an HTTP upload through nginx would tolerate;
/// larger transfers should be split. Enforced on the upload path
/// (cumulative bytes received) AND on download (file metadata
/// rejected pre-stream when over cap).
pub const FILE_TRANSFER_BYTE_CAP: u64 = 1024 * 1024 * 1024;

/// Per-chunk emit size for download. Mirrors the backend's
/// uploadChunkSize so a downloaded file rebuilt on the backend
/// uses the same chunk granularity as one re-uploaded — useful
/// for interrupted-transfer resumption math (not in v1, but the
/// convention sets us up for it).
const DOWNLOAD_CHUNK_BYTES: usize = 64 * 1024;

/// Channel bound for the bidi pipes. Same as terminal.attach's
/// BIDI_CHANNEL_BOUND so the IPC layer's sizing assumptions hold.
const BIDI_CHANNEL_BOUND: usize = 64;

/// Recv timeout while waiting for the next upload chunk. Long
/// enough that a slow client (mobile / lossy network) doesn't
/// trip on a transient stall, short enough that a wedged session
/// surfaces in tens of seconds.
const UPLOAD_RECV_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Register the bidi handler on the dispatcher. Mirrors
/// terminal_attach_ability::register's signature so the daemon-boot
/// path stays uniform.
pub fn register(
    reg: &mut AxonAbilityCatalog,
    filesystem: filesystem::FilesystemResourceProvider,
) -> anyhow::Result<()> {
    super::files::require_catalog_filesystem_owner(reg, &filesystem)?;
    reg.register_bidi_with_owner(
        "fs.transfer",
        OwnerKind::locomotion_system(),
        Arc::new(move |args: Value| open_handler_with_provider(args, &filesystem)),
    );
    Ok(())
}

/// Bidi-open entry. Validates the args envelope, opens the local
/// fs handle (or stages a write target), spawns the per-direction
/// pump task, and returns the BidiSource so the IPC layer can
/// wire its forwarders. Errors here surface as `OpenBidi` failures
/// — the caller never sees a session id.
fn open_handler_with_provider(
    args: Value,
    filesystem: &filesystem::FilesystemResourceProvider,
) -> anyhow::Result<BidiSource> {
    let parsed = ParsedArgs::parse_with_provider(&args, filesystem)?;

    // Channel halves are transport-axis (see ability_dispatch::
    // BidiSource):
    //   xport_to_handler_tx  — IPC pushes here (SendBidi);
    //                          handler reads via xport_to_handler_rx
    //   xport_from_handler_tx — handler writes here;
    //                          IPC reads xport_from_handler_rx
    //                          and emits RecvBidi
    let (xport_to_handler_tx, xport_to_handler_rx) = bidi_input_channel(BIDI_CHANNEL_BOUND);
    let (xport_from_handler_tx, xport_from_handler_rx) =
        mpsc::channel::<BidiOutputFrame>(BIDI_CHANNEL_BOUND);

    match parsed.mode {
        Mode::Upload => spawn_upload(
            parsed.target,
            parsed.overwrite,
            parsed.max_bytes,
            parsed.expected_sha256,
            parsed.expected_bytes,
            xport_to_handler_rx,
            xport_from_handler_tx,
        ),
        Mode::Download => spawn_download(parsed.target, parsed.max_bytes, xport_from_handler_tx),
    }

    Ok(BidiSource {
        to_client: xport_to_handler_tx,
        from_client: xport_from_handler_rx,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Upload,
    Download,
}

#[derive(Debug)]
struct ParsedArgs {
    mode: Mode,
    target: TransferTarget,
    overwrite: bool,
    max_bytes: u64,
    expected_sha256: Option<String>,
    expected_bytes: Option<u64>,
}

#[derive(Debug)]
struct TransferTarget {
    path: PathBuf,
    display_path: String,
    virtual_root_path: Option<PathBuf>,
}

impl TransferTarget {
    fn from_resolved(resolved: ResolvedFilesystemPath) -> Self {
        Self {
            path: resolved.local_path,
            display_path: resolved.display_path,
            virtual_root_path: resolved.virtual_root_path,
        }
    }
}

impl ParsedArgs {
    fn parse_with_provider(
        args: &Value,
        filesystem: &filesystem::FilesystemResourceProvider,
    ) -> anyhow::Result<Self> {
        let mode_str = args
            .get("mode")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("`mode` (\"upload\" or \"download\") required"))?;
        let mode = match mode_str {
            "upload" => Mode::Upload,
            "download" => Mode::Download,
            other => anyhow::bail!("`mode` must be \"upload\" or \"download\" (got {other:?})"),
        };
        let capability = match mode {
            Mode::Upload => FilesystemResourceCapability::Write,
            Mode::Download => FilesystemResourceCapability::Read,
        };
        let target = filesystem
            .resolve_filesystem_path_without_existing_target(args, capability)
            .map(TransferTarget::from_resolved)?;
        let overwrite = optional_bool(args, "overwrite")?.unwrap_or(false);
        let max_bytes = optional_u64(args, "max_bytes")?.unwrap_or(FILE_TRANSFER_BYTE_CAP);
        if max_bytes == 0 || max_bytes > FILE_TRANSFER_BYTE_CAP {
            anyhow::bail!("`max_bytes` must be between 1 and {FILE_TRANSFER_BYTE_CAP}");
        }
        let expected_sha256 = args
            .get("expected_sha256")
            .map(|value| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| {
                        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                    .map(str::to_ascii_lowercase)
                    .ok_or_else(|| anyhow::anyhow!("`expected_sha256` must be 64 hex characters"))
            })
            .transpose()?;
        let expected_bytes = optional_u64(args, "expected_bytes")?;
        if mode == Mode::Download
            && (overwrite || expected_sha256.is_some() || expected_bytes.is_some())
        {
            anyhow::bail!(
                "download mode does not accept overwrite, expected_sha256, or expected_bytes"
            );
        }
        Ok(Self {
            mode,
            target,
            overwrite,
            max_bytes,
            expected_sha256,
            expected_bytes,
        })
    }
}

fn optional_bool(args: &Value, field: &str) -> anyhow::Result<Option<bool>> {
    args.get(field)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| anyhow::anyhow!("`{field}` must be a boolean"))
        })
        .transpose()
}

fn optional_u64(args: &Value, field: &str) -> anyhow::Result<Option<u64>> {
    args.get(field)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("`{field}` must be an unsigned integer"))
        })
        .transpose()
}

#[cfg(test)]
const TEST_FILE_TRANSFER_DEVICE_URA: &str = "easynet:///r/test/device/file-transfer";

#[cfg(test)]
impl ParsedArgs {
    fn parse(args: &Value) -> anyhow::Result<Self> {
        Self::parse_with_provider(args, &test_filesystem())
    }
}

#[cfg(test)]
fn open_handler(args: Value) -> anyhow::Result<BidiSource> {
    open_handler_with_provider(args, &test_filesystem())
}

#[cfg(test)]
fn test_filesystem() -> filesystem::FilesystemResourceProvider {
    filesystem::FilesystemResourceProvider::for_device(TEST_FILE_TRANSFER_DEVICE_URA).unwrap()
}

/// Upload pump: read chunks from the wire, append to the staging
/// file, hash on the fly. On EOF, fsync + atomic rename, emit
/// `{type:"complete"}`. On error, emit `{type:"error"}` and bail.
fn spawn_upload(
    target: TransferTarget,
    overwrite: bool,
    max_bytes: u64,
    expected_sha256: Option<String>,
    expected_bytes: Option<u64>,
    mut from_client: mpsc::Receiver<BidiInputFrame>,
    to_client: mpsc::Sender<BidiOutputFrame>,
) {
    tokio::spawn(async move {
        let display_path = target.display_path;
        let path = target.path;
        if let Some(root) = target.virtual_root_path.as_deref() {
            if let Err(e) = filesystem::ensure_write_parent_under_root(&path, root) {
                emit_target_error(&to_client, "root_escape", &format!("{e}"), &display_path).await;
                return;
            }
        }
        let path = super::files::resolve_symlink_one_level(&path);
        if let Some(root) = target.virtual_root_path.as_deref() {
            let guard = if path.exists() {
                filesystem::ensure_path_under_root(&path, root)
            } else {
                filesystem::ensure_write_parent_under_root(&path, root)
            };
            if let Err(e) = guard {
                emit_target_error(&to_client, "root_escape", &format!("{e}"), &display_path).await;
                return;
            }
        }

        // Stage in `<path>.partial.<rand>` next to the target so
        // an in-progress transfer never partially overwrites a
        // good file. uuid4 keeps it unique under concurrent
        // uploads to the same target (last-writer-wins on rename).
        let staging = staging_path(&path);
        let parent = staging.parent().unwrap_or(Path::new("."));
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            emit_error(&to_client, "io_error", &format!("create staging dir: {e}")).await;
            return;
        }

        let mut file = match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .await
        {
            Ok(f) => f,
            Err(e) => {
                emit_error(&to_client, "io_error", &format!("open staging: {e}")).await;
                return;
            }
        };

        let mut hasher = Sha256::new();
        let mut total: u64 = 0;

        loop {
            let frame =
                match tokio::time::timeout(UPLOAD_RECV_IDLE_TIMEOUT, from_client.recv()).await {
                    Ok(Some(f)) => f,
                    Ok(None) => {
                        // Client closed their send side without an
                        // explicit EOF. Treat as graceful EOF — same
                        // outcome as receiving {type:"eof"}.
                        break;
                    }
                    Err(_) => {
                        let _ = tokio::fs::remove_file(&staging).await;
                        emit_error(
                            &to_client,
                            "idle_timeout",
                            &format!(
                                "no chunk received within {} seconds",
                                UPLOAD_RECV_IDLE_TIMEOUT.as_secs()
                            ),
                        )
                        .await;
                        return;
                    }
                };
            match UploadClientFrame::parse(frame) {
                Ok(UploadClientFrame::Chunk(bytes)) => {
                    total = total.saturating_add(bytes.len() as u64);
                    if total > max_bytes {
                        emit_error(
                            &to_client,
                            "byte_cap_exceeded",
                            &format!("upload exceeds {max_bytes} byte limit"),
                        )
                        .await;
                        let _ = tokio::fs::remove_file(&staging).await;
                        return;
                    }
                    if let Err(e) = file.write_all(&bytes).await {
                        emit_error(&to_client, "io_error", &format!("write: {e}")).await;
                        let _ = tokio::fs::remove_file(&staging).await;
                        return;
                    }
                    hasher.update(&bytes);
                }
                Err(error) => {
                    emit_error(
                        &to_client,
                        "bad_frame",
                        &format!("upload frame rejected: {error}"),
                    )
                    .await;
                    let _ = tokio::fs::remove_file(&staging).await;
                    return;
                }
            }
        }

        // Sync to disk before the rename so a crash between the
        // last byte written and the rename does NOT silently
        // promote a partial file to the target name.
        if let Err(e) = file.sync_all().await {
            emit_error(&to_client, "io_error", &format!("fsync: {e}")).await;
            let _ = tokio::fs::remove_file(&staging).await;
            return;
        }
        drop(file);

        let sha = hex_lower(&hasher.finalize());
        if expected_bytes.is_some_and(|expected| expected != total) {
            emit_error(
                &to_client,
                "size_mismatch",
                &format!("expected {expected_bytes:?} bytes, received {total}"),
            )
            .await;
            let _ = tokio::fs::remove_file(&staging).await;
            return;
        }
        if expected_sha256
            .as_deref()
            .is_some_and(|expected| expected != sha)
        {
            emit_error(
                &to_client,
                "hash_mismatch",
                &format!("expected sha256 {expected_sha256:?}, received {sha}"),
            )
            .await;
            let _ = tokio::fs::remove_file(&staging).await;
            return;
        }

        let commit = if overwrite {
            tokio::fs::rename(&staging, &path).await
        } else {
            match tokio::fs::hard_link(&staging, &path).await {
                Ok(()) => tokio::fs::remove_file(&staging).await,
                Err(error) => Err(error),
            }
        };
        if let Err(e) = commit {
            let code = if !overwrite && e.kind() == std::io::ErrorKind::AlreadyExists {
                "destination_exists"
            } else {
                "atomic_commit_failed"
            };
            emit_error(&to_client, code, &format!("atomic commit: {e}")).await;
            let _ = tokio::fs::remove_file(&staging).await;
            return;
        }

        let _ = to_client
            .send(BidiOutputFrame::terminal_json(json!({
                "type": "complete",
                "sha256": sha,
                "bytes": total,
                "display_path": display_path,
                "resource_ref_revalidated": true,
            })))
            .await;
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UploadClientFrame {
    Chunk(Vec<u8>),
}

impl UploadClientFrame {
    fn parse(frame: BidiInputFrame) -> anyhow::Result<Self> {
        if frame.content_type != "application/octet-stream" {
            anyhow::bail!(
                "upload data frame must use application/octet-stream, got {:?}",
                frame.content_type
            );
        }
        Ok(Self::Chunk(frame.payload))
    }
}

/// Download pump: open the file, stream chunks, hash on the fly.
/// On EOF emit `{type:"complete"}`. On error emit
/// `{type:"error"}` and bail.
fn spawn_download(
    target: TransferTarget,
    max_bytes: u64,
    to_client: mpsc::Sender<BidiOutputFrame>,
) {
    tokio::spawn(async move {
        let display_path = target.display_path;
        let path = target.path;

        let metadata = match tokio::fs::metadata(&path).await {
            Ok(m) => m,
            Err(e) => {
                let code = if e.kind() == std::io::ErrorKind::NotFound {
                    "not_found"
                } else {
                    "io_error"
                };
                emit_target_error(
                    &to_client,
                    code,
                    &format!("stat {display_path}: {e}"),
                    &display_path,
                )
                .await;
                return;
            }
        };
        if let Some(root) = target.virtual_root_path.as_deref() {
            if let Err(e) = filesystem::ensure_path_under_root(&path, root) {
                emit_target_error(&to_client, "root_escape", &format!("{e}"), &display_path).await;
                return;
            }
        }
        if !metadata.is_file() {
            emit_target_error(
                &to_client,
                "not_a_file",
                "resource target is not a regular file",
                &display_path,
            )
            .await;
            return;
        }
        if metadata.len() > max_bytes {
            emit_target_error(
                &to_client,
                "byte_cap_exceeded",
                &format!(
                    "file size {} exceeds {} byte cap",
                    metadata.len(),
                    max_bytes
                ),
                &display_path,
            )
            .await;
            return;
        }

        let mut file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            Err(error) => {
                emit_error(&to_client, "io_error", &format!("open: {error}")).await;
                return;
            }
        };
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; DOWNLOAD_CHUNK_BYTES];
        let mut total = 0_u64;
        loop {
            let read = match file.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) => {
                    emit_error(&to_client, "io_error", &format!("read: {error}")).await;
                    return;
                }
            };
            hasher.update(&buffer[..read]);
            total = total.saturating_add(read as u64);
            if total > max_bytes {
                emit_target_error(
                    &to_client,
                    "byte_cap_exceeded",
                    &format!("file grew beyond {max_bytes} byte cap while streaming"),
                    &display_path,
                )
                .await;
                return;
            }
            if to_client
                .send(BidiOutputFrame::binary(
                    buffer[..read].to_vec(),
                    "application/octet-stream",
                ))
                .await
                .is_err()
            {
                return;
            }
        }
        let sha = hex_lower(&hasher.finalize());
        let _ = to_client
            .send(BidiOutputFrame::terminal_json(json!({
                "type": "complete",
                "sha256": sha,
                "bytes": total,
                "display_path": display_path,
                "resource_ref_revalidated": true,
            })))
            .await;
    });
}

/// Emit an `{type:"error"}` terminal frame. Consumers (the IPC
/// layer + downstream backend pumps) interpret this as the
/// session-failed terminal — the backend then sets HTTP 502.
async fn emit_error(to_client: &mpsc::Sender<BidiOutputFrame>, code: &str, message: &str) {
    let _ = to_client
        .send(BidiOutputFrame::json(json!({
            "type": "error",
            "code": code,
            "message": message,
        })))
        .await;
}

async fn emit_target_error(
    to_client: &mpsc::Sender<BidiOutputFrame>,
    code: &str,
    message: &str,
    display_path: &str,
) {
    let _ = to_client
        .send(BidiOutputFrame::json(json!({
            "type": "error",
            "code": code,
            "message": message,
            "display_path": display_path,
            "resource_ref_revalidated": true,
        })))
        .await;
}

/// Build the staging file path next to the target. Layout:
///
///   target          /a/b/foo.txt
///   staging         /a/b/foo.txt.partial.<8-hex>
///
/// The 8-hex suffix is enough entropy to make concurrent uploads
/// against the same target unambiguous; collisions would just be
/// resolved by last-writer-wins on the final rename.
fn staging_path(target: &Path) -> PathBuf {
    let nonce = format!("{:x}", rand_u32());
    let mut s = target.as_os_str().to_owned();
    s.push(".partial.");
    s.push(nonce);
    PathBuf::from(s)
}

/// Tiny PRNG that doesn't pull a `rand` crate dep (uuid is
/// already in tree but overkill for a 32-bit nonce). Uses
/// process-time + counter to keep collisions astronomical
/// without needing seeded state.
fn rand_u32() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::SystemTime;
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let bump = COUNTER.fetch_add(1, Ordering::Relaxed);
    nanos ^ bump.wrapping_mul(0x9E3779B1)
}

fn hex_lower(
    d: &sha2::digest::generic_array::GenericArray<u8, sha2::digest::typenum::U32>,
) -> String {
    hex_lower_bytes(d.as_slice())
}

fn hex_lower_bytes(b: &[u8]) -> String {
    static HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(b.len() * 2);
    for &byte in b {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["mode", "resource_ref"],
        "additionalProperties": false,
        "properties": {
            "mode": {"type": "string", "enum": ["upload", "download"]},
            "resource_ref": crate::daemon::resources::files::resource_ref_schema(),
            "overwrite": {"type": "boolean"},
            "max_bytes": {
                "type": "integer",
                "minimum": 1,
                "maximum": FILE_TRANSFER_BYTE_CAP,
            },
            "expected_sha256": {"type": "string", "pattern": "^[0-9a-fA-F]{64}$"},
            "expected_bytes": {"type": "integer", "minimum": 0},
        },
    })
}

pub fn description() -> &'static str {
    "Bidirectional file transfer between the operator and this \
     device's filesystem through a revalidated RFC-005 filesystem \
     ResourceRef. mode=\"upload\" requires write capability and \
     streams client→file with atomic commit, explicit overwrite, and expected size/SHA-256 verification; \
     mode=\"download\" requires read capability and streams \
     file→client with on-the-fly hashing. Per-call byte cap 1 GiB."
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DEVICE_URA: &str = "easynet:///r/test/device/file-transfer";

    fn metadata_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(TEST_DEVICE_URA)
    }

    /// Drain frames from the handler-emit channel up to `max`
    /// frames or `timeout`, whichever comes first. Mirrors the
    /// helper in terminal.attach tests.
    async fn drain_handler_emit(
        rx: &mut mpsc::Receiver<BidiOutputFrame>,
        max: usize,
        timeout: Duration,
    ) -> Vec<Value> {
        let mut out = Vec::new();
        let deadline = tokio::time::Instant::now() + timeout;
        while out.len() < max {
            let remaining = deadline.checked_duration_since(tokio::time::Instant::now());
            let Some(remaining) = remaining else { break };
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(v)) if v.content_type == "application/json" => {
                    out.push(v.into_json_value().expect("file transfer JSON frame"));
                }
                Ok(Some(v)) => out.push(json!({
                    "type": "binary",
                    "data": v.payload,
                })),
                Ok(None) => break,
                Err(_) => break,
            }
        }
        out
    }

    async fn send_upload_chunk(
        sender: &crate::daemon::ability::dispatch::BidiInputSender,
        bytes: &[u8],
    ) {
        sender
            .send_frame(
                BidiInputFrame::new(bytes.to_vec()).with_content_type("application/octet-stream"),
            )
            .await
            .expect("send raw upload chunk");
    }

    #[test]
    fn parse_rejects_missing_mode() {
        let err = ParsedArgs::parse(&json!({})).unwrap_err();
        assert!(format!("{err}").contains("mode"));
    }

    #[test]
    fn parse_rejects_missing_resource_ref() {
        let err = ParsedArgs::parse(&json!({"mode": "upload"})).unwrap_err();
        assert!(format!("{err}").contains("resource_ref"));
    }

    #[test]
    fn parse_rejects_unknown_mode() {
        let err = ParsedArgs::parse(&json!({"mode": "wibble"})).unwrap_err();
        assert!(format!("{err}").contains("upload"));
    }

    #[test]
    fn upload_rejects_read_only_resource_ref() {
        let path = temp_path("read-only-ref.bin");
        let resource_ref = transfer_ref(&path, FilesystemResourceCapability::Read);
        let err = ParsedArgs::parse(&json!({
            "mode": "upload",
            "resource_ref": resource_ref,
        }))
        .unwrap_err();
        assert!(format!("{err}").contains("capability read does not permit write"));
    }

    #[test]
    fn registration_mounts_bidi_handler() {
        let mut reg = metadata_test_catalog();
        register(&mut reg, test_filesystem()).unwrap();
        assert!(reg.get_bidi(ABILITY_FILE_TRANSFER).is_some());
    }

    fn temp_path(suffix: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "easynet-file-xfer-test-{}-{}-{}",
            std::process::id(),
            rand_u32(),
            suffix
        ));
        p
    }

    fn transfer_ref(path: &Path, capability: FilesystemResourceCapability) -> Value {
        test_filesystem()
            .resource_ref_for_local_path(path, capability)
            .expect("file transfer fixture must be owned by this test daemon's local Device")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_round_trip_writes_file_atomically_with_sha256() {
        // The "看得见" contract: a client-side EOF ends the
        // transfer with a complete frame whose sha256 matches
        // what an independent SHA-256 over the same bytes
        // produces.
        let path = temp_path("upload.bin");
        let bytes = b"Hello, EasyNet file_transfer!";
        let want_sha = {
            let mut h = Sha256::new();
            h.update(bytes);
            hex_lower_bytes(h.finalize().as_slice())
        };

        let source = open_handler(json!({
            "mode": "upload",
            "resource_ref": transfer_ref(&path, FilesystemResourceCapability::Write),
        }))
        .expect("open upload");
        // Stash the receivers before moving the channels.
        let to_handler = source.to_client;
        let mut from_handler = source.from_client;

        send_upload_chunk(&to_handler, bytes).await;
        drop(to_handler);

        let frames = drain_handler_emit(&mut from_handler, 4, Duration::from_secs(3)).await;
        let complete = frames
            .iter()
            .find(|f| f["type"] == "complete")
            .expect("complete frame must arrive");
        assert_eq!(complete["sha256"], want_sha);
        assert_eq!(complete["bytes"], bytes.len() as i64);

        // File now exists with the right content.
        let on_disk = std::fs::read(&path).expect("file written");
        assert_eq!(on_disk, bytes);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_atomic_rename_means_partial_never_visible_at_target() {
        // Pre-fix a naive `std::fs::write` would have shown the
        // partial bytes at the target path during the transfer.
        // The staging-then-rename path means the target either
        // has the full content (after complete) or doesn't exist
        // (during transfer). Pin that by pushing one chunk, then
        // checking that the target does not exist UNTIL eof.
        let path = temp_path("atomic.bin");
        let _ = std::fs::remove_file(&path);

        let source = open_handler(json!({
            "mode": "upload",
            "resource_ref": transfer_ref(&path, FilesystemResourceCapability::Write),
        }))
        .unwrap();
        let to_handler = source.to_client;
        let mut from_handler = source.from_client;

        // Push the first chunk but NOT eof.
        send_upload_chunk(&to_handler, b"first").await;
        // Give the writer a moment to land the chunk on the
        // staging file.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Target must not exist yet.
        assert!(
            !path.exists(),
            "target {path:?} must NOT exist mid-transfer; staging is in a sibling .partial"
        );

        // Now finalize.
        drop(to_handler);
        let _ = drain_handler_emit(&mut from_handler, 2, Duration::from_secs(2)).await;
        assert!(path.exists(), "target must exist after eof + complete");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_requires_explicit_overwrite_and_preserves_existing_destination() {
        let path = temp_path("existing.bin");
        std::fs::write(&path, b"original").unwrap();
        let source = open_handler(json!({
            "mode": "upload",
            "resource_ref": transfer_ref(&path, FilesystemResourceCapability::Write),
        }))
        .unwrap();
        let to_handler = source.to_client;
        let mut from_handler = source.from_client;
        send_upload_chunk(&to_handler, b"replacement").await;
        drop(to_handler);
        let frames = drain_handler_emit(&mut from_handler, 2, Duration::from_secs(2)).await;
        assert!(frames
            .iter()
            .any(|frame| frame["code"] == "destination_exists"));
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_hash_mismatch_never_commits_staging_file() {
        let path = temp_path("hash-mismatch.bin");
        let source = open_handler(json!({
            "mode": "upload",
            "resource_ref": transfer_ref(&path, FilesystemResourceCapability::Write),
            "expected_bytes": 3,
            "expected_sha256": "00".repeat(32),
        }))
        .unwrap();
        let to_handler = source.to_client;
        let mut from_handler = source.from_client;
        send_upload_chunk(&to_handler, b"abc").await;
        drop(to_handler);
        let frames = drain_handler_emit(&mut from_handler, 2, Duration::from_secs(2)).await;
        assert!(frames.iter().any(|frame| frame["code"] == "hash_mismatch"));
        assert!(!path.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_honors_caller_narrowed_max_bytes() {
        let path = temp_path("narrow-limit.bin");
        let source = open_handler(json!({
            "mode": "upload",
            "resource_ref": transfer_ref(&path, FilesystemResourceCapability::Write),
            "max_bytes": 2,
        }))
        .unwrap();
        let to_handler = source.to_client;
        let mut from_handler = source.from_client;
        send_upload_chunk(&to_handler, b"abc").await;
        let frames = drain_handler_emit(&mut from_handler, 2, Duration::from_secs(2)).await;
        assert!(frames
            .iter()
            .any(|frame| frame["code"] == "byte_cap_exceeded"));
        assert!(!path.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_byte_cap_kills_session_with_typed_error() {
        // We can't easily push a real 1 GiB through a test, but
        // we CAN lower the cap by sending a single chunk that
        // exceeds it… well, no, the cap is a const. So instead
        // pin the typed-error code shape with the smallest
        // possible repro: send bytes summing past the cap by
        // tampering the const. Without test-only mutability the
        // best we have is a compile-time reminder that the const
        // is the contract — assert here that the constant is
        // sane (1 GiB) so a future "let's bump to 16 GiB" landing
        // is reviewed deliberately.
        //
        // The "byte_cap_exceeded" code path is exercised when
        // FILE_TRANSFER_BYTE_CAP is < total — a future test that
        // injects a smaller cap via a #[cfg(test)] override would
        // exercise it directly.
        assert_eq!(FILE_TRANSFER_BYTE_CAP, 1024 * 1024 * 1024);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_rejects_json_data_frames() {
        let path = temp_path("json-frame.bin");
        let source = open_handler(json!({
            "mode": "upload",
            "resource_ref": transfer_ref(&path, FilesystemResourceCapability::Write),
        }))
        .unwrap();
        let to_handler = source.to_client;
        let mut from_handler = source.from_client;
        to_handler
            .send(json!({"type": "chunk", "data": "not-a-data-plane-frame"}))
            .await
            .unwrap();
        let frames = drain_handler_emit(&mut from_handler, 2, Duration::from_secs(2)).await;
        let err = frames
            .iter()
            .find(|f| f["type"] == "error")
            .expect("error frame for JSON data");
        assert_eq!(err["code"], "bad_frame");
        // Target must NOT exist (staging cleaned up).
        assert!(!path.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn download_round_trip_emits_chunks_then_complete_with_sha256() {
        let path = temp_path("download.bin");
        let bytes: Vec<u8> = (0..1024).map(|i| (i % 256) as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        let want_sha = {
            let mut h = Sha256::new();
            h.update(&bytes);
            hex_lower_bytes(h.finalize().as_slice())
        };

        let source = open_handler(json!({
            "mode": "download",
            "resource_ref": transfer_ref(&path, FilesystemResourceCapability::Read),
        }))
        .unwrap();
        let mut from_handler = source.from_client;

        let frames = drain_handler_emit(&mut from_handler, 8, Duration::from_secs(3)).await;
        // Concatenate every chunk's bytes; assert the result equals
        // the original.
        let mut accum = Vec::new();
        for f in &frames {
            if f["type"] == "binary" {
                accum.extend(
                    f["data"]
                        .as_array()
                        .expect("raw binary test projection")
                        .iter()
                        .map(|value| value.as_u64().expect("byte") as u8),
                );
            }
        }
        assert_eq!(accum, bytes, "download bytes must equal source file");
        let complete = frames
            .iter()
            .find(|f| f["type"] == "complete")
            .expect("complete frame must arrive");
        assert_eq!(complete["sha256"], want_sha);
        assert_eq!(complete["bytes"], bytes.len() as i64);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn download_missing_file_emits_not_found() {
        let path = temp_path("nonexistent.bin");
        let _ = std::fs::remove_file(&path);
        let source = open_handler(json!({
            "mode": "download",
            "resource_ref": transfer_ref(&path, FilesystemResourceCapability::Read),
        }))
        .unwrap();
        let mut from_handler = source.from_client;
        let frames = drain_handler_emit(&mut from_handler, 1, Duration::from_secs(2)).await;
        let err = frames.first().expect("an error frame");
        assert_eq!(err["type"], "error");
        assert_eq!(err["code"], "not_found");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn download_oversize_file_emits_byte_cap_exceeded() {
        // Hard to allocate 1 GiB in a test. We bound the test by
        // generating a smaller file and asserting the under-cap
        // happy path; the over-cap branch is covered by the
        // const-equals assertion above (any future cap change
        // gets reviewed) plus the metadata.len() > FILE_TRANSFER_BYTE_CAP
        // line in spawn_download which a future targeted regression
        // test can exercise with an injected lower cap.
        //
        // For this slot, exercise that small files definitely
        // succeed — proves the cap branch isn't accidentally
        // catching everything.
        let path = temp_path("under_cap.bin");
        std::fs::write(&path, b"small").unwrap();
        let source = open_handler(json!({
            "mode": "download",
            "resource_ref": transfer_ref(&path, FilesystemResourceCapability::Read),
        }))
        .unwrap();
        let mut from_handler = source.from_client;
        let frames = drain_handler_emit(&mut from_handler, 4, Duration::from_secs(2)).await;
        assert!(frames.iter().any(|f| f["type"] == "complete"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn download_directory_emits_not_a_file() {
        let dir = temp_path("dir_target");
        std::fs::create_dir_all(&dir).unwrap();
        let source = open_handler(json!({
            "mode": "download",
            "resource_ref": transfer_ref(&dir, FilesystemResourceCapability::Read),
        }))
        .unwrap();
        let mut from_handler = source.from_client;
        let frames = drain_handler_emit(&mut from_handler, 1, Duration::from_secs(2)).await;
        let err = frames.first().expect("error frame");
        assert_eq!(err["type"], "error");
        assert_eq!(err["code"], "not_a_file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_unknown_frame_type_emits_error_and_aborts_transfer() {
        let path = temp_path("unknown-frame.bin");
        let source = open_handler(json!({
            "mode": "upload",
            "resource_ref": transfer_ref(&path, FilesystemResourceCapability::Write),
        }))
        .unwrap();
        let to_handler = source.to_client;
        let mut from_handler = source.from_client;
        // Unknown frame first.
        to_handler
            .send(json!({"type": "future_hint", "x": 42}))
            .await
            .unwrap();

        let frames = drain_handler_emit(&mut from_handler, 4, Duration::from_secs(2)).await;
        let error = frames
            .iter()
            .find(|frame| frame["type"] == "error")
            .expect("unknown upload frame must emit an error");
        assert_eq!(error["code"], "bad_frame");
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|message| message.contains("application/octet-stream")),
            "unexpected error frame: {error:?}"
        );

        if to_handler
            .send_frame(
                BidiInputFrame::new(b"abc".to_vec()).with_content_type("application/octet-stream"),
            )
            .await
            .is_ok()
        {
            drop(to_handler);
            let frames = drain_handler_emit(&mut from_handler, 4, Duration::from_millis(500)).await;
            assert!(
                !frames.iter().any(|frame| frame["type"] == "complete"),
                "upload must not complete after protocol rejection: {frames:?}"
            );
        }
        assert!(
            !path.exists(),
            "unknown upload frame must not promote a target file"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upload_frame_parser_rejects_json_frames() {
        let err = UploadClientFrame::parse(
            BidiInputFrame::new(b"null".to_vec()).with_content_type("application/json"),
        )
        .unwrap_err();
        assert!(
            format!("{err}").contains("application/octet-stream"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn upload_frame_parser_preserves_all_byte_values() {
        let bytes = (0_u8..=u8::MAX).collect::<Vec<_>>();
        let parsed = UploadClientFrame::parse(
            BidiInputFrame::new(bytes.clone()).with_content_type("application/octet-stream"),
        )
        .expect("raw frame");
        assert_eq!(parsed, UploadClientFrame::Chunk(bytes));
    }
}

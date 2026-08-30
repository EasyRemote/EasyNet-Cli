// EasyNet CLI - governed device file copy
// =======================================
//
// File: src/cli/commands/device_cp.rs
// Description: Stream exactly one local file to or from one canonical Device
//              endpoint through descriptor-bound fs.transfer InvokeBidi.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(feature = "axon-pb")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::daemon::resources::files::FilesystemResourceCapability;

const TRANSFER_CHUNK_BYTES: usize = 64 * 1024;
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
enum CopyEndpoint {
    Local(PathBuf),
    Remote {
        device_ura: String,
        absolute_virtual_path: String,
    },
}

impl CopyEndpoint {
    fn parse(raw: &str) -> anyhow::Result<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            anyhow::bail!("INVALID_ENDPOINT: endpoint must not be empty");
        }
        if !raw.starts_with("easynet:///") {
            return Ok(Self::Local(PathBuf::from(raw)));
        }
        let separator = raw.rfind(":/").ok_or_else(|| {
            anyhow::anyhow!(
                "INVALID_ENDPOINT: remote endpoint must be <canonical-device-ura>:<absolute-path>"
            )
        })?;
        let device_ura = &raw[..separator];
        let absolute_virtual_path = &raw[separator + 1..];
        let parsed = crate::core::ura::parse_ura(device_ura)
            .map_err(|error| anyhow::anyhow!("INVALID_ENDPOINT: {error}"))?;
        if parsed.kind != crate::core::ura::URAKind::Device {
            anyhow::bail!("INVALID_ENDPOINT: remote owner must be a Device URA");
        }
        if !absolute_virtual_path.starts_with('/') {
            anyhow::bail!("INVALID_ENDPOINT: remote path must be absolute");
        }
        Ok(Self::Remote {
            device_ura: device_ura.to_string(),
            absolute_virtual_path: absolute_virtual_path.to_string(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyDirection {
    Upload,
    Download,
}

#[derive(Debug)]
struct CopyPlan {
    direction: CopyDirection,
    local_path: PathBuf,
    remote_device_ura: String,
    remote_path: String,
    overwrite: bool,
    max_bytes: u64,
}

impl CopyPlan {
    fn new(
        source: &str,
        destination: &str,
        overwrite: bool,
        max_bytes: u64,
    ) -> anyhow::Result<Self> {
        if max_bytes == 0
            || max_bytes > crate::daemon::ability::builtins::device_control::file_transfer::FILE_TRANSFER_BYTE_CAP
        {
            anyhow::bail!("SIZE_LIMIT_EXCEEDED: max-bytes is outside the fs.transfer limit");
        }
        match (
            CopyEndpoint::parse(source)?,
            CopyEndpoint::parse(destination)?,
        ) {
            (
                CopyEndpoint::Local(local_path),
                CopyEndpoint::Remote {
                    device_ura,
                    absolute_virtual_path,
                },
            ) => Ok(Self {
                direction: CopyDirection::Upload,
                local_path,
                remote_device_ura: device_ura,
                remote_path: absolute_virtual_path,
                overwrite,
                max_bytes,
            }),
            (
                CopyEndpoint::Remote {
                    device_ura,
                    absolute_virtual_path,
                },
                CopyEndpoint::Local(local_path),
            ) => Ok(Self {
                direction: CopyDirection::Download,
                local_path,
                remote_device_ura: device_ura,
                remote_path: absolute_virtual_path,
                overwrite,
                max_bytes,
            }),
            (CopyEndpoint::Remote { .. }, CopyEndpoint::Remote { .. }) => {
                anyhow::bail!("REMOTE_TO_REMOTE_UNSUPPORTED: exactly one endpoint must be local")
            }
            (CopyEndpoint::Local(_), CopyEndpoint::Local(_)) => {
                anyhow::bail!(
                    "INVALID_ENDPOINT: exactly one endpoint must be a canonical Device endpoint"
                )
            }
        }
    }
}

pub(crate) fn run(
    source: &str,
    destination: &str,
    overwrite: bool,
    max_bytes: u64,
) -> anyhow::Result<()> {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (source, destination, overwrite, max_bytes);
        return Err(
            crate::support::platform::local_invoke::federation_capability_unsupported_error(
                "copying a file through remote fs.transfer",
            ),
        );
    }

    #[cfg(feature = "axon-pb")]
    {
        let plan = CopyPlan::new(source, destination, overwrite, max_bytes)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("build runtime for device cp")?;
        runtime.block_on(execute(plan))
    }
}

#[cfg(feature = "axon-pb")]
async fn execute(plan: CopyPlan) -> anyhow::Result<()> {
    let identity =
        crate::support::platform::remote_device::PairedInvocationIdentity::load("device cp")?;
    if plan.remote_device_ura == identity.local_device_ura() {
        anyhow::bail!("INVALID_ENDPOINT: device cp requires a remote Device; use the local filesystem directly");
    }
    let caller_ura = identity.caller_user_ura().to_string();
    let signer =
        crate::daemon::invocation::routing::remote_invoke::load_remote_invocation_caller_signer(
            &caller_ura,
        )
        .context("prepare device cp caller signer")?;
    let capability = match plan.direction {
        CopyDirection::Upload => FilesystemResourceCapability::Write,
        CopyDirection::Download => FilesystemResourceCapability::Read,
    };
    let resource_ref =
        crate::daemon::resources::files::resource_ref_for_target_absolute_virtual_path(
            &plan.remote_path,
            capability,
            &plan.remote_device_ura,
        )?;
    let subject_ura = resource_ref
        .get("resource_ura")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("ResourceRef omitted resource_ura"))?;

    match plan.direction {
        CopyDirection::Upload => {
            validate_local_source(&plan.local_path, plan.max_bytes).await?;
            let session =
                crate::cli::daemon_client::remote_system_ability::open_remote_file_transfer(
                    &plan.remote_device_ura,
                    &caller_ura,
                    &subject_ura,
                    json!({
                        "mode": "upload",
                        "resource_ref": resource_ref,
                        "overwrite": plan.overwrite,
                        "max_bytes": plan.max_bytes,
                    }),
                    signer,
                    TRANSFER_TIMEOUT,
                )
                .await?;
            upload(session, &plan.local_path, plan.max_bytes).await?;
        }
        CopyDirection::Download => {
            if plan.local_path.exists() && !plan.overwrite {
                anyhow::bail!("DESTINATION_EXISTS: {}", plan.local_path.display());
            }
            let session =
                crate::cli::daemon_client::remote_system_ability::open_remote_file_transfer(
                    &plan.remote_device_ura,
                    &caller_ura,
                    &subject_ura,
                    json!({
                        "mode": "download",
                        "resource_ref": resource_ref,
                        "max_bytes": plan.max_bytes,
                    }),
                    signer,
                    TRANSFER_TIMEOUT,
                )
                .await?;
            download(session, &plan.local_path, plan.overwrite, plan.max_bytes).await?;
        }
    }
    Ok(())
}

#[cfg(feature = "axon-pb")]
async fn validate_local_source(path: &Path, max_bytes: u64) -> anyhow::Result<()> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .with_context(|| format!("read source metadata for {}", path.display()))?;
    if !metadata.file_type().is_file() {
        anyhow::bail!("INVALID_ENDPOINT: source must be a regular non-symlink file");
    }
    if metadata.len() > max_bytes {
        anyhow::bail!("SIZE_LIMIT_EXCEEDED: source is {} bytes", metadata.len());
    }
    Ok(())
}

#[cfg(feature = "axon-pb")]
async fn upload(
    session: crate::support::platform::bidi_session::DaemonBidiSession,
    path: &Path,
    max_bytes: u64,
) -> anyhow::Result<()> {
    let (mut upstream, mut downstream) = session.split();
    let completion = receive_completion(&mut downstream, None);
    tokio::pin!(completion);
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; TRANSFER_CHUNK_BYTES];
    loop {
        let read = tokio::select! {
            result = &mut completion => {
                result?;
                anyhow::bail!("TRANSFER_INTERRUPTED: remote completed before local EOF");
            }
            result = file.read(&mut buffer) => result?,
        };
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > max_bytes {
            anyhow::bail!("SIZE_LIMIT_EXCEEDED: source changed while streaming");
        }
        hasher.update(&buffer[..read]);
        tokio::select! {
            result = &mut completion => {
                result?;
                anyhow::bail!("TRANSFER_INTERRUPTED: remote completed before local EOF");
            }
            result = upstream.send_binary(buffer[..read].to_vec()) => result?,
        }
    }
    tokio::select! {
        result = &mut completion => {
            result?;
            anyhow::bail!("TRANSFER_INTERRUPTED: remote completed before local EOF");
        }
        result = upstream.send_eof() => result?,
    }
    let complete = completion.await?;
    let sha256 = hex::encode(hasher.finalize());
    verify_completion(&complete, total, &sha256)?;
    println!("copied {total} bytes (sha256 {sha256})");
    Ok(())
}

struct LocalDownloadStage {
    path: PathBuf,
    committed: bool,
}

impl Drop for LocalDownloadStage {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(feature = "axon-pb")]
async fn download(
    session: crate::support::platform::bidi_session::DaemonBidiSession,
    destination: &Path,
    overwrite: bool,
    max_bytes: u64,
) -> anyhow::Result<()> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        anyhow::bail!("INVALID_ENDPOINT: destination parent does not exist");
    }
    let stage_path = parent.join(format!(
        ".{}.easynet-partial-{}",
        destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("download"),
        uuid::Uuid::new_v4().simple(),
    ));
    let mut stage = LocalDownloadStage {
        path: stage_path.clone(),
        committed: false,
    };
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stage_path)
        .await?;
    let (_upstream, mut downstream) = session.split();
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut sink = DownloadSink {
        file: &mut file,
        hasher: &mut hasher,
        total: &mut total,
        max_bytes,
    };
    let complete = receive_completion(&mut downstream, Some(&mut sink)).await?;
    file.sync_all().await?;
    drop(file);
    let sha256 = hex::encode(hasher.finalize());
    verify_completion(&complete, total, &sha256)?;
    let commit = if overwrite {
        tokio::fs::rename(&stage_path, destination).await
    } else {
        match tokio::fs::hard_link(&stage_path, destination).await {
            Ok(()) => tokio::fs::remove_file(&stage_path).await,
            Err(error) => Err(error),
        }
    };
    commit.with_context(|| format!("ATOMIC_COMMIT_FAILED: {}", destination.display()))?;
    stage.committed = true;
    println!("copied {total} bytes (sha256 {sha256})");
    Ok(())
}

#[cfg(feature = "axon-pb")]
struct DownloadSink<'a> {
    file: &'a mut tokio::fs::File,
    hasher: &'a mut Sha256,
    total: &'a mut u64,
    max_bytes: u64,
}

#[cfg(feature = "axon-pb")]
async fn receive_completion(
    downstream: &mut crate::support::platform::bidi_session::DaemonBidiReceiver,
    mut download: Option<&mut DownloadSink<'_>>,
) -> anyhow::Result<Value> {
    let mut complete = None;
    while let Some(frame) = downstream.recv().await? {
        if let Some(binary) = frame.binary.as_ref() {
            write_download_bytes(&mut download, &binary.data).await?;
            continue;
        }
        if frame.payload.get("type").and_then(Value::as_str) == Some("receipt") {
            if frame.terminal {
                if let Some(message) = frame
                    .payload
                    .pointer("/failure/message")
                    .and_then(Value::as_str)
                {
                    anyhow::bail!("TRANSFER_INTERRUPTED: {message}");
                }
                let terminal_payload = frame.payload.get("payload").ok_or_else(|| {
                    anyhow::anyhow!("TRANSFER_INTERRUPTED: terminal receipt omitted payload")
                })?;
                complete = apply_transfer_business_frame(terminal_payload)?;
            }
        } else if frame.payload.get("type").and_then(Value::as_str) != Some("control") {
            complete = apply_transfer_business_frame(&frame.payload)?;
        }
        if frame.terminal {
            break;
        }
    }
    complete.ok_or_else(|| anyhow::anyhow!("TRANSFER_INTERRUPTED: completion frame missing"))
}

fn apply_transfer_business_frame(payload: &Value) -> anyhow::Result<Option<Value>> {
    match payload.get("type").and_then(Value::as_str) {
        Some("complete") => Ok(Some(payload.clone())),
        Some("error") => {
            let code = payload
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("TRANSFER_INTERRUPTED");
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("remote transfer failed");
            anyhow::bail!("{code}: {message}");
        }
        Some(other) => anyhow::bail!("TRANSFER_INTERRUPTED: unsupported frame {other:?}"),
        None => anyhow::bail!("TRANSFER_INTERRUPTED: business frame omitted type"),
    }
}

#[cfg(feature = "axon-pb")]
async fn write_download_bytes(
    download: &mut Option<&mut DownloadSink<'_>>,
    bytes: &[u8],
) -> anyhow::Result<()> {
    let Some(sink) = download.as_deref_mut() else {
        anyhow::bail!("TRANSFER_INTERRUPTED: upload received a download chunk");
    };
    *sink.total = sink.total.saturating_add(bytes.len() as u64);
    if *sink.total > sink.max_bytes {
        anyhow::bail!("SIZE_LIMIT_EXCEEDED: remote stream exceeded max-bytes");
    }
    sink.file.write_all(bytes).await?;
    sink.hasher.update(bytes);
    Ok(())
}

fn verify_completion(
    complete: &Value,
    expected_bytes: u64,
    expected_sha256: &str,
) -> anyhow::Result<()> {
    let actual_bytes = complete
        .get("bytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("TRANSFER_INTERRUPTED: completion omitted bytes"))?;
    let actual_sha256 = complete
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("TRANSFER_INTERRUPTED: completion omitted sha256"))?;
    if actual_bytes != expected_bytes {
        anyhow::bail!("HASH_MISMATCH: byte count expected {expected_bytes}, got {actual_bytes}");
    }
    if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
        anyhow::bail!("HASH_MISMATCH: sha256 expected {expected_sha256}, got {actual_sha256}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_parser_distinguishes_device_separator_from_scheme() {
        assert_eq!(
            CopyEndpoint::parse("easynet:///r/acme/device/node-a:/tmp/a.txt").unwrap(),
            CopyEndpoint::Remote {
                device_ura: "easynet:///r/acme/device/node-a".to_string(),
                absolute_virtual_path: "/tmp/a.txt".to_string(),
            }
        );
        assert_eq!(
            CopyEndpoint::parse("./local.txt").unwrap(),
            CopyEndpoint::Local(PathBuf::from("./local.txt"))
        );
    }

    #[test]
    fn plan_rejects_remote_to_remote_and_local_to_local() {
        let remote = "easynet:///r/acme/device/node-a:/tmp/a";
        assert!(CopyPlan::new(remote, remote, false, 1024)
            .unwrap_err()
            .to_string()
            .contains("REMOTE_TO_REMOTE_UNSUPPORTED"));
        assert!(CopyPlan::new("a", "b", false, 1024)
            .unwrap_err()
            .to_string()
            .contains("exactly one endpoint"));
    }

    #[test]
    fn terminal_receipt_payload_is_the_authoritative_completion() {
        let completion = apply_transfer_business_frame(&json!({
            "type": "complete",
            "bytes": 3,
            "sha256": "abc",
        }))
        .expect("valid terminal receipt payload")
        .expect("completion extracted");

        assert_eq!(completion["bytes"], 3);
        assert_eq!(completion["sha256"], "abc");
    }

    #[test]
    fn upload_rejects_unexpected_download_chunk() {
        let error = apply_transfer_business_frame(&json!({"type": "chunk"}))
            .expect_err("upload cannot accept a download frame");

        assert!(error.to_string().contains("unsupported frame"));
    }

    #[tokio::test]
    async fn download_accepts_lossless_binary_wire_projection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("download.bin");
        let mut file = tokio::fs::File::create(&path)
            .await
            .expect("create download");
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut sink = DownloadSink {
            file: &mut file,
            hasher: &mut hasher,
            total: &mut total,
            max_bytes: 16,
        };
        write_download_bytes(&mut Some(&mut sink), &[0, 1, 2, 255])
            .await
            .expect("binary chunk accepted");
        drop(sink);
        file.sync_all().await.expect("flush download bytes");
        drop(file);

        assert_eq!(total, 4);
        assert_eq!(std::fs::read(path).expect("download bytes"), [0, 1, 2, 255]);
    }
}

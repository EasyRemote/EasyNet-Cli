// EasyNet CLI - governed device file copy
// =======================================
//
// File: src/cli/commands/device_cp.rs
// Description: Stream exactly one local file to or from one canonical Device
//              endpoint through descriptor-bound fs.transfer InvokeBidi.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

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
            let (bytes, sha256) = hash_local_source(&plan.local_path, plan.max_bytes)?;
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
                        "expected_bytes": bytes,
                        "expected_sha256": sha256,
                    }),
                    signer,
                    TRANSFER_TIMEOUT,
                )
                .await?;
            upload(session, &plan.local_path, bytes, &sha256).await?;
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

fn hash_local_source(path: &Path, max_bytes: u64) -> anyhow::Result<(u64, String)> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("read source metadata for {}", path.display()))?;
    if !metadata.file_type().is_file() {
        anyhow::bail!("INVALID_ENDPOINT: source must be a regular non-symlink file");
    }
    if metadata.len() > max_bytes {
        anyhow::bail!("SIZE_LIMIT_EXCEEDED: source is {} bytes", metadata.len());
    }
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; TRANSFER_CHUNK_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > max_bytes {
            anyhow::bail!("SIZE_LIMIT_EXCEEDED: source changed while hashing");
        }
        hasher.update(&buffer[..read]);
    }
    Ok((total, hex::encode(hasher.finalize())))
}

#[cfg(feature = "axon-pb")]
async fn upload(
    session: crate::support::platform::bidi_session::DaemonBidiSession,
    path: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
) -> anyhow::Result<()> {
    let (mut upstream, mut downstream) = session.split();
    let mut file = File::open(path)?;
    let mut buffer = vec![0_u8; TRANSFER_CHUNK_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        upstream.send_binary(buffer[..read].to_vec()).await?;
    }
    upstream.send_eof().await?;
    let complete = receive_completion(&mut downstream, None).await?;
    verify_completion(&complete, expected_bytes, expected_sha256)?;
    println!("copied {expected_bytes} bytes (sha256 {expected_sha256})");
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
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stage_path)?;
    let (_upstream, mut downstream) = session.split();
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let complete = receive_completion(
        &mut downstream,
        Some((&mut file, &mut hasher, &mut total, max_bytes)),
    )
    .await?;
    file.sync_all()?;
    drop(file);
    let sha256 = hex::encode(hasher.finalize());
    verify_completion(&complete, total, &sha256)?;
    let commit = if overwrite {
        std::fs::rename(&stage_path, destination)
    } else {
        std::fs::hard_link(&stage_path, destination)
            .and_then(|()| std::fs::remove_file(&stage_path))
    };
    commit.with_context(|| format!("ATOMIC_COMMIT_FAILED: {}", destination.display()))?;
    stage.committed = true;
    println!("copied {total} bytes (sha256 {sha256})");
    Ok(())
}

#[cfg(feature = "axon-pb")]
async fn receive_completion(
    downstream: &mut crate::support::platform::bidi_session::DaemonBidiReceiver,
    mut download: Option<(&mut File, &mut Sha256, &mut u64, u64)>,
) -> anyhow::Result<Value> {
    let mut complete = None;
    while let Some(frame) = downstream.recv().await? {
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
                complete = apply_transfer_business_frame(terminal_payload, &mut download)?;
            }
        } else if frame.payload.get("type").and_then(Value::as_str) != Some("control") {
            complete = apply_transfer_business_frame(&frame.payload, &mut download)?;
        }
        if frame.terminal {
            break;
        }
    }
    complete.ok_or_else(|| anyhow::anyhow!("TRANSFER_INTERRUPTED: completion frame missing"))
}

fn apply_transfer_business_frame(
    payload: &Value,
    download: &mut Option<(&mut File, &mut Sha256, &mut u64, u64)>,
) -> anyhow::Result<Option<Value>> {
    match payload.get("type").and_then(Value::as_str) {
        Some("chunk") => {
            let data = payload
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("TRANSFER_INTERRUPTED: chunk omitted data"))?;
            let bytes = BASE64_STANDARD
                .decode(data)
                .context("TRANSFER_INTERRUPTED: invalid chunk base64")?;
            write_download_bytes(download, &bytes)?;
            Ok(None)
        }
        Some("binary") => {
            let data = payload
                .get("data_b64")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    anyhow::anyhow!("TRANSFER_INTERRUPTED: binary frame omitted data_b64")
                })?;
            let bytes = BASE64_STANDARD
                .decode(data)
                .context("TRANSFER_INTERRUPTED: invalid binary frame base64")?;
            write_download_bytes(download, &bytes)?;
            Ok(None)
        }
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

fn write_download_bytes(
    download: &mut Option<(&mut File, &mut Sha256, &mut u64, u64)>,
    bytes: &[u8],
) -> anyhow::Result<()> {
    let Some((file, hasher, total, max_bytes)) = download.as_mut() else {
        anyhow::bail!("TRANSFER_INTERRUPTED: upload received a download chunk");
    };
    **total = (**total).saturating_add(bytes.len() as u64);
    if **total > *max_bytes {
        anyhow::bail!("SIZE_LIMIT_EXCEEDED: remote stream exceeded max-bytes");
    }
    file.write_all(bytes)?;
    hasher.update(bytes);
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
        let mut download = None;
        let completion = apply_transfer_business_frame(
            &json!({
                "type": "complete",
                "bytes": 3,
                "sha256": "abc",
            }),
            &mut download,
        )
        .expect("valid terminal receipt payload")
        .expect("completion extracted");

        assert_eq!(completion["bytes"], 3);
        assert_eq!(completion["sha256"], "abc");
    }

    #[test]
    fn upload_rejects_unexpected_download_chunk() {
        let mut download = None;
        let error =
            apply_transfer_business_frame(&json!({"type": "chunk", "data": "YQ=="}), &mut download)
                .expect_err("upload cannot accept a download frame");

        assert!(error
            .to_string()
            .contains("upload received a download chunk"));
    }

    #[test]
    fn download_accepts_lossless_binary_wire_projection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("download.bin");
        let mut file = File::create(&path).expect("create download");
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut download = Some((&mut file, &mut hasher, &mut total, 16));

        let completion = apply_transfer_business_frame(
            &json!({"type": "binary", "stream_id": 1, "data_b64": "AAEC/w=="}),
            &mut download,
        )
        .expect("binary chunk accepted");
        assert!(completion.is_none());
        drop(download);
        drop(file);

        assert_eq!(total, 4);
        assert_eq!(std::fs::read(path).expect("download bytes"), [0, 1, 2, 255]);
    }
}

//! RemoteApp host-E2E fault injection.
//! ==================================
//!
//! This module is compiled only with `remoteapp-e2e-fault-injection` on Unix.
//! It owns no product lifecycle semantics. The lifecycle coordinator calls it
//! only after the canonical terminal recovery snapshot has been promoted and
//! fsynced, but before Closed is installed in memory or returned by RPC.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const ARM_FILE_ENV: &str = "EASYNET_REMOTEAPP_E2E_TERMINAL_PROMOTION_ARM_FILE";
const FAULT_KIND: &str = "crash_after_terminal_promotion";
// Exercise the real product End action. The fault injection may alter the
// process lifecycle, but it must not invent a test-only session-end reason.
const REQUIRED_REASON: &str = "caller_ended";
const MAX_ARM_BYTES: u64 = 4096;
const TARGET_MONITOR_ARM_FILE_ENV: &str = "EASYNET_REMOTEAPP_E2E_TARGET_MONITOR_ARM_FILE";
const TARGET_MONITOR_FAULT_KIND: &str = "crash_target_monitor_generation";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct TerminalPromotionCrashArm {
    schema_version: u32,
    fault: String,
    session_id: String,
    reason: String,
    marker_path: PathBuf,
    nonce: String,
    armed_at_ms: u64,
}

#[derive(Debug, Serialize)]
struct TerminalPromotionCrashMarker<'a> {
    schema_version: u32,
    fault: &'static str,
    session_id: &'a str,
    reason: &'a str,
    pid: u32,
    arm_nonce: &'a str,
    promoted_at_ms: u64,
    terminal_receipt: &'a Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct TargetMonitorCrashArm {
    schema_version: u32,
    fault: String,
    session_id: String,
    marker_path: PathBuf,
    nonce: String,
    armed_at_ms: u64,
}

#[derive(Debug, Serialize)]
struct TargetMonitorCrashMarker<'a> {
    schema_version: u32,
    fault: &'static str,
    session_id: &'a str,
    pid: u32,
    generation: u64,
    arm_nonce: &'a str,
    crashed_at_ms: u64,
}

pub(in crate::daemon::plugins::remote_desktop) fn maybe_crash_after_terminal_promotion(
    session_id: &str,
    reason: &str,
    terminal_receipt: &Value,
) {
    let arm_path = match std::env::var_os(ARM_FILE_ENV) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => return,
    };
    let arm = match claim_matching_arm(&arm_path, session_id, reason) {
        Ok(Some(arm)) => arm,
        Ok(None) => return,
        Err(error) => {
            eprintln!("[remote-desktop] ignored invalid terminal-promotion E2E arm: {error:#}");
            return;
        }
    };
    if terminal_receipt["session_id"] != Value::String(session_id.to_string())
        || terminal_receipt["reason_code"] != Value::String(reason.to_string())
        || terminal_receipt["terminal"] != Value::Bool(true)
        || !terminal_receipt["terminal_event_id"].is_string()
        || terminal_receipt["terminal_event_sequence"]
            .as_u64()
            .unwrap_or(0)
            == 0
    {
        eprintln!(
            "[remote-desktop] terminal-promotion E2E arm claimed but canonical terminal receipt was incomplete; refusing crash"
        );
        return;
    }
    let marker = TerminalPromotionCrashMarker {
        schema_version: 1,
        fault: FAULT_KIND,
        session_id,
        reason,
        pid: std::process::id(),
        arm_nonce: &arm.nonce,
        promoted_at_ms: crate::daemon::plugins::remote_desktop::session::now_ms(),
        terminal_receipt,
    };
    if let Err(error) = write_json_atomically(&arm.marker_path, &marker) {
        eprintln!(
            "[remote-desktop] failed to persist terminal-promotion E2E crash marker; refusing crash: {error:#}"
        );
        return;
    }
    eprintln!(
        "[remote-desktop] E2E fault: terminal snapshot promoted for {session_id}; sending SIGKILL before in-memory publication"
    );
    let pid = std::process::id() as libc::pid_t;
    let result = unsafe { libc::kill(pid, libc::SIGKILL) };
    if result == -1 {
        eprintln!(
            "[remote-desktop] E2E SIGKILL failed after durable marker: {}",
            std::io::Error::last_os_error()
        );
    }
    std::process::abort();
}

pub(in crate::daemon::plugins::remote_desktop) fn maybe_crash_target_monitor_generation(
    generation: u64,
    tracked_session_ids: &std::collections::HashSet<String>,
) {
    let arm_path = match std::env::var_os(TARGET_MONITOR_ARM_FILE_ENV) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => return,
    };
    if !arm_path.exists() {
        return;
    }
    let arm = match claim_target_monitor_arm(&arm_path, tracked_session_ids) {
        Ok(Some(arm)) => arm,
        Ok(None) => return,
        Err(error) => {
            eprintln!("[remote-desktop] ignored invalid target-monitor E2E arm: {error:#}");
            return;
        }
    };
    let marker = TargetMonitorCrashMarker {
        schema_version: 1,
        fault: TARGET_MONITOR_FAULT_KIND,
        session_id: &arm.session_id,
        pid: std::process::id(),
        generation,
        arm_nonce: &arm.nonce,
        crashed_at_ms: crate::daemon::plugins::remote_desktop::session::now_ms(),
    };
    if let Err(error) = write_json_atomically(&arm.marker_path, &marker) {
        eprintln!(
            "[remote-desktop] failed to persist target-monitor E2E crash marker; refusing worker crash: {error:#}"
        );
        return;
    }
    panic!(
        "E2E fault: target monitor generation {generation} crashed for {}",
        arm.session_id
    );
}

fn claim_matching_arm(
    arm_path: &Path,
    session_id: &str,
    reason: &str,
) -> anyhow::Result<Option<TerminalPromotionCrashArm>> {
    claim_private_arm(arm_path, |arm: &TerminalPromotionCrashArm| {
        validate_arm(arm, session_id, reason)
    })
    .map(Some)
}

fn claim_target_monitor_arm(
    arm_path: &Path,
    tracked_session_ids: &std::collections::HashSet<String>,
) -> anyhow::Result<Option<TargetMonitorCrashArm>> {
    claim_private_arm(arm_path, |arm: &TargetMonitorCrashArm| {
        if arm.schema_version != 1 {
            anyhow::bail!("unsupported arm schema {}", arm.schema_version);
        }
        if arm.fault != TARGET_MONITOR_FAULT_KIND {
            anyhow::bail!("unexpected fault kind {}", arm.fault);
        }
        if !tracked_session_ids.contains(&arm.session_id) {
            anyhow::bail!("target-monitor arm session is not tracked by this generation");
        }
        if arm.nonce.trim().is_empty() || arm.armed_at_ms == 0 {
            anyhow::bail!("arm nonce and timestamp must be present");
        }
        require_safe_absolute_output_path(&arm.marker_path, "marker_path")
    })
    .map(Some)
}

fn claim_private_arm<T>(
    arm_path: &Path,
    validate: impl FnOnce(&T) -> anyhow::Result<()>,
) -> anyhow::Result<T>
where
    T: DeserializeOwned,
{
    require_private_absolute_file(arm_path)?;
    let bytes = fs::read(arm_path)?;
    let arm: T = serde_json::from_slice(&bytes)?;
    validate(&arm)?;

    let claimed_path = arm_path.with_extension(format!("claimed.{}", std::process::id()));
    if claimed_path.exists() {
        anyhow::bail!(
            "claimed arm path already exists: {}",
            claimed_path.display()
        );
    }
    fs::rename(arm_path, &claimed_path)?;
    let claimed_bytes = fs::read(&claimed_path)?;
    if claimed_bytes != bytes {
        anyhow::bail!("E2E arm changed while being claimed");
    }
    sync_parent(&claimed_path)?;
    Ok(arm)
}

fn validate_arm(
    arm: &TerminalPromotionCrashArm,
    session_id: &str,
    reason: &str,
) -> anyhow::Result<()> {
    if arm.schema_version != 1 {
        anyhow::bail!("unsupported arm schema {}", arm.schema_version);
    }
    if arm.fault != FAULT_KIND {
        anyhow::bail!("unexpected fault kind {}", arm.fault);
    }
    if arm.session_id != session_id || arm.reason != reason {
        anyhow::bail!("arm session/reason does not match promoted terminal candidate");
    }
    if reason != REQUIRED_REASON {
        anyhow::bail!("terminal-promotion fault requires reason {REQUIRED_REASON}");
    }
    if arm.nonce.trim().is_empty() || arm.armed_at_ms == 0 {
        anyhow::bail!("arm nonce and timestamp must be present");
    }
    require_safe_absolute_output_path(&arm.marker_path, "marker_path")?;
    Ok(())
}

fn require_private_absolute_file(path: &Path) -> anyhow::Result<()> {
    require_safe_absolute_output_path(path, "arm file")?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!("arm path must be a regular non-symlink file");
    }
    if metadata.len() == 0 || metadata.len() > MAX_ARM_BYTES {
        anyhow::bail!("arm file size must be within 1..={MAX_ARM_BYTES} bytes");
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        anyhow::bail!("arm file must be owned by the daemon user");
    }
    if metadata.mode() & 0o077 != 0 {
        anyhow::bail!("arm file must not grant group/other permissions");
    }
    Ok(())
}

fn require_safe_absolute_output_path(path: &Path, label: &str) -> anyhow::Result<()> {
    if !path.is_absolute() || path.parent().is_none() || path == Path::new("/") {
        anyhow::bail!("{label} must be a bounded absolute file path");
    }
    if std::env::var_os("HOME").is_some_and(|home| path == Path::new(&home)) {
        anyhow::bail!("{label} must not be the user home directory");
    }
    Ok(())
}

fn write_json_atomically(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    require_safe_absolute_output_path(path, "marker_path")?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("marker path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temp = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options.open(&temp)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent"))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_arm(path: &Path, marker_path: PathBuf, session_id: &str) {
        let arm = TerminalPromotionCrashArm {
            schema_version: 1,
            fault: FAULT_KIND.to_string(),
            session_id: session_id.to_string(),
            reason: REQUIRED_REASON.to_string(),
            marker_path,
            nonce: "0123456789abcdef".to_string(),
            armed_at_ms: 1,
        };
        fs::write(path, serde_json::to_vec(&arm).unwrap()).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn write_target_monitor_arm(path: &Path, marker_path: PathBuf, session_id: &str) {
        let arm = TargetMonitorCrashArm {
            schema_version: 1,
            fault: TARGET_MONITOR_FAULT_KIND.to_string(),
            session_id: session_id.to_string(),
            marker_path,
            nonce: "fedcba9876543210".to_string(),
            armed_at_ms: 1,
        };
        fs::write(path, serde_json::to_vec(&arm).unwrap()).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn owner_only_arm_is_claimed_once_for_exact_session_and_reason() {
        let temp = tempfile::tempdir().unwrap();
        let arm_path = temp.path().join("arm.json");
        let marker_path = temp.path().join("marker.json");
        write_arm(&arm_path, marker_path.clone(), "rd-crash");

        let arm = claim_matching_arm(&arm_path, "rd-crash", REQUIRED_REASON)
            .unwrap()
            .expect("matching arm claims");

        assert_eq!(arm.marker_path, marker_path);
        assert!(!arm_path.exists());
        assert!(temp
            .path()
            .join(format!("arm.claimed.{}", std::process::id()))
            .exists());
    }

    #[test]
    fn group_readable_arm_is_rejected_before_claim() {
        let temp = tempfile::tempdir().unwrap();
        let arm_path = temp.path().join("arm.json");
        write_arm(&arm_path, temp.path().join("marker.json"), "rd-crash");
        fs::set_permissions(&arm_path, fs::Permissions::from_mode(0o640)).unwrap();

        let error = claim_matching_arm(&arm_path, "rd-crash", REQUIRED_REASON)
            .expect_err("non-private arm must fail");

        assert!(error.to_string().contains("group/other"));
        assert!(arm_path.exists());
    }

    #[test]
    fn marker_write_is_private_and_contains_canonical_terminal_identity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("marker.json");
        let receipt = serde_json::json!({
            "session_id": "rd-crash",
            "reason_code": REQUIRED_REASON,
            "terminal": true,
            "terminal_event_id": "rd-crash:7",
            "terminal_event_sequence": 7,
        });
        let marker = TerminalPromotionCrashMarker {
            schema_version: 1,
            fault: FAULT_KIND,
            session_id: "rd-crash",
            reason: REQUIRED_REASON,
            pid: 42,
            arm_nonce: "nonce",
            promoted_at_ms: 3,
            terminal_receipt: &receipt,
        };
        write_json_atomically(&path, &marker).unwrap();
        let stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            stored["terminal_receipt"]["terminal_event_id"],
            "rd-crash:7"
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn target_monitor_arm_claims_only_for_an_exact_tracked_session() {
        let temp = tempfile::tempdir().unwrap();
        let arm_path = temp.path().join("target-arm.json");
        write_target_monitor_arm(
            &arm_path,
            temp.path().join("target-marker.json"),
            "rd-target-monitor",
        );
        let wrong = std::collections::HashSet::from(["rd-other".to_string()]);

        let error = claim_target_monitor_arm(&arm_path, &wrong)
            .expect_err("an unrelated generation must not claim the arm");
        assert!(error.to_string().contains("not tracked"));
        assert!(arm_path.exists());

        let tracked = std::collections::HashSet::from(["rd-target-monitor".to_string()]);
        let arm = claim_target_monitor_arm(&arm_path, &tracked)
            .unwrap()
            .expect("the exact tracked session claims once");
        assert_eq!(arm.session_id, "rd-target-monitor");
        assert!(!arm_path.exists());
    }

    #[test]
    fn target_monitor_marker_is_private_and_binds_generation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("target-marker.json");
        let marker = TargetMonitorCrashMarker {
            schema_version: 1,
            fault: TARGET_MONITOR_FAULT_KIND,
            session_id: "rd-target-monitor",
            pid: 42,
            generation: 7,
            arm_nonce: "nonce",
            crashed_at_ms: 9,
        };
        write_json_atomically(&path, &marker).unwrap();
        let stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored["generation"], 7);
        assert_eq!(stored["session_id"], "rd-target-monitor");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

// EasyNet CLI — daemon clipboard tracker
// =======================================
//
// File: src/daemon/context/clipboard_tracker.rs
// Description: Opt-in clipboard history capture for the Context
//              surface. A dedicated OS thread polls the system
//              clipboard; on change it appends a `ClipEntry`
//              (timestamp + this device's URA + text or PNG) to
//              `persistence::context_store`.
//
// Opt-in contract
// ---------------
// Tracking is OFF by default. `easynet context clipboard on|off`
// (or the `context.clipboard.track` ability) flips
// `context/config.json`; the tracker re-reads that file every tick,
// so toggles take effect within one poll interval without IPC or a
// daemon restart. While disabled the thread only sleeps + stats the
// config — no clipboard access happens at all.
//
// Capture mechanics (macOS)
// -------------------------
// * text  — `pbpaste` (empty stdout = no text on the pasteboard).
// * image — `osascript -e 'clipboard info'` to detect a `PNGf`
//           flavour (screenshots land as PNGf with no text), then an
//           osascript `write (the clipboard as «class PNGf»)` to a
//           file under `context/clips/`.
//
// Non-macOS hosts: `pbpaste` is absent, the probe returns None every
// tick and the tracker idles harmlessly. Linux/Windows probes can be
// added behind the same `read_clipboard` seam later.
//
// Dedup: a hash of the captured payload is kept in thread memory;
// a poll that sees the same hash as the previous capture is a no-op,
// so holding one clipboard item for minutes produces one entry.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::daemon::persistence::context_store::{self, ClipEntry};

/// Poll interval while tracking is enabled.
const TICK: Duration = Duration::from_millis(1500);
/// Poll interval while tracking is disabled (config stat only).
const IDLE_TICK: Duration = Duration::from_secs(3);
/// Text clips above this many bytes are stored truncated. Clipboard
/// managers cap similarly; a 10 MB accidental copy should not bloat
/// the JSONL forever.
const TEXT_CAP_BYTES: usize = 64 * 1024;
const PREVIEW_CHARS: usize = 120;

/// What one poll saw on the pasteboard.
enum Captured {
    Text(String),
    /// Raw PNG bytes.
    Png(Vec<u8>),
}

/// Spawn the tracker thread. Called once from daemon boot; never
/// fails — a spawn error is logged and the daemon continues without
/// clipboard capture. The capturing device's URA is resolved here
/// (not passed in) because hosted identity projection belongs to the
/// daemon persistence aggregate, not the daemon binary surface.
pub fn spawn() {
    let device_ura =
        crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_hosted_identity_status()
            .ok()
            .and_then(|status| status.host_device_agent_ura().map(str::to_string))
            .unwrap_or_default();
    if let Err(e) = std::thread::Builder::new()
        .name("clipboard-tracker".into())
        .spawn(move || run_loop(&device_ura))
    {
        eprintln!("[clipboard-tracker] failed to spawn: {e}");
    }
}

fn run_loop(device_ura: &str) {
    let mut last_hash: Option<[u8; 32]> = None;
    loop {
        let tracking_enabled = match context_store::clipboard_tracking() {
            Ok(enabled) => enabled,
            Err(error) => {
                eprintln!("[clipboard-tracker] tracking config unavailable: {error:#}");
                false
            }
        };
        if !tracking_enabled {
            std::thread::sleep(IDLE_TICK);
            continue;
        }
        match read_clipboard() {
            Some(Captured::Text(text)) => {
                let hash = Sha256::digest(format!("t:{text}").as_bytes()).into();
                if last_hash != Some(hash) {
                    last_hash = Some(hash);
                    if let Err(e) = record_text(device_ura, &text) {
                        eprintln!("[clipboard-tracker] persist text failed: {e}");
                    }
                }
            }
            Some(Captured::Png(bytes)) => {
                let hash = Sha256::digest(&bytes).into();
                if last_hash != Some(hash) {
                    last_hash = Some(hash);
                    if let Err(e) = record_png(device_ura, &bytes) {
                        eprintln!("[clipboard-tracker] persist image failed: {e}");
                    }
                }
            }
            None => {}
        }
        std::thread::sleep(TICK);
    }
}

fn record_text(device_ura: &str, text: &str) -> anyhow::Result<()> {
    let stored: String = if text.len() > TEXT_CAP_BYTES {
        let mut s: String = text.chars().take(TEXT_CAP_BYTES / 4).collect();
        s.push('…');
        s
    } else {
        text.to_string()
    };
    let entry = ClipEntry {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        device: device_ura.to_string(),
        kind: "text".into(),
        preview: make_preview(&stored),
        text: Some(stored),
        image_file: None,
    };
    context_store::append_clip(&entry)
}

fn record_png(device_ura: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let file = format!("{id}.png");
    std::fs::create_dir_all(context_store::clips_dir())?;
    std::fs::write(context_store::clips_dir().join(&file), bytes)?;
    let entry = ClipEntry {
        id,
        timestamp: chrono::Utc::now().to_rfc3339(),
        device: device_ura.to_string(),
        kind: "image".into(),
        preview: format!("Image ({} KB)", bytes.len() / 1024),
        text: None,
        image_file: Some(file),
    };
    context_store::append_clip(&entry)
}

fn make_preview(text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    let cut: String = first.chars().take(PREVIEW_CHARS).collect();
    if cut.len() < first.len() {
        format!("{cut}…")
    } else {
        cut
    }
}

/// One pasteboard probe. Text wins when both text and an image
/// flavour are present (copying rich content usually carries both;
/// the text is the useful half). Screenshots are PNGf-only.
fn read_clipboard() -> Option<Captured> {
    if let Some(text) = pbpaste_text() {
        if !text.trim().is_empty() {
            return Some(Captured::Text(text));
        }
    }
    if clipboard_has_png() {
        if let Some(bytes) = clipboard_png_bytes() {
            return Some(Captured::Png(bytes));
        }
    }
    None
}

fn pbpaste_text() -> Option<String> {
    let out = Command::new("pbpaste").output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn clipboard_has_png() -> bool {
    let Ok(out) = Command::new("osascript")
        .args(["-e", "clipboard info"])
        .output()
    else {
        return false;
    };
    out.status.success() && String::from_utf8_lossy(&out.stdout).contains("PNGf")
}

/// Write the pasteboard PNG to a temp file via osascript, read it
/// back, remove the temp. AppleScript is the only stock-macOS way to
/// extract image flavours without linking AppKit.
fn clipboard_png_bytes() -> Option<Vec<u8>> {
    let tmp = std::env::temp_dir().join(format!("easynet-clip-{}.png", std::process::id()));
    let tmp_str = tmp.to_string_lossy().to_string();
    let script = format!(
        "set f to open for access POSIX file \"{tmp_str}\" with write permission\n\
         try\n\
             set eof of f to 0\n\
             write (the clipboard as «class PNGf») to f\n\
         end try\n\
         close access f",
    );
    let ok = Command::new("osascript")
        .args(["-e", &script])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let bytes = if ok { std::fs::read(&tmp).ok() } else { None };
    let _ = std::fs::remove_file(&tmp);
    bytes.filter(|b| !b.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_takes_first_line_and_caps_length() {
        assert_eq!(make_preview("hello\nworld"), "hello");
        let long = "x".repeat(500);
        let p = make_preview(&long);
        assert!(p.chars().count() <= PREVIEW_CHARS + 1);
        assert!(p.ends_with('…'));
    }

    #[test]
    fn record_text_truncates_oversized_clips() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let huge = "y".repeat(TEXT_CAP_BYTES + 10);
        record_text("easynet:///r/localhost/device/d1", &huge).unwrap();
        let clips = crate::daemon::persistence::context_store::list_clips(1).unwrap();
        assert_eq!(clips.len(), 1);
        let stored = clips[0].text.as_deref().unwrap();
        assert!(stored.len() < TEXT_CAP_BYTES);
        assert!(stored.ends_with('…'));
    }
}

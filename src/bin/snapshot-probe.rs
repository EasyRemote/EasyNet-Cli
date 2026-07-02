// Operator-only probe that exercises the same production
// `SnapshotBackend::capture_jpeg` path the `camera.snapshot` ability uses.
// Skips the envelope/subject layer so the operator can verify camera
// permission + JPEG encode end-to-end without wiring a registered resource.
//
// Usage: cargo run --bin snapshot-probe -- /tmp/probe.jpg

use anyhow::Context;
use serde_json::Value;
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: snapshot-probe <output-jpg>")?;

    let entry = easynet_cli::persistence::resources::ResourceEntry {
        resource_ura: "easynet:///r/probe/resource/probe-camera".to_string(),
        owner_agent: String::new(),
        kind: easynet_cli::persistence::resources::ResourceType::Camera,
        binding: easynet_cli::persistence::resources::ResourceBinding::LocalDevice,
        hardware_id: "default-0".to_string(),
        display_name: "default camera".to_string(),
        metadata: Value::Null,
        first_seen_at: chrono::Utc::now().to_rfc3339(),
    };

    let backend: Arc<dyn easynet_cli::daemon::ability::builtins::resources::media::camera_snapshot::SnapshotBackend> =
        Arc::new(easynet_cli::daemon::ability::builtins::resources::media::camera_snapshot::NokhwaBackend);

    eprintln!("opening default camera (index 0)...");
    let frame = backend
        .capture_jpeg(&entry)
        .context("production camera capture failed")?;

    std::fs::write(&path, &frame.jpeg_bytes)?;
    eprintln!(
        "wrote {} ({} bytes, {}x{})",
        path,
        frame.jpeg_bytes.len(),
        frame.width,
        frame.height
    );
    Ok(())
}

// EasyNet CLI — `easynet ability deploy`
// =======================================
//
// File: src/cli/deploy.rs
// Description: Deploy an ability bundle to a canonical host Device while the
//              resulting public descriptor is owned by that Device's
//              ability-management SystemAgent.
//
// Per the ability-only ontology, deploying an ability is itself an
// ability invocation: the paired User Principal invokes `ability.deploy`
// through the local daemon, passing a short-lived filesystem ResourceRef and
// target Device URA. The target Device is the execution host/custody boundary
// only; descriptor owner/callee is the device-sponsored ability-management
// SystemAgent. The daemon-side handler reads the bundle,
// validates the manifest, computes the integrity digest, and publishes through
// the canonical ability deployment registrar. Local deploy passes a local
// directory ResourceRef. Remote deploy first stages a tar.gz bundle into the
// target Device's local tmp ResourceRef through canonical `fs.transfer`, then
// invokes the target Device's `ability-management` SystemAgent.
//
// What this CLI shim does
// -----------------------
//   1. Validate args locally (path exists, target resolves to a Device URA).
//   2. Select local or remote target route from canonical Device URAs.
//   3. Resolve the paired User Principal that carries deployment accountability.
//   4. Local: mint a local ResourceRef and invoke local `ability.deploy`.
//   5. Remote: upload a staged tar.gz ResourceRef to the target Device, then
//      invoke target-owned `ability.deploy` with the staged ResourceRef as
//      subject.
//   6. Print the daemon's response.
//
// All policy (manifest validation, signature handling, ordering of
// publish→install→activate) lives inside the ability handler.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use console::style;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::daemon::resources::files::{FilesystemResourceCapability, FilesystemResourceProvider};
use crate::support::platform::output::{self, OutputFormat};

#[derive(Debug, Args)]
pub struct DeployArgs {
    /// Path to the ability directory (must contain `ability.json`).
    /// The CLI converts it to a short-lived ResourceRef before invocation.
    /// Current device deployment accepts executable manifests whose `[exec]`
    /// binding is `kind = "host_stream"`; other exec kinds are rejected by
    /// the daemon until their runtime boundaries are implemented.
    pub path: String,
    /// Target device. Defaults to 'local', which resolves to this daemon's
    /// canonical Device URA before daemon invocation. Remote targets must be
    /// canonical Device URAs; bare node ids are rejected before mutation.
    #[arg(
        long = "node",
        short = 'n',
        value_name = "DEVICE_URA",
        default_value = "local"
    )]
    pub node: String,
    /// Output format. `json` prints the daemon's deploy result to stdout
    /// so scripts can consume ability_ura, install_id, and state directly.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

pub fn run(args: DeployArgs) -> anyhow::Result<()> {
    let dir = Path::new(&args.path);
    anyhow::ensure!(dir.is_dir(), "{} is not a directory", args.path);
    anyhow::ensure!(
        !args.node.trim().is_empty(),
        "--node was given but empty; pass `local` for this device or a canonical Device URA"
    );
    let target_ura = crate::support::platform::remote_device::resolve_cli_device_target_ura(
        Some(&args.node),
        "ability deploy",
    )?;
    let invocation =
        crate::support::platform::remote_device::PairedInvocationIdentity::load("ability deploy")?;

    eprint!(
        "  deploying {} to {} ... ",
        style(&args.path).cyan(),
        style(&target_ura).cyan()
    );
    let result = match DeployTargetRoute::resolve(&target_ura, invocation.local_device_ura()) {
        DeployTargetRoute::Local { target_ura } => {
            invoke_local_ability_deploy(dir, &target_ura, invocation.caller_user_ura())?
        }
        DeployTargetRoute::Remote { target_ura } => {
            invoke_remote_ability_deploy(dir, &target_ura, invocation.caller_user_ura())?
        }
    };
    eprintln!("{}", style("✓").green());

    render_deploy_result(args.format, &args.node, &result)
}

enum DeployTargetRoute {
    Local { target_ura: String },
    Remote { target_ura: String },
}

impl DeployTargetRoute {
    fn resolve(target_ura: &str, local_device_ura: &str) -> Self {
        if target_ura == local_device_ura {
            Self::Local {
                target_ura: target_ura.to_string(),
            }
        } else {
            Self::Remote {
                target_ura: target_ura.to_string(),
            }
        }
    }
}

fn invoke_local_ability_deploy(
    dir: &Path,
    target_ura: &str,
    caller_user_ura: &str,
) -> anyhow::Result<serde_json::Value> {
    let filesystem = FilesystemResourceProvider::for_device(target_ura.to_string())
        .context("construct target Device filesystem provider")?;
    let resource_ref = filesystem
        .resource_ref_for_local_path(dir, FilesystemResourceCapability::Read)
        .context("mint ability bundle ResourceRef")?;
    let subject_ura = resource_ref
        .get("resource_ura")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("minted ResourceRef did not include resource_ura"))?;
    crate::cli::daemon_client::remote_system_ability::invoke_target_ability_deploy_from_resource(
        target_ura,
        caller_user_ura,
        &subject_ura,
        resource_ref,
    )
    .context("invoke local ability.deploy")
}

#[cfg(feature = "axon-pb")]
fn invoke_remote_ability_deploy(
    dir: &Path,
    target_ura: &str,
    caller_user_ura: &str,
) -> anyhow::Result<serde_json::Value> {
    let archive = AbilityDeployArchive::from_bundle_dir(dir)?;
    let staging_relative_path = format!(
        "easynet-ability-deploy/{}.tar.gz",
        uuid::Uuid::new_v4().simple()
    );
    let upload_ref = crate::daemon::resources::files::resource_ref_for_target_tmp_relative_path(
        &staging_relative_path,
        FilesystemResourceCapability::Write,
        target_ura,
    )
    .context("mint target tmp ResourceRef for remote ability deploy staging")?;
    let subject_ura = resource_ura(&upload_ref)?;

    let transfer_frames =
        crate::cli::daemon_client::remote_system_ability::upload_target_resource_via_file_transfer(
            target_ura,
            caller_user_ura,
            &subject_ura,
            upload_ref.clone(),
            archive.into_upload_chunks()?,
        )
        .context("upload ability bundle archive to remote target tmp")?;
    ensure_remote_upload_completed(&transfer_frames)?;

    crate::cli::daemon_client::remote_system_ability::invoke_target_ability_deploy_from_resource(
        target_ura,
        caller_user_ura,
        &subject_ura,
        upload_ref,
    )
    .context("invoke remote ability.deploy")
}

#[cfg(not(feature = "axon-pb"))]
fn invoke_remote_ability_deploy(
    _dir: &Path,
    target_ura: &str,
    _caller_user_ura: &str,
) -> anyhow::Result<serde_json::Value> {
    Err(
        crate::support::platform::local_invoke::federation_capability_unsupported_error(&format!(
            "deploying ability bundle to remote device target {target_ura:?}"
        )),
    )
}

fn resource_ura(resource_ref: &serde_json::Value) -> anyhow::Result<String> {
    resource_ref
        .get("resource_ura")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("ResourceRef did not include resource_ura"))
}

struct AbilityDeployArchive {
    path: PathBuf,
}

impl AbilityDeployArchive {
    fn from_bundle_dir(dir: &Path) -> anyhow::Result<Self> {
        let manifest_path = dir.join("ability.json");
        let manifest = std::fs::read(&manifest_path)
            .with_context(|| format!("read ability manifest at {}", manifest_path.display()))?;
        let archive_path = std::env::temp_dir().join(format!(
            "easynet-ability-deploy-{}.tar.gz",
            uuid::Uuid::new_v4().simple()
        ));
        {
            let file = std::fs::File::create(&archive_path)
                .with_context(|| format!("create {}", archive_path.display()))?;
            let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(manifest.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, "ability.json", &manifest[..])
                .context("write root ability.json into deploy archive")?;
            archive.finish().context("finish deploy archive")?;
        }
        Ok(Self { path: archive_path })
    }

    fn into_upload_chunks(self) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut bytes = Vec::new();
        std::fs::File::open(&self.path)
            .and_then(|mut file| file.read_to_end(&mut bytes))
            .with_context(|| format!("read deploy archive {}", self.path.display()))?;
        let _ = std::fs::remove_file(&self.path);
        Ok(bytes.chunks(64 * 1024).map(<[u8]>::to_vec).collect())
    }
}

fn ensure_remote_upload_completed(
    frames: &[crate::support::platform::local_invoke::LocalBidiFrame],
) -> anyhow::Result<()> {
    for frame in frames {
        if frame
            .payload
            .get("type")
            .and_then(serde_json::Value::as_str)
            == Some("error")
        {
            anyhow::bail!("remote ability bundle upload failed: {}", frame.payload);
        }
        if frame
            .payload
            .get("failure")
            .and_then(|failure| failure.get("message"))
            .and_then(serde_json::Value::as_str)
            .is_some()
        {
            anyhow::bail!("remote ability bundle upload failed: {}", frame.payload);
        }
    }
    if frames.iter().any(upload_frame_is_complete) {
        return Ok(());
    }
    anyhow::bail!("remote ability bundle upload did not emit a complete frame")
}

fn upload_frame_is_complete(
    frame: &crate::support::platform::local_invoke::LocalBidiFrame,
) -> bool {
    if frame
        .payload
        .get("type")
        .and_then(serde_json::Value::as_str)
        == Some("complete")
    {
        return true;
    }
    frame
        .payload
        .get("payload")
        .and_then(|payload| payload.get("type"))
        .and_then(serde_json::Value::as_str)
        == Some("complete")
}

fn render_deploy_result(
    format: OutputFormat,
    requested_node: &str,
    result: &serde_json::Value,
) -> anyhow::Result<()> {
    if format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if let Some(install_id) = result.get("install_id").and_then(|v| v.as_str()) {
        output::step(&format!("install_id: {install_id}"));
    }
    match (
        result.get("state").and_then(|v| v.as_str()),
        result.get("ability_ura").and_then(|v| v.as_str()),
    ) {
        (Some("ACTIVE"), Some(ability_ura)) => {
            output::success(&format!("{ability_ura} is active on {requested_node}"));
        }
        (Some("INSTALLED"), Some(ability_ura)) => {
            output::step(&format!(
                "{ability_ura} installed on {requested_node}; activation is pending route availability"
            ));
        }
        _ => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use serde_json::json;

    use super::{AbilityDeployArchive, DeployTargetRoute, OutputFormat};
    use crate::cli::commands::groups::ability::AbilityAction;
    use crate::cli::{App, Command};

    fn provision_local_device_credentials() -> crate::cli::commands::test_support::HomeGuard {
        let guard = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "local".to_string(),
                credential_token: "token".to_string(),
                hub_endpoint: "axon://hub.example:50051".to_string(),
                realm: "test".to_string(),
                username: Some("alice".to_string()),
                user_id: Some("user-alice".to_string()),
                ..Default::default()
            },
        )
        .expect("write deploy test credentials");
        guard
    }

    #[test]
    fn deploy_json_format_is_a_stable_machine_surface() {
        let app = App::parse_from([
            "easynet",
            "ability",
            "deploy",
            "/tmp/native-ability",
            "--node",
            "local",
            "--format",
            "json",
        ]);

        match app.command {
            Command::Ability(args) => match args.action {
                AbilityAction::Deploy(deploy) => {
                    assert_eq!(deploy.path, "/tmp/native-ability");
                    assert_eq!(deploy.node, "local");
                    assert_eq!(deploy.format, OutputFormat::Json);
                }
                other => panic!("expected ability deploy, got {other:?}"),
            },
            other => panic!("expected ability command, got {other:?}"),
        }
    }

    #[test]
    fn deploy_target_ura_rejects_bare_node_id_before_daemon_payload() {
        let err = crate::support::platform::remote_device::resolve_cli_device_target_ura(
            Some("node-a"),
            "ability deploy",
        )
        .expect_err("bare node id must not be accepted");
        let message = err.to_string();
        assert!(
            message.contains("canonical URA") || message.contains("Device URA"),
            "unexpected target error: {message}"
        );
    }

    #[test]
    fn deploy_route_selects_local_only_for_current_device_ura() {
        let _home = provision_local_device_credentials();

        match DeployTargetRoute::resolve(
            "easynet:///r/test/device/local",
            "easynet:///r/test/device/local",
        ) {
            DeployTargetRoute::Local { target_ura } => {
                assert_eq!(target_ura, "easynet:///r/test/device/local");
            }
            DeployTargetRoute::Remote { target_ura } => {
                panic!("current device must not route through remote deploy: {target_ura}");
            }
        }
    }

    #[test]
    fn deploy_route_selects_remote_for_peer_device_ura() {
        let _home = provision_local_device_credentials();

        match DeployTargetRoute::resolve(
            "easynet:///r/test/device/peer",
            "easynet:///r/test/device/local",
        ) {
            DeployTargetRoute::Remote { target_ura } => {
                assert_eq!(target_ura, "easynet:///r/test/device/peer");
            }
            DeployTargetRoute::Local { target_ura } => {
                panic!("peer device must route through remote deploy: {target_ura}");
            }
        }
    }

    #[test]
    fn deploy_invocation_separates_user_caller_from_device_execution_host() {
        let _home = provision_local_device_credentials();

        let context = crate::support::platform::remote_device::PairedInvocationIdentity::load(
            "ability deploy",
        )
        .unwrap();

        assert_eq!(
            context.caller_user_ura(),
            "easynet:///r/test/user/user-alice"
        );
        assert_eq!(context.local_device_ura(), "easynet:///r/test/device/local");
    }

    #[test]
    fn deploy_invocation_rejects_device_only_credentials() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "local".to_string(),
                hub_endpoint: "axon://hub.example:50051".to_string(),
                realm: "test".to_string(),
                join_receipt_hash: Some("join-receipt".to_string()),
                ..Default::default()
            },
        )
        .expect("write device-only deploy test credentials");

        let error = crate::support::platform::remote_device::PairedInvocationIdentity::load(
            "ability deploy",
        )
        .expect_err("device-only credentials cannot provide accountable deploy caller");

        assert!(
            error.to_string().contains("User Principal caller"),
            "{error}"
        );
    }

    #[test]
    fn deploy_archive_emits_raw_file_transfer_chunks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ability.json"),
            r#"{"schema_version":"1","name":"weather","namespace":"er","description":"w",
                "input_schema":{"type":"object"},
                "exec":{"kind":"host_stream","host_socket":"/tmp/er-host.sock","function":"er.weather"}}"#,
        )
        .unwrap();

        let chunks = AbilityDeployArchive::from_bundle_dir(dir.path())
            .unwrap()
            .into_upload_chunks()
            .unwrap();

        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|chunk| !chunk.is_empty()));
    }

    #[test]
    fn remote_upload_completion_rejects_error_frame() {
        let frames = vec![crate::support::platform::local_invoke::LocalBidiFrame {
            sequence: 0,
            content_type: "application/json".to_string(),
            terminal: false,
            payload: json!({"type": "error", "message": "denied"}),
        }];

        let err = super::ensure_remote_upload_completed(&frames)
            .expect_err("error frame must fail upload completion");

        assert!(err
            .to_string()
            .contains("remote ability bundle upload failed"));
    }

    #[test]
    fn remote_upload_completion_accepts_canonical_receipt_payload() {
        let frames = vec![crate::support::platform::local_invoke::LocalBidiFrame {
            sequence: 2,
            content_type: "application/octet-stream".to_string(),
            terminal: true,
            payload: json!({
                "type": "receipt",
                "state": 4,
                "payload": {
                    "type": "complete",
                    "bytes": 3,
                    "sha256": "abc"
                }
            }),
        }];

        super::ensure_remote_upload_completed(&frames)
            .expect("canonical terminal receipt payload completes remote upload");
    }
}

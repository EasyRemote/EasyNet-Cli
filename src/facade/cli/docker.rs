// EasyNet CLI — Docker operator facade
// ====================================
//
// File: src/facade/cli/docker.rs
// Description: Container-oriented diagnostics for EasyNet deployments.
//
// Protocol Responsibility:
// - Exposes the same join-to-connected state snapshot used by
//   `runtime status` and `doctor`. Docker is an execution environment, not a
//   separate product state machine.
//
// Implementation Approach:
// - Thin facade over existing status/doctor commands. This keeps state codes,
//   transitions, and failure reasons single-sourced in
//   runtime::join_connection_state.
//
// Usage Contract:
// - `easynet docker status --json` is the machine-readable operator surface
//   for CI/e2e/container health checks.
//
// Architectural Position:
// - CLI facade only. It does not inspect Docker directly and does not own
//   runtime liveness.

use clap::{Args, Subcommand};

use super::{doctor, status};

#[derive(Debug, Args)]
pub struct DockerArgs {
    #[command(subcommand)]
    pub action: DockerAction,
}

#[derive(Debug, Subcommand)]
pub enum DockerAction {
    /// Report the canonical connection state snapshot.
    Status(DockerStatusArgs),
    /// Run the normal EasyNet doctor through the Docker operator facade.
    Doctor(DockerDoctorArgs),
}

#[derive(Debug, Args)]
pub struct DockerStatusArgs {
    /// Emit JSON instead of the human-readable report.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct DockerDoctorArgs {
    /// Emit JSON instead of the human-readable report.
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: DockerArgs) -> anyhow::Result<()> {
    match args.action {
        DockerAction::Status(a) => status::run(status::StatusArgs { json: a.json }),
        DockerAction::Doctor(a) => doctor::run(doctor::DoctorArgs { json: a.json }),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::super::{App, Command};
    use super::DockerAction;

    #[test]
    fn docker_status_json_is_a_stable_operator_surface() {
        let app = App::parse_from(["easynet", "docker", "status", "--json"]);
        match app.command {
            Command::Docker(args) => match args.action {
                DockerAction::Status(status) => assert!(status.json),
                other => panic!("expected docker status, got {other:?}"),
            },
            other => panic!("expected docker command, got {other:?}"),
        }
    }

    #[test]
    fn docker_doctor_json_delegates_to_canonical_doctor_surface() {
        let app = App::parse_from(["easynet", "docker", "doctor", "--json"]);
        match app.command {
            Command::Docker(args) => match args.action {
                DockerAction::Doctor(doctor) => assert!(doctor.json),
                other => panic!("expected docker doctor, got {other:?}"),
            },
            other => panic!("expected docker command, got {other:?}"),
        }
    }
}

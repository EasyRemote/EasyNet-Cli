// EasyNet CLI — sidecar command model
// ===================================
//
// File: src/daemon/plugins/sidecar/command.rs
// Description: Executable command projection for sidecar plugin packages.

use std::path::{Path, PathBuf};

use crate::daemon::plugins::package::PluginPackage;

/// Process lifecycle model used by a process-backed plugin binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarExecutionModel {
    /// Spawn one child process per ability call/session.
    OneShotProcess,
    /// Reserved for a future daemon-managed persistent process pool.
    LongLivedSidecar,
}

/// Executable sidecar process declaration resolved from a package.
///
/// What this is NOT: plugin installation state. A command is an invocation-time
/// projection of a package entrypoint and is re-created from the immutable
/// package directory whenever the runtime host registers or invokes a sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarCommand {
    program: PathBuf,
    args: Vec<String>,
    execution_model: SidecarExecutionModel,
}

impl SidecarCommand {
    /// Construct a sidecar command from a concrete executable path.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            execution_model: SidecarExecutionModel::OneShotProcess,
        }
    }

    /// Construct a sidecar command from a program and static argv.
    pub fn with_args(program: impl Into<PathBuf>, args: impl Into<Vec<String>>) -> Self {
        Self {
            program: program.into(),
            args: args.into(),
            execution_model: SidecarExecutionModel::OneShotProcess,
        }
    }

    /// Resolve a sidecar package's entrypoint relative to the package root.
    pub fn from_package(package: &PluginPackage) -> Self {
        Self::new(package.entrypoint_path())
    }

    /// Executable path that will be spawned.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// Static process arguments.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Child process lifecycle model used by this command.
    pub fn execution_model(&self) -> SidecarExecutionModel {
        self.execution_model
    }
}

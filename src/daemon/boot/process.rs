use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::daemon::control::{discovery, transport};
use crate::daemon::persistence::config;
use crate::daemon::persistence::daemon_config::{
    self, DaemonConfig, DaemonMode as PersistedDaemonMode, DEFAULT_DAEMON_CONFIG_PATH,
    DEFAULT_DAEMON_UDS_PATH,
};
use crate::support::platform::{local_daemon_grpc, net};

use super::identity_fact::DeviceNodeIdFact;
use super::{DaemonError, Result};

const DEFAULT_DAEMON_BIN: &str = "easynet-daemon";
const DEFAULT_STOP_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_START_READY_TIMEOUT: Duration = Duration::from_secs(30);
const START_READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// SDK configuration for launching `easynet-daemon`.
///
/// Invariants:
/// 1. `node_id` is non-empty. Device mode passes the paired device id;
///    hub mode uses the stable sentinel `hub`.
/// 2. `daemon_bin` is either caller-supplied or resolved by the same
///    production rule as the CLI: `EASYNET_DAEMON_BIN`, sibling of the
///    current executable, then `easynet-daemon` on `PATH`.
/// 3. `start` never attaches to a half-alive daemon. If `control.sock`
///    is accepting but `daemon.sock` is not, startup returns
///    `ControlAliveInvocationDown` so callers do not persist a broken
///    Invocation endpoint.
#[derive(Debug, Clone)]
pub struct DaemonStartConfig {
    mode: DaemonStartMode,
    realm: Option<String>,
    node_id: String,
    daemon_bin: Option<PathBuf>,
    working_dir: Option<PathBuf>,
    env: BTreeMap<String, String>,
    log_path: Option<PathBuf>,
    detach: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DaemonStartMode {
    Device,
    Hub,
}

impl DaemonStartMode {
    fn as_str(self) -> &'static str {
        match self {
            DaemonStartMode::Device => "device",
            DaemonStartMode::Hub => "hub",
        }
    }
}

impl DaemonStartConfig {
    /// Build a daemon start config for device mode.
    pub fn device(node_id: impl Into<String>) -> Result<Self> {
        Self::new(DaemonStartMode::Device, node_id)
    }

    /// Build a daemon start config for hub mode.
    pub fn hub() -> Self {
        Self {
            mode: DaemonStartMode::Hub,
            realm: None,
            node_id: "hub".to_string(),
            daemon_bin: None,
            working_dir: None,
            env: BTreeMap::new(),
            log_path: None,
            detach: true,
        }
    }

    fn new(mode: DaemonStartMode, node_id: impl Into<String>) -> Result<Self> {
        let node_id = node_id.into();
        if node_id.trim().is_empty() {
            return Err(DaemonError::EmptyNodeId);
        }
        Ok(Self {
            mode,
            realm: None,
            node_id,
            daemon_bin: None,
            working_dir: None,
            env: BTreeMap::new(),
            log_path: None,
            detach: true,
        })
    }

    /// Attach the realm this launch request expects the daemon to
    /// serve. Existing-daemon reuse refuses mismatched realms.
    pub fn with_realm(mut self, realm: impl Into<String>) -> Self {
        let realm = realm.into();
        self.realm = Some(realm.trim().to_string());
        self
    }

    /// Override the daemon binary path.
    pub fn with_daemon_bin(mut self, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(DaemonError::EmptyBinaryPath);
        }
        self.daemon_bin = Some(path);
        Ok(self)
    }

    /// Override the daemon process working directory.
    pub fn with_working_dir(mut self, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(DaemonError::EmptyWorkingDir);
        }
        self.working_dir = Some(path);
        Ok(self)
    }

    /// Add an environment variable to the daemon process.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Override the daemon log file path.
    pub fn with_log_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.log_path = Some(path.into());
        self
    }

    /// Control whether Unix starts detach into a new session.
    pub fn detached(mut self, detach: bool) -> Self {
        self.detach = detach;
        self
    }

    /// Return the node id that will be passed through
    /// `EASYNET_NODE_ID`.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Start `easynet-daemon`, or return a handle to the already-live
    /// daemon when both control and Invocation endpoints are accepting.
    pub fn start(&self) -> Result<DaemonHandle> {
        let paths = self.launch_paths()?;
        let endpoints = paths.endpoints.clone();
        if local_daemon_grpc::probe_accepting(&endpoints.control) {
            if !local_daemon_grpc::probe_accepting(&endpoints.invocation) {
                return Err(DaemonError::ControlAliveInvocationDown {
                    control: endpoints.control.clone(),
                    invocation: endpoints.invocation.clone(),
                });
            }
            self.validate_existing_daemon_identity(&paths.discovery_path)?;
            return Ok(DaemonHandle {
                child: None,
                pid: discover_existing_daemon_pid_at(&paths.pid_path),
                endpoints,
                pid_path: paths.pid_path,
            });
        }

        let binary = self.resolve_daemon_bin();
        let log_path = paths.log_path.clone();
        let mut child = self.spawn_child(&binary, &log_path)?;
        let pid = child.id();
        if let Err(err) = write_daemon_pid_at(&paths.pid_path, pid) {
            let _ = net::kill_and_wait(pid, DEFAULT_STOP_TIMEOUT);
            return Err(err);
        }
        if let Err(err) = wait_for_ready_endpoints(
            &mut child,
            &endpoints,
            &paths.discovery_path,
            DEFAULT_START_READY_TIMEOUT,
            START_READY_POLL_INTERVAL,
        ) {
            let _ = net::kill_and_wait(pid, DEFAULT_STOP_TIMEOUT);
            let _ = std::fs::remove_file(&paths.pid_path);
            return Err(err);
        }
        Ok(DaemonHandle {
            child: Some(child),
            pid: Some(pid),
            endpoints,
            pid_path: paths.pid_path,
        })
    }

    fn resolve_daemon_bin(&self) -> PathBuf {
        if let Some(path) = &self.daemon_bin {
            return path.clone();
        }
        std::env::var_os("EASYNET_DAEMON_BIN")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join(DEFAULT_DAEMON_BIN)))
            })
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DAEMON_BIN))
    }

    fn resolve_log_path(&self) -> Result<PathBuf> {
        match self.log_path.clone() {
            Some(path) => Ok(path),
            None => Ok(self
                .effective_state_dir()?
                .join("logs")
                .join("easynet-daemon.log")),
        }
    }

    fn launch_paths(&self) -> Result<DaemonLaunchPaths> {
        let state_dir = self.effective_state_dir()?;
        Ok(DaemonLaunchPaths {
            endpoints: DaemonEndpoints {
                control: state_dir.join(transport::UDS_FILENAME),
                invocation: self.resolve_invocation_endpoint()?,
            },
            discovery_path: state_dir.join(discovery::CONTROL_JSON_FILENAME),
            pid_path: state_dir.join("easynet-daemon.pid"),
            log_path: self.resolve_log_path()?,
        })
    }

    fn effective_state_dir(&self) -> Result<PathBuf> {
        Ok(self.effective_home_dir()?.join(".easynet"))
    }

    fn effective_home_dir(&self) -> Result<PathBuf> {
        if let Some(value) = self.env.get("HOME") {
            if value.trim().is_empty() {
                return Err(DaemonError::DaemonHomeUnavailable {
                    context: "daemon child HOME override",
                });
            }
            return validate_effective_home("daemon child HOME override", PathBuf::from(value));
        }

        let home = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or(DaemonError::DaemonHomeUnavailable {
                context: "daemon process HOME",
            })?;
        validate_effective_home("daemon process HOME", home)
    }

    fn expand_effective_home(&self, path: impl AsRef<Path>) -> Result<PathBuf> {
        let path = path.as_ref();
        let Some(raw) = path.to_str() else {
            return Ok(path.to_path_buf());
        };
        if let Some(rest) = raw.strip_prefix("~/") {
            return Ok(self.effective_home_dir()?.join(rest));
        }
        Ok(path.to_path_buf())
    }

    fn resolve_invocation_endpoint(&self) -> Result<PathBuf> {
        if let Some(raw) = self
            .env
            .get("EASYNET_DAEMON_GRPC_UDS")
            .filter(|value| !value.trim().is_empty())
        {
            return self.expand_effective_home(raw);
        }

        let config_path = self.expand_effective_home(DEFAULT_DAEMON_CONFIG_PATH)?;
        match DaemonConfig::load(&config_path) {
            Ok(cfg) => self.expand_effective_home(cfg.uds_path()),
            Err(_) => self.expand_effective_home(DEFAULT_DAEMON_UDS_PATH),
        }
    }

    fn validate_existing_daemon_identity(&self, path: &Path) -> Result<()> {
        let disc = discovery::read(path)
            .map_err(|_| DaemonError::DiscoveryMissing {
                path: path.to_path_buf(),
            })?
            .ok_or_else(|| DaemonError::DiscoveryMissing {
                path: path.to_path_buf(),
            })?;
        let identity =
            disc.daemon_identity
                .ok_or_else(|| DaemonError::DiscoveryIdentityMissing {
                    path: path.to_path_buf(),
                })?;

        self.validate_discovered_identity(identity)
    }

    fn validate_discovered_identity(&self, identity: discovery::DaemonIdentity) -> Result<()> {
        if !mode_matches(self.mode, identity.mode.as_str()) {
            return Err(DaemonError::DiscoveryIdentityMismatch {
                field: "mode",
                requested: self.mode.as_str().to_string(),
                actual: identity.mode,
            });
        }
        if let Some(expected_realm) = self.realm.as_ref().filter(|realm| !realm.is_empty()) {
            if expected_realm != &identity.realm {
                return Err(DaemonError::DiscoveryIdentityMismatch {
                    field: "realm",
                    requested: expected_realm.clone(),
                    actual: identity.realm,
                });
            }
        }
        if matches!(self.mode, DaemonStartMode::Device) {
            let requested_node = DeviceNodeIdFact::from_optional(Some(self.node_id.trim()));
            let actual_node = DeviceNodeIdFact::from_optional(identity.node_id.as_deref());
            if requested_node.present_value() != actual_node.present_value() {
                return Err(DaemonError::DiscoveryIdentityMismatch {
                    field: "node_id",
                    requested: requested_node.mismatch_value(),
                    actual: actual_node.mismatch_value(),
                });
            }
        }
        Ok(())
    }

    fn spawn_child(&self, binary: &Path, log_path: &Path) -> Result<Child> {
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| DaemonError::CreateLogDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .map_err(|source| DaemonError::OpenLog {
                path: log_path.to_path_buf(),
                source,
            })?;

        let mut cmd = Command::new(binary);
        if let Some(working_dir) = &self.working_dir {
            cmd.current_dir(working_dir);
        }
        cmd.env("EASYNET_NODE_ID", self.node_id.trim());
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        cmd.stdin(Stdio::null());
        if let Ok(stdout) = log.try_clone() {
            cmd.stdout(Stdio::from(stdout));
        }
        cmd.stderr(Stdio::from(log));

        #[cfg(unix)]
        if self.detach {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::signal(libc::SIGHUP, libc::SIG_IGN) == libc::SIG_ERR {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        cmd.spawn().map_err(|source| DaemonError::Spawn {
            binary: binary.to_path_buf(),
            source,
        })
    }
}

fn validate_effective_home(context: &'static str, path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }
    Err(DaemonError::DaemonStateRootUnavailable {
        context,
        source: anyhow::anyhow!(
            "HOME must resolve to an absolute path before deriving .easynet state root, got {}",
            path.display()
        ),
    })
}

fn mode_matches(requested: DaemonStartMode, actual: &str) -> bool {
    match requested {
        DaemonStartMode::Device => actual == PersistedDaemonMode::Device.as_str(),
        DaemonStartMode::Hub => {
            actual == PersistedDaemonMode::Hub.as_str()
                || actual == PersistedDaemonMode::Both.as_str()
        }
    }
}

/// Runtime endpoints exposed by the local daemon.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DaemonEndpoints {
    pub(crate) control: PathBuf,
    pub(crate) invocation: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct DaemonLaunchPaths {
    endpoints: DaemonEndpoints,
    discovery_path: PathBuf,
    pid_path: PathBuf,
    log_path: PathBuf,
}

impl DaemonEndpoints {
    /// Resolve endpoints from the current process environment and
    /// daemon configuration files.
    pub fn current() -> Self {
        Self::try_current().unwrap_or_else(|error| panic!("{error}"))
    }

    /// Fallible endpoint resolution for production lifecycle paths.
    ///
    /// Endpoint discovery is part of the daemon state-root state machine. A
    /// missing or invalid state root must surface as a lifecycle error instead
    /// of panicking or re-deriving paths from the current working directory.
    pub fn try_current() -> Result<Self> {
        Ok(Self {
            control: transport::try_default_socket_path().map_err(|source| {
                DaemonError::DaemonStateRootUnavailable {
                    context: "control endpoint discovery",
                    source,
                }
            })?,
            invocation: daemon_config::resolved_local_uds_path_with_env_override(),
        })
    }

    /// Boot/status control endpoint (`control.sock` or named pipe).
    pub fn control(&self) -> &Path {
        &self.control
    }

    /// Axon Invocation endpoint (`daemon.sock` or named pipe).
    pub fn invocation(&self) -> &Path {
        &self.invocation
    }
}

/// Handle returned by `DaemonStartConfig::start`.
///
/// Invariants:
/// 1. `child.is_some()` means this handle owns the process it just
///    spawned. `child.is_none()` means a daemon was already alive.
/// 2. `pid` is best-effort discovery for status/stop; endpoint probes
///    are the authoritative liveness checks.
/// 3. Dropping the handle does not stop the daemon. Call `stop()` for
///    explicit teardown, matching the CLI's background lifecycle.
#[derive(Debug)]
pub struct DaemonHandle {
    child: Option<Child>,
    pid: Option<u32>,
    endpoints: DaemonEndpoints,
    pid_path: PathBuf,
}

impl DaemonHandle {
    /// Attach to an already-running daemon without spawning a new
    /// process.
    pub fn attach_current() -> Result<Self> {
        let endpoints = DaemonEndpoints::try_current()?;
        let control_accepting = local_daemon_grpc::probe_accepting(&endpoints.control);
        let invocation_accepting = local_daemon_grpc::probe_accepting(&endpoints.invocation);
        if control_accepting && !invocation_accepting {
            return Err(DaemonError::ControlAliveInvocationDown {
                control: endpoints.control.clone(),
                invocation: endpoints.invocation.clone(),
            });
        }
        if !invocation_accepting {
            return Err(DaemonError::InvocationEndpointDown {
                endpoint: endpoints.invocation.clone(),
            });
        }
        Ok(Self {
            child: None,
            pid: discover_existing_daemon_pid(),
            endpoints,
            pid_path: config::try_easynet_daemon_pid_path().map_err(|source| {
                DaemonError::DaemonStateRootUnavailable {
                    context: "daemon pidfile discovery",
                    source,
                }
            })?,
        })
    }

    /// PID of the daemon process, when known.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Borrow the spawned child process, when this handle started it.
    pub fn child_mut(&mut self) -> Option<&mut Child> {
        self.child.as_mut()
    }

    /// Local control endpoint.
    pub fn control_endpoint(&self) -> &Path {
        self.endpoints.control()
    }

    /// Local Axon Invocation endpoint.
    pub fn invocation_endpoint(&self) -> &Path {
        self.endpoints.invocation()
    }

    /// Runtime endpoints owned by this handle.
    pub fn endpoints(&self) -> &DaemonEndpoints {
        &self.endpoints
    }

    /// Release local process ownership without stopping the daemon.
    pub fn detach(&mut self) {
        self.child = None;
    }

    /// Snapshot daemon liveness through pid and endpoint probes.
    pub fn status(&self) -> DaemonStatus {
        DaemonStatus::from_parts(self.pid, self.endpoints.clone())
    }

    /// Stop this daemon if a PID is known.
    pub fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            let result = stop_owned_child(&mut child, DEFAULT_STOP_TIMEOUT);
            if result.is_ok() {
                self.pid = None;
                let _ = std::fs::remove_file(&self.pid_path);
            } else {
                self.child = Some(child);
            }
            return result;
        }

        let Some(pid) = self.pid else {
            return Ok(());
        };
        stop_pid(pid, DEFAULT_STOP_TIMEOUT)?;
        self.pid = None;
        let _ = std::fs::remove_file(&self.pid_path);
        Ok(())
    }
}

/// Current daemon liveness snapshot.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DaemonStatus {
    pid: Option<u32>,
    pid_alive: bool,
    control_accepting: bool,
    invocation_accepting: bool,
    endpoints: DaemonEndpoints,
}

impl DaemonStatus {
    /// Resolve status for the current host without starting a daemon.
    pub fn current() -> Self {
        let endpoints = DaemonEndpoints::current();
        Self::from_parts(discover_existing_daemon_pid(), endpoints)
    }

    /// Fallible status resolution for lifecycle paths that must not panic on
    /// an invalid state root.
    pub fn try_current() -> Result<Self> {
        let endpoints = DaemonEndpoints::try_current()?;
        Ok(Self::from_parts(discover_existing_daemon_pid(), endpoints))
    }

    fn from_parts(pid: Option<u32>, endpoints: DaemonEndpoints) -> Self {
        let pid_alive = pid.is_some_and(net::is_pid_alive);
        let control_accepting = local_daemon_grpc::probe_accepting(&endpoints.control);
        let invocation_accepting = local_daemon_grpc::probe_accepting(&endpoints.invocation);
        Self {
            pid,
            pid_alive,
            control_accepting,
            invocation_accepting,
            endpoints,
        }
    }

    /// PID of the daemon process, when known.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Whether the known PID is alive.
    pub fn pid_alive(&self) -> bool {
        self.pid_alive
    }

    /// Whether the control endpoint accepts connections.
    pub fn control_accepting(&self) -> bool {
        self.control_accepting
    }

    /// Whether the Invocation endpoint accepts connections.
    pub fn invocation_accepting(&self) -> bool {
        self.invocation_accepting
    }

    /// Endpoints this status checked.
    pub fn endpoints(&self) -> &DaemonEndpoints {
        &self.endpoints
    }
}

/// Stop the daemon named by the SDK pidfile, if it exists.
pub fn stop_daemon() -> Result<()> {
    let Some(pid) = read_daemon_pid() else {
        return Ok(());
    };
    stop_pid(pid, DEFAULT_STOP_TIMEOUT)?;
    let _ = std::fs::remove_file(config::easynet_daemon_pid_path());
    Ok(())
}

/// Start `easynet-daemon` with the supplied SDK config.
pub fn start_daemon(config: &DaemonStartConfig) -> Result<DaemonHandle> {
    config.start()
}

fn read_daemon_pid() -> Option<u32> {
    std::fs::read_to_string(config::easynet_daemon_pid_path())
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

fn write_daemon_pid_at(pid_path: &Path, pid: u32) -> Result<()> {
    if let Some(parent) = pid_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| DaemonError::CreatePidDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(pid_path, pid.to_string()).map_err(|source| DaemonError::WritePid {
        path: pid_path.to_path_buf(),
        source,
    })
}

fn discover_existing_daemon_pid() -> Option<u32> {
    config::try_easynet_daemon_pid_path()
        .ok()
        .and_then(|path| discover_existing_daemon_pid_at(&path))
}

fn discover_existing_daemon_pid_at(pid_path: &Path) -> Option<u32> {
    read_daemon_pid_at(pid_path).filter(|pid| net::is_pid_alive(*pid))
}

fn read_daemon_pid_at(pid_path: &Path) -> Option<u32> {
    std::fs::read_to_string(pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

fn stop_pid(pid: u32, timeout: Duration) -> Result<()> {
    if !net::is_pid_alive(pid) {
        return Err(DaemonError::PidNotAlive { pid });
    }
    if !net::is_easynet_process(pid) {
        return Err(DaemonError::PidReuseRefused { pid });
    }
    if net::kill_and_wait(pid, timeout) {
        Ok(())
    } else {
        Err(DaemonError::StopTimedOut {
            pid,
            timeout_ms: timeout.as_millis() as u64,
        })
    }
}

fn stop_owned_child(child: &mut Child, timeout: Duration) -> Result<()> {
    let pid = child.id();
    if child_has_exited(child)? {
        return Ok(());
    }

    signal_child_terminate(child)?;
    let deadline = Instant::now() + timeout;
    loop {
        if child_has_exited(child)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(DaemonError::StopTimedOut {
                pid,
                timeout_ms: timeout.as_millis() as u64,
            });
        }
        std::thread::sleep(STOP_POLL_INTERVAL);
    }
}

fn child_has_exited(child: &mut Child) -> Result<bool> {
    child
        .try_wait()
        .map(|status| status.is_some())
        .map_err(|source| DaemonError::WaitChild {
            pid: child.id(),
            source,
        })
}

#[cfg(unix)]
fn signal_child_terminate(child: &mut Child) -> Result<()> {
    let pid = child.id();
    let raw_pid = i32::try_from(pid).map_err(|_| DaemonError::SignalChild {
        pid,
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "pid does not fit i32"),
    })?;
    let rc = unsafe { libc::kill(raw_pid, libc::SIGTERM) };
    if rc == 0 {
        return Ok(());
    }

    let source = std::io::Error::last_os_error();
    if source.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(DaemonError::SignalChild { pid, source })
    }
}

#[cfg(not(unix))]
fn signal_child_terminate(child: &mut Child) -> Result<()> {
    let pid = child.id();
    child
        .kill()
        .map_err(|source| DaemonError::SignalChild { pid, source })
}

fn wait_for_ready_endpoints(
    child: &mut Child,
    endpoints: &DaemonEndpoints,
    discovery_path: &Path,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if local_daemon_grpc::probe_accepting(&endpoints.control)
            && local_daemon_grpc::probe_accepting(&endpoints.invocation)
            && discovery_is_ready(discovery_path, endpoints)
        {
            return Ok(());
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(DaemonError::ExitedBeforeReady {
                    pid: child.id(),
                    status: status.to_string(),
                    control: endpoints.control.clone(),
                    invocation: endpoints.invocation.clone(),
                });
            }
            Ok(None) => {}
            Err(source) => {
                return Err(DaemonError::ProbeChild {
                    pid: child.id(),
                    source,
                });
            }
        }

        if Instant::now() >= deadline {
            return Err(DaemonError::ReadyTimedOut {
                pid: child.id(),
                timeout_ms: timeout.as_millis() as u64,
                control: endpoints.control.clone(),
                invocation: endpoints.invocation.clone(),
            });
        }
        std::thread::sleep(poll_interval);
    }
}

fn discovery_is_ready(path: &Path, endpoints: &DaemonEndpoints) -> bool {
    let Ok(Some(disc)) = discovery::read(path) else {
        return false;
    };
    disc.invocation_endpoint.as_deref() == Some(endpoints.invocation())
        && disc.daemon_identity.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_config_rejects_empty_node_id() {
        assert!(matches!(
            DaemonStartConfig::device("  "),
            Err(DaemonError::EmptyNodeId)
        ));
    }

    #[test]
    fn endpoints_current_uses_control_and_invocation_paths() {
        let endpoints = DaemonEndpoints::current();
        assert!(
            endpoints.control().ends_with("control.sock") || cfg!(windows),
            "control endpoint should resolve to control.sock on Unix"
        );
        assert!(
            endpoints.invocation().ends_with("daemon.sock") || cfg!(windows),
            "invocation endpoint should resolve to daemon.sock on Unix"
        );
    }

    #[test]
    fn launch_paths_resolve_against_child_environment() {
        let config = DaemonStartConfig::device("node-a")
            .unwrap()
            .with_env("HOME", "/tmp/easynet-sdk-home")
            .with_env(
                "EASYNET_DAEMON_GRPC_UDS",
                "~/.easynet/custom-invocation.sock",
            );
        let paths = config.launch_paths().expect("launch paths");

        assert_eq!(
            paths.endpoints.control(),
            Path::new("/tmp/easynet-sdk-home/.easynet/control.sock")
        );
        assert_eq!(
            paths.endpoints.invocation(),
            Path::new("/tmp/easynet-sdk-home/.easynet/custom-invocation.sock")
        );
        assert_eq!(
            paths.discovery_path,
            PathBuf::from("/tmp/easynet-sdk-home/.easynet/control.json")
        );
        assert_eq!(
            paths.pid_path,
            PathBuf::from("/tmp/easynet-sdk-home/.easynet/easynet-daemon.pid")
        );
    }

    #[test]
    fn launch_paths_reject_blank_child_home_instead_of_process_fallback() {
        let config = DaemonStartConfig::device("node-a")
            .unwrap()
            .with_env("HOME", " ");
        let err = config
            .launch_paths()
            .expect_err("blank child HOME must fail closed");

        assert!(matches!(
            err,
            DaemonError::DaemonHomeUnavailable {
                context: "daemon child HOME override"
            }
        ));
    }

    #[test]
    fn launch_paths_reject_relative_child_home_before_cwd_fallback() {
        let config = DaemonStartConfig::device("node-a")
            .unwrap()
            .with_env("HOME", "relative-home");
        let err = config
            .launch_paths()
            .expect_err("relative child HOME must fail closed");

        assert!(matches!(
            err,
            DaemonError::DaemonStateRootUnavailable {
                context: "daemon child HOME override",
                ..
            }
        ));
    }

    #[test]
    fn launch_paths_reject_missing_process_home_instead_of_cwd_fallback() {
        let _lock = crate::cli::commands::test_support::env_lock();
        let previous_home = std::env::var_os("HOME");
        struct RestoreHome(Option<std::ffi::OsString>);
        impl Drop for RestoreHome {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var("HOME", value),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
        let _restore = RestoreHome(previous_home);
        std::env::remove_var("HOME");

        let config = DaemonStartConfig::device("node-a").unwrap();
        let err = config
            .launch_paths()
            .expect_err("missing HOME must fail closed");

        assert!(matches!(
            err,
            DaemonError::DaemonHomeUnavailable {
                context: "daemon process HOME"
            }
        ));
    }

    #[test]
    fn discover_existing_daemon_pid_returns_none_for_missing_pidfile() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pid_path = temp.path().join("missing-daemon.pid");

        assert_eq!(
            discover_existing_daemon_pid_at(&pid_path),
            None,
            "missing pidfile must not fall back to global process-name discovery"
        );
    }

    #[test]
    fn discover_existing_daemon_pid_returns_none_for_stale_pidfile() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pid_path = temp.path().join("stale-daemon.pid");
        std::fs::write(&pid_path, "999999999").expect("write stale pidfile");

        assert_eq!(
            discover_existing_daemon_pid_at(&pid_path),
            None,
            "stale pidfile must not fall back to global process-name discovery"
        );
    }

    #[test]
    fn device_attach_identity_rejects_wrong_node() {
        let config = DaemonStartConfig::device("node-a")
            .unwrap()
            .with_realm("realm-a");
        let err = config
            .validate_discovered_identity(discovery::DaemonIdentity {
                mode: "device".into(),
                realm: "realm-a".into(),
                node_id: Some("node-b".into()),
            })
            .unwrap_err();

        assert!(matches!(
            err,
            DaemonError::DiscoveryIdentityMismatch {
                field: "node_id",
                ..
            }
        ));
    }

    #[test]
    fn device_attach_identity_rejects_missing_discovered_node_id() {
        let config = DaemonStartConfig::device("node-a")
            .unwrap()
            .with_realm("realm-a");
        let err = config
            .validate_discovered_identity(discovery::DaemonIdentity {
                mode: "device".into(),
                realm: "realm-a".into(),
                node_id: None,
            })
            .unwrap_err();

        assert!(matches!(
            err,
            DaemonError::DiscoveryIdentityMismatch {
                field: "node_id",
                requested,
                actual,
            } if requested == "node-a" && actual == "<missing>"
        ));
    }

    #[test]
    fn device_attach_identity_rejects_blank_discovered_node_id() {
        let config = DaemonStartConfig::device("node-a")
            .unwrap()
            .with_realm("realm-a");
        let err = config
            .validate_discovered_identity(discovery::DaemonIdentity {
                mode: "device".into(),
                realm: "realm-a".into(),
                node_id: Some("  ".into()),
            })
            .unwrap_err();

        assert!(matches!(
            err,
            DaemonError::DiscoveryIdentityMismatch {
                field: "node_id",
                requested,
                actual,
            } if requested == "node-a" && actual == "<blank>"
        ));
    }

    #[test]
    fn hub_attach_identity_accepts_both_mode_same_realm() {
        let config = DaemonStartConfig::hub().with_realm("realm-a");

        config
            .validate_discovered_identity(discovery::DaemonIdentity {
                mode: "both".into(),
                realm: "realm-a".into(),
                node_id: Some("hub".into()),
            })
            .expect("hub start may reuse both-mode daemon");
    }

    #[test]
    fn hub_attach_identity_rejects_wrong_realm() {
        let config = DaemonStartConfig::hub().with_realm("realm-a");
        let err = config
            .validate_discovered_identity(discovery::DaemonIdentity {
                mode: "hub".into(),
                realm: "realm-b".into(),
                node_id: Some("hub".into()),
            })
            .unwrap_err();

        assert!(matches!(
            err,
            DaemonError::DiscoveryIdentityMismatch { field: "realm", .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_ready_endpoints_reports_child_exit_before_ready() {
        let mut child = Command::new("sh")
            .args(["-c", "exit 17"])
            .spawn()
            .expect("spawn exiting child");
        let endpoints = DaemonEndpoints {
            control: PathBuf::from("/tmp/easynet-never-ready-control.sock"),
            invocation: PathBuf::from("/tmp/easynet-never-ready-daemon.sock"),
        };
        let err = wait_for_ready_endpoints(
            &mut child,
            &endpoints,
            Path::new("/tmp/easynet-never-ready-control.json"),
            Duration::from_secs(1),
            Duration::from_millis(10),
        )
        .unwrap_err();
        assert!(matches!(err, DaemonError::ExitedBeforeReady { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn stop_owned_child_reaps_sigterm_exit_without_timeout() {
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep child");

        stop_owned_child(&mut child, Duration::from_secs(2)).expect("stop owned child");
        assert!(
            child.try_wait().expect("query stopped child").is_some(),
            "owned child should be reaped after stop"
        );
    }

    #[cfg(unix)]
    #[test]
    fn daemon_handle_stop_clears_owned_pid_after_reap() {
        let child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep child");
        let pid = child.id();
        let endpoints = DaemonEndpoints {
            control: PathBuf::from("/tmp/easynet-owned-control.sock"),
            invocation: PathBuf::from("/tmp/easynet-owned-daemon.sock"),
        };
        let mut handle = DaemonHandle {
            child: Some(child),
            pid: Some(pid),
            endpoints,
            pid_path: PathBuf::from("/tmp/easynet-owned-daemon.pid"),
        };

        handle.stop().expect("stop owned daemon handle");

        assert_eq!(handle.pid(), None);
        assert!(handle.child_mut().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn wait_for_ready_endpoints_times_out_when_child_stays_alive() {
        let mut child = Command::new("sh")
            .args(["-c", "sleep 1"])
            .spawn()
            .expect("spawn sleeping child");
        let endpoints = DaemonEndpoints {
            control: PathBuf::from("/tmp/easynet-never-ready-control.sock"),
            invocation: PathBuf::from("/tmp/easynet-never-ready-daemon.sock"),
        };
        let err = wait_for_ready_endpoints(
            &mut child,
            &endpoints,
            Path::new("/tmp/easynet-never-ready-control.json"),
            Duration::from_millis(20),
            Duration::from_millis(5),
        )
        .unwrap_err();
        let _ = net::kill_and_wait(child.id(), DEFAULT_STOP_TIMEOUT);
        assert!(matches!(err, DaemonError::ReadyTimedOut { .. }));
    }
}

//! Lifecycle owner for the local daemon key service.
//!
//! Every product surface attaches through this one state machine.  Callers do
//! not open the vault and do not reproduce child-process management.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context as _;

use crate::daemon::identity::self_identity::{KeyringClient, SelfIdentityError};

use super::{home_relative, try_default_owner_path, try_default_socket_path};

// A fresh vault performs the production Argon2id derivation before binding its
// socket. Debug builds and contended edge devices can legitimately cross five
// seconds, so readiness must cover that supported startup path.
const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(15);
const PROBE_INTERVAL: Duration = Duration::from_millis(100);
const STARTUP_LEASE_STALE_AFTER: Duration = Duration::from_secs(30);
const SUPERVISOR_PROBE_INTERVAL: Duration = Duration::from_secs(1);
const SUPERVISOR_MAX_BACKOFF: Duration = Duration::from_secs(5);
const KEY_SERVICE_OWNER_PID_ENV: &str = "EASYNET_KEYRING_OWNER_PID";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyServiceStartup {
    Attached,
    Spawned,
}

/// RAII ownership for key-service use by a short-lived bootstrap command.
///
/// Attached daemon custody is never stopped. A service spawned by the current
/// command is synchronously reaped on [`Self::release`] and is also reclaimed
/// best-effort on early-return paths through `Drop`.
pub struct KeyServiceBootstrapLease {
    owns_service: bool,
}

impl KeyServiceBootstrapLease {
    pub fn acquire() -> anyhow::Result<Self> {
        ensure_bootstrap_key_service_running().map(Self::from_startup)
    }

    pub fn from_startup(startup: KeyServiceStartup) -> Self {
        Self {
            owns_service: startup == KeyServiceStartup::Spawned,
        }
    }

    pub fn release(mut self) -> anyhow::Result<()> {
        if !self.owns_service {
            return Ok(());
        }
        shutdown_bootstrap_key_service()?;
        self.owns_service = false;
        Ok(())
    }
}

impl Drop for KeyServiceBootstrapLease {
    fn drop(&mut self) {
        if self.owns_service {
            let _ = shutdown_bootstrap_key_service();
            self.owns_service = false;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyServiceOwnershipIntent {
    /// A short-lived command may attach to a daemon-owned service. When no
    /// service exists it temporarily owns one and must release it before the
    /// command exits or forks the runtime daemon.
    Bootstrap,
    /// The runtime daemon is the lifecycle authority. It may only attach to a
    /// service already owned by this process; a service owned by another live
    /// process is a conflicting runtime, not a valid dependency.
    Daemon,
}

/// Observable lifecycle state for the process-local key-service supervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyServiceLifecycleState {
    Attached,
    Running,
    Restarting { attempt: u32 },
    Failed { message: String },
}

static KEY_SERVICE_MANAGER: OnceLock<KeyServiceManager> = OnceLock::new();

/// Attach to daemon-owned custody or start a temporary bootstrap service.
///
/// Callers that receive [`KeyServiceStartup::Spawned`] own a short-lived
/// service and must call [`shutdown_bootstrap_key_service`] before exiting or
/// spawning the runtime daemon.
pub fn ensure_bootstrap_key_service_running() -> anyhow::Result<KeyServiceStartup> {
    key_service_manager()?.ensure_running(KeyServiceOwnershipIntent::Bootstrap)
}

/// Establish custody owned by the current runtime daemon.
///
/// Unlike bootstrap access, daemon startup never attaches to a service owned
/// by another live process. Doing so would make signer availability depend on
/// that process's lifetime and publish a false Ready state.
pub fn ensure_daemon_key_service_running() -> anyhow::Result<KeyServiceStartup> {
    key_service_manager()?.ensure_running(KeyServiceOwnershipIntent::Daemon)
}

fn key_service_manager() -> anyhow::Result<&'static KeyServiceManager> {
    if let Some(manager) = KEY_SERVICE_MANAGER.get() {
        return Ok(manager);
    }
    let manager = KeyServiceManager::try_default()?;
    let _ = KEY_SERVICE_MANAGER.set(manager);
    KEY_SERVICE_MANAGER
        .get()
        .context("daemon key service manager initialized")
}

/// Current lifecycle state observed by this process.
pub fn key_service_lifecycle_state() -> Option<KeyServiceLifecycleState> {
    KEY_SERVICE_MANAGER
        .get()
        .and_then(KeyServiceManager::snapshot)
}

/// Stop the key service only when this daemon process started it.
///
/// An attached key service belongs to another live runtime and is deliberately
/// left alone. This is the terminal lifecycle transition for daemon shutdown
/// and boot failure: without it, the detached custody child outlives its
/// daemon and later starts can accumulate orphan processes.
pub fn shutdown_key_service() -> anyhow::Result<()> {
    match KEY_SERVICE_MANAGER.get() {
        Some(manager) => manager.shutdown(),
        None => Ok(()),
    }
}

/// Stop a key service that a short-lived bootstrap command started.
///
/// `device join` needs the custody service before `easynet-daemon` exists so it
/// can publish the device public projection.  That CLI-owned service must not
/// remain alive for a later daemon to attach to a soon-to-exit owner.  Unlike
/// daemon shutdown, this transition leaves the process-local manager reusable
/// for the rest of the command.
pub fn shutdown_bootstrap_key_service() -> anyhow::Result<()> {
    match KEY_SERVICE_MANAGER.get() {
        Some(manager) => manager.shutdown_bootstrap(),
        None => Ok(()),
    }
}

struct KeyServiceManager {
    lifecycle: KeyServiceLifecycle,
    state: Mutex<KeyServiceManagerState>,
}

impl KeyServiceManager {
    fn try_default() -> anyhow::Result<Self> {
        Ok(Self {
            lifecycle: KeyServiceLifecycle::try_default()?,
            state: Mutex::new(KeyServiceManagerState::default()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyServiceManagerPhase {
    Dormant,
    Attached,
    Running,
    Restarting { attempt: u32 },
    Failed { message: String },
}

struct KeyServiceManagerState {
    phase: KeyServiceManagerPhase,
    child: Option<Child>,
    supervisor: Option<std::thread::JoinHandle<()>>,
    shutdown_requested: bool,
}

impl Default for KeyServiceManagerState {
    fn default() -> Self {
        Self {
            phase: KeyServiceManagerPhase::Dormant,
            child: None,
            supervisor: None,
            shutdown_requested: false,
        }
    }
}

impl KeyServiceManagerState {
    fn snapshot(&self) -> Option<KeyServiceLifecycleState> {
        match &self.phase {
            KeyServiceManagerPhase::Dormant => None,
            KeyServiceManagerPhase::Attached => Some(KeyServiceLifecycleState::Attached),
            KeyServiceManagerPhase::Running => Some(KeyServiceLifecycleState::Running),
            KeyServiceManagerPhase::Restarting { attempt } => {
                Some(KeyServiceLifecycleState::Restarting { attempt: *attempt })
            }
            KeyServiceManagerPhase::Failed { message } => Some(KeyServiceLifecycleState::Failed {
                message: message.clone(),
            }),
        }
    }

    /// Reap an owned process that has already exited without giving up
    /// ownership of a process whose state cannot be inspected.
    fn reap_exited_child(&mut self) -> anyhow::Result<()> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        match child.try_wait() {
            Ok(Some(_status)) => {
                self.child.take();
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(error) => Err(error).context("inspect owned daemon key service process"),
        }
    }

    /// Stop and reap the owned process. If either operation cannot be
    /// completed, the `Child` remains in this global state so it is never
    /// silently detached from lifecycle ownership.
    fn terminate_owned_child(&mut self) -> anyhow::Result<()> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };

        if let Ok(Some(_status)) = child.try_wait() {
            self.child.take();
            return Ok(());
        }

        if let Err(kill_error) = child.kill() {
            return match child.try_wait() {
                Ok(Some(_status)) => {
                    self.child.take();
                    Ok(())
                }
                Ok(None) => Err(kill_error).context(
                    "terminate owned daemon key service process; ownership retained by manager",
                ),
                Err(wait_error) => Err(anyhow::anyhow!(
                    "terminate owned daemon key service process: {kill_error}; \
                     inspect process after failed termination: {wait_error}; \
                     ownership retained by manager"
                )),
            };
        }

        match child.wait() {
            Ok(_status) => {
                self.child.take();
                Ok(())
            }
            Err(error) => Err(error).context(
                "reap terminated daemon key service process; ownership retained by manager",
            ),
        }
    }
}

impl KeyServiceManager {
    /// One process-wide critical section owns the complete transition:
    /// inspect/attach/spawn, transfer `Child` ownership, publish state, and
    /// install the supervisor for an owned child. Attached services retain
    /// their existing lifecycle owner and are never supervised by a consumer.
    fn ensure_running(
        &'static self,
        intent: KeyServiceOwnershipIntent,
    ) -> anyhow::Result<KeyServiceStartup> {
        let mut state = self.lock_state();
        let startup = self.transition_to_running(&mut state, 1, intent)?;
        if state.child.is_none() {
            debug_assert_eq!(startup, KeyServiceStartup::Attached);
            return Ok(startup);
        }
        if let Err(supervisor_error) = self.install_supervisor(&mut state) {
            let cleanup = state.terminate_owned_child();
            let error = combine_lifecycle_errors(supervisor_error, cleanup.err());
            state.phase = KeyServiceManagerPhase::Failed {
                message: error.to_string(),
            };
            return Err(error);
        }
        Ok(startup)
    }

    fn snapshot(&self) -> Option<KeyServiceLifecycleState> {
        self.lock_state().snapshot()
    }

    fn shutdown(&self) -> anyhow::Result<()> {
        let supervisor = {
            let mut state = self.lock_state();
            state.shutdown_requested = true;
            state.supervisor.take()
        };

        if let Some(supervisor) = supervisor {
            supervisor
                .join()
                .map_err(|_| anyhow::anyhow!("join daemon key service supervisor"))?;
        }

        let mut state = self.lock_state();
        let result = state.terminate_owned_child();
        state.phase = match &result {
            Ok(()) => KeyServiceManagerPhase::Dormant,
            Err(error) => KeyServiceManagerPhase::Failed {
                message: error.to_string(),
            },
        };
        result
    }

    fn shutdown_bootstrap(&self) -> anyhow::Result<()> {
        let supervisor = {
            let mut state = self.lock_state();
            state.shutdown_requested = true;
            state.supervisor.take()
        };

        if let Some(supervisor) = supervisor {
            supervisor
                .join()
                .map_err(|_| anyhow::anyhow!("join bootstrap key service supervisor"))?;
        }

        let mut state = self.lock_state();
        let result = state.terminate_owned_child();
        state.phase = match &result {
            Ok(()) => KeyServiceManagerPhase::Dormant,
            Err(error) => KeyServiceManagerPhase::Failed {
                message: error.to_string(),
            },
        };
        state.shutdown_requested = false;
        result
    }

    fn lock_state(&self) -> MutexGuard<'_, KeyServiceManagerState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn transition_to_running(
        &self,
        state: &mut KeyServiceManagerState,
        attempt: u32,
        intent: KeyServiceOwnershipIntent,
    ) -> anyhow::Result<KeyServiceStartup> {
        let result = self.try_transition_to_running(state, attempt, intent);
        if let Err(error) = &result {
            state.phase = KeyServiceManagerPhase::Failed {
                message: error.to_string(),
            };
        }
        result
    }

    fn try_transition_to_running(
        &self,
        state: &mut KeyServiceManagerState,
        attempt: u32,
        intent: KeyServiceOwnershipIntent,
    ) -> anyhow::Result<KeyServiceStartup> {
        anyhow::ensure!(
            !state.shutdown_requested,
            "daemon key service lifecycle is shutting down"
        );
        state.reap_exited_child()?;

        if state.child.is_some() {
            let client = KeyringClient::new(&self.lifecycle.socket_path)
                .with_timeout(Duration::from_secs(2));
            match probe_ready(&client) {
                Ok(true) => {
                    state.phase = KeyServiceManagerPhase::Running;
                    return Ok(KeyServiceStartup::Attached);
                }
                Ok(false) => {
                    // A live owned custody process can transiently refuse the
                    // UDS health probe while the listener is rebinding or the
                    // accept loop is saturated. Treat that as a bounded
                    // recovery window, not as proof that the child is dead.
                    //
                    // Killing immediately here races foreground daemon boot:
                    // `daemon-key-service` can report ready, the supervisor can
                    // observe one missed probe, and `daemon-invocation-transport`
                    // then sees `Connection refused` while the supervisor
                    // restarts the signer service. The process state machine
                    // must instead distinguish:
                    //
                    //   Running -> Restarting (transient, owned child alive)
                    //   Restarting -> Running (health restored)
                    //   Restarting -> Failed (child exits or readiness times out)
                    state.phase = KeyServiceManagerPhase::Restarting { attempt };
                    let readiness_deadline = Instant::now() + self.lifecycle.ready_timeout;
                    if let Err(readiness_error) =
                        self.lifecycle
                            .wait_ready(state, &client, readiness_deadline)
                    {
                        let cleanup = state.terminate_owned_child().err();
                        return Err(combine_lifecycle_errors(readiness_error, cleanup));
                    }
                    state.phase = KeyServiceManagerPhase::Running;
                    return Ok(KeyServiceStartup::Attached);
                }
                Err(probe_error) => {
                    let cleanup = state.terminate_owned_child().err();
                    return Err(combine_lifecycle_errors(
                        probe_error.context("owned daemon key service is unhealthy"),
                        cleanup,
                    ));
                }
            }
        }

        state.phase = KeyServiceManagerPhase::Restarting { attempt };
        let startup = self.lifecycle.ensure_running(state, intent)?;
        state.phase = if state.child.is_some() {
            KeyServiceManagerPhase::Running
        } else {
            KeyServiceManagerPhase::Attached
        };
        Ok(startup)
    }

    fn install_supervisor(&'static self, state: &mut KeyServiceManagerState) -> anyhow::Result<()> {
        if state
            .supervisor
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            if let Some(finished) = state.supervisor.take() {
                let _ = finished.join();
            }
        }
        if state.supervisor.is_some() {
            return Ok(());
        }

        let worker = std::thread::Builder::new()
            .name("easynet-key-service-supervisor".into())
            .spawn(move || self.supervise())
            .context("spawn daemon key service supervisor thread")?;
        state.supervisor = Some(worker);
        Ok(())
    }

    fn supervise(&'static self) {
        let mut restart_attempt = 0u32;
        loop {
            std::thread::sleep(SUPERVISOR_PROBE_INTERVAL);
            restart_attempt = restart_attempt.saturating_add(1);

            let result = {
                let mut state = self.lock_state();
                if state.shutdown_requested {
                    return;
                }
                self.transition_to_running(
                    &mut state,
                    restart_attempt,
                    KeyServiceOwnershipIntent::Daemon,
                )
            };

            match result {
                Ok(_) => restart_attempt = 0,
                Err(_) => std::thread::sleep(supervisor_backoff(restart_attempt)),
            }
        }
    }
}

fn combine_lifecycle_errors(
    primary: anyhow::Error,
    cleanup: Option<anyhow::Error>,
) -> anyhow::Error {
    match cleanup {
        Some(cleanup) => anyhow::anyhow!(
            "{primary}; additionally failed to terminate/reap owned key service: {cleanup}"
        ),
        None => primary,
    }
}

#[derive(Debug, Clone)]
struct KeyServiceLifecycle {
    socket_path: PathBuf,
    binary_path: PathBuf,
    log_path: PathBuf,
    lease_path: PathBuf,
    owner_path: PathBuf,
    ready_timeout: Duration,
}

impl KeyServiceLifecycle {
    fn try_default() -> anyhow::Result<Self> {
        Ok(Self {
            socket_path: try_default_socket_path()?,
            binary_path: resolve_key_service_binary(),
            log_path: home_relative(".easynet/logs/easynet-keyring.log")?,
            lease_path: home_relative(".easynet/keyring.start.lock")?,
            owner_path: try_default_owner_path()?,
            ready_timeout: DEFAULT_READY_TIMEOUT,
        })
    }

    #[cfg(test)]
    fn try_default_with_home(home: Option<&std::ffi::OsStr>) -> anyhow::Result<Self> {
        Ok(Self {
            socket_path: super::home_relative_from(super::DEFAULT_KEYRING_SOCKET_REL, home)?,
            binary_path: PathBuf::from("easynet-keyring"),
            log_path: super::home_relative_from(".easynet/logs/easynet-keyring.log", home)?,
            lease_path: super::home_relative_from(".easynet/keyring.start.lock", home)?,
            owner_path: super::home_relative_from(".easynet/keyring.owner.json", home)?,
            ready_timeout: DEFAULT_READY_TIMEOUT,
        })
    }

    fn ensure_running(
        &self,
        state: &mut KeyServiceManagerState,
        intent: KeyServiceOwnershipIntent,
    ) -> anyhow::Result<KeyServiceStartup> {
        let client = KeyringClient::new(&self.socket_path).with_timeout(Duration::from_secs(2));
        if probe_ready(&client)? {
            if self.ready_service_is_attachable(intent)? {
                return Ok(KeyServiceStartup::Attached);
            }
            self.retire_orphaned_ready_service()?;
        }

        reap_stale_lease(&self.lease_path)?;
        let lease_wait_deadline = Instant::now() + self.ready_timeout;
        loop {
            match StartupLease::try_acquire(&self.lease_path)? {
                Some(_lease) => {
                    // A preceding process can become ready between our first
                    // probe and lease acquisition.
                    if probe_ready(&client)? {
                        if self.ready_service_is_attachable(intent)? {
                            return Ok(KeyServiceStartup::Attached);
                        }
                        self.retire_orphaned_ready_service()?;
                    }
                    anyhow::ensure!(
                        state.child.is_none(),
                        "refuse to spawn a second owned daemon key service process"
                    );
                    self.remove_stale_transport()?;
                    state.child = Some(self.spawn()?);

                    // Readiness owns a fresh budget. Time spent waiting for
                    // another process's startup lease never consumes the
                    // spawned process's readiness deadline.
                    let readiness_deadline = Instant::now() + self.ready_timeout;
                    if let Err(readiness_error) =
                        self.wait_ready(state, &client, readiness_deadline)
                    {
                        let cleanup = state.terminate_owned_child().err();
                        return Err(combine_lifecycle_errors(readiness_error, cleanup));
                    }
                    return Ok(KeyServiceStartup::Spawned);
                }
                None if probe_ready(&client)? => {
                    if self.ready_service_is_attachable(intent)? {
                        return Ok(KeyServiceStartup::Attached);
                    }
                    self.retire_orphaned_ready_service()?;
                }
                None if Instant::now() >= lease_wait_deadline => {
                    anyhow::bail!(
                        "daemon key service startup lease remained busy at {}",
                        self.lease_path.display()
                    );
                }
                None => std::thread::sleep(PROBE_INTERVAL),
            }
        }
    }

    fn ready_service_is_attachable(
        &self,
        intent: KeyServiceOwnershipIntent,
    ) -> anyhow::Result<bool> {
        let owner = load_key_service_owner_fact(&self.owner_path)?.ok_or_else(|| {
            anyhow::anyhow!(
                "healthy key service at {} has no lifecycle owner fact",
                self.socket_path.display()
            )
        })?;
        if owner.owner_pid == std::process::id() {
            return Ok(true);
        }
        if !process_is_alive(owner.owner_pid) {
            return Ok(false);
        }
        match intent {
            KeyServiceOwnershipIntent::Bootstrap => Ok(true),
            KeyServiceOwnershipIntent::Daemon => anyhow::bail!(
                "key service pid={} is owned by live process pid={}; runtime daemon refuses \
                 foreign signer custody",
                owner.key_service_pid,
                owner.owner_pid
            ),
        }
    }

    fn retire_orphaned_ready_service(&self) -> anyhow::Result<()> {
        let owner = load_key_service_owner_fact(&self.owner_path)?.ok_or_else(|| {
            anyhow::anyhow!(
                "daemon key service at {} is ready but has no owner fact",
                self.socket_path.display()
            )
        })?;
        terminate_foreign_key_service(owner.key_service_pid).with_context(|| {
            format!(
                "terminate orphaned key service pid={} dead_owner_pid={} before owned restart",
                owner.key_service_pid, owner.owner_pid
            )
        })?;
        let _ = std::fs::remove_file(&self.owner_path);
        self.remove_stale_transport()
    }

    fn remove_stale_transport(&self) -> anyhow::Result<()> {
        #[cfg(unix)]
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path).with_context(|| {
                format!(
                    "remove stale daemon key service socket {}",
                    self.socket_path.display()
                )
            })?;
        }
        Ok(())
    }

    fn spawn(&self) -> anyhow::Result<Child> {
        if let Some(parent) = self.log_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("create daemon key service log dir {}", parent.display())
            })?;
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .with_context(|| format!("open daemon key service log {}", self.log_path.display()))?;

        let mut command = Command::new(&self.binary_path);
        // The child is session-detached so arbitrary daemon descendants never
        // inherit it. It still has one lifecycle owner: this daemon process.
        // The keyring watches that explicit owner PID and exits if a crash
        // bypasses the daemon's orderly shutdown guard.
        command.env(KEY_SERVICE_OWNER_PID_ENV, std::process::id().to_string());
        command.stdin(Stdio::null());
        if let Ok(stdout) = log.try_clone() {
            command.stdout(Stdio::from(stdout));
        }
        command.stderr(Stdio::from(log));

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            unsafe {
                command.pre_exec(|| {
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

        command
            .spawn()
            .with_context(|| format!("spawn daemon key service at {}", self.binary_path.display()))
    }

    fn wait_ready(
        &self,
        state: &mut KeyServiceManagerState,
        client: &KeyringClient,
        deadline: Instant,
    ) -> anyhow::Result<()> {
        loop {
            if probe_ready(client)? {
                return Ok(());
            }
            let child_status = state
                .child
                .as_mut()
                .context("daemon key service child ownership disappeared before readiness")?
                .try_wait()
                .context("inspect daemon key service process during readiness")?;
            if let Some(status) = child_status {
                state.child.take();
                anyhow::bail!(
                    "daemon key service exited before readiness: status={status} (see {})",
                    self.log_path.display()
                );
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "daemon key service did not become ready within {}s (see {})",
                    self.ready_timeout.as_secs(),
                    self.log_path.display()
                );
            }
            std::thread::sleep(PROBE_INTERVAL);
        }
    }
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM)
    )
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    true
}

#[derive(Debug, serde::Deserialize)]
struct KeyServiceOwnerFact {
    protocol_version: u32,
    key_service_pid: u32,
    owner_pid: u32,
}

fn load_key_service_owner_fact(path: &Path) -> anyhow::Result<Option<KeyServiceOwnerFact>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read key service owner fact {}", path.display()));
        }
    };
    let fact: KeyServiceOwnerFact = serde_json::from_slice(&bytes)
        .with_context(|| format!("decode key service owner fact {}", path.display()))?;
    anyhow::ensure!(
        fact.protocol_version == 1,
        "unsupported key service owner fact version {} at {}",
        fact.protocol_version,
        path.display()
    );
    anyhow::ensure!(
        fact.key_service_pid > 1 && fact.owner_pid > 1,
        "invalid key service owner fact at {}",
        path.display()
    );
    Ok(Some(fact))
}

#[cfg(unix)]
fn terminate_foreign_key_service(pid: u32) -> anyhow::Result<()> {
    let pid = libc::pid_t::try_from(pid)
        .map_err(|_| anyhow::anyhow!("key service pid is not representable as pid_t: {pid}"))?;
    if unsafe { libc::kill(pid, libc::SIGTERM) } == -1 {
        let error = std::io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::ESRCH)) {
            return Err(error).context("send SIGTERM to foreign key service");
        }
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if unsafe { libc::kill(pid, 0) } == -1
            && matches!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            )
        {
            return Ok(());
        }
        std::thread::sleep(PROBE_INTERVAL);
    }
    if unsafe { libc::kill(pid, libc::SIGKILL) } == -1 {
        let error = std::io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::ESRCH)) {
            return Err(error).context("send SIGKILL to foreign key service");
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn terminate_foreign_key_service(_pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("foreign key-service retirement is unsupported on this platform")
}

fn supervisor_backoff(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(6);
    let millis = 100u64.saturating_mul(1u64 << shift);
    Duration::from_millis(millis).min(SUPERVISOR_MAX_BACKOFF)
}

fn probe_ready(client: &KeyringClient) -> anyhow::Result<bool> {
    match client.health() {
        Ok(_) => Ok(true),
        Err(SelfIdentityError::DaemonOffline { .. }) => Ok(false),
        Err(error) => Err(anyhow::anyhow!(
            "daemon key service endpoint is present but unhealthy: {error}"
        )),
    }
}

struct StartupLease {
    path: PathBuf,
    _file: File,
}

impl StartupLease {
    fn try_acquire(path: &Path) -> anyhow::Result<Option<Self>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create key service state dir {}", parent.display()))?;
        }
        match OpenOptions::new().create_new(true).write(true).open(path) {
            Ok(file) => {
                let mut lease = Self {
                    path: path.to_path_buf(),
                    _file: file,
                };
                writeln!(lease._file, "{}", std::process::id()).with_context(|| {
                    format!("write key service startup lease {}", path.display())
                })?;
                Ok(Some(lease))
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
            Err(error) => Err(error)
                .with_context(|| format!("acquire key service startup lease {}", path.display())),
        }
    }
}

impl Drop for StartupLease {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn reap_stale_lease(path: &Path) -> anyhow::Result<()> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect key service startup lease {}", path.display()));
        }
    };
    let stale = metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age >= STARTUP_LEASE_STALE_AFTER);
    if stale {
        std::fs::remove_file(path).with_context(|| {
            format!("remove stale key service startup lease {}", path.display())
        })?;
    }
    Ok(())
}

fn resolve_key_service_binary() -> PathBuf {
    const KEY_SERVICE_BINARY: &str = "easynet-keyring";
    std::env::var_os("EASYNET_KEYRING_BIN")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(|parent| parent.join(KEY_SERVICE_BINARY)))
        })
        .unwrap_or_else(|| PathBuf::from(KEY_SERVICE_BINARY))
}

#[cfg(test)]
mod tests {
    use super::{
        supervisor_backoff, KeyServiceLifecycle, KeyServiceLifecycleState, KeyServiceManager,
        KeyServiceManagerPhase, KeyServiceManagerState, KeyServiceOwnershipIntent, KeyringClient,
        StartupLease, SUPERVISOR_MAX_BACKOFF,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn startup_lease_is_single_writer_and_released_on_drop() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("key-service.start.lock");

        let first = StartupLease::try_acquire(&path)
            .expect("first acquire")
            .expect("first writer owns lease");
        assert!(
            StartupLease::try_acquire(&path)
                .expect("contended acquire")
                .is_none(),
            "a second process must attach/wait instead of spawning another service"
        );

        drop(first);
        assert!(
            StartupLease::try_acquire(&path)
                .expect("acquire after release")
                .is_some(),
            "dropping the lifecycle owner must release the startup lease"
        );
    }

    #[test]
    fn supervisor_restart_backoff_is_exponential_and_bounded() {
        assert_eq!(supervisor_backoff(1), Duration::from_millis(100));
        assert_eq!(supervisor_backoff(2), Duration::from_millis(200));
        assert_eq!(supervisor_backoff(3), Duration::from_millis(400));
        assert_eq!(supervisor_backoff(100), SUPERVISOR_MAX_BACKOFF);
    }

    #[test]
    fn manager_phase_is_the_single_observable_lifecycle_state() {
        let mut state = KeyServiceManagerState::default();
        assert_eq!(state.snapshot(), None);

        state.phase = KeyServiceManagerPhase::Attached;
        assert_eq!(state.snapshot(), Some(KeyServiceLifecycleState::Attached));

        state.phase = KeyServiceManagerPhase::Running;
        assert_eq!(state.snapshot(), Some(KeyServiceLifecycleState::Running));

        state.phase = KeyServiceManagerPhase::Restarting { attempt: 7 };
        assert_eq!(
            state.snapshot(),
            Some(KeyServiceLifecycleState::Restarting { attempt: 7 })
        );

        state.phase = KeyServiceManagerPhase::Failed {
            message: "probe failed".into(),
        };
        assert_eq!(
            state.snapshot(),
            Some(KeyServiceLifecycleState::Failed {
                message: "probe failed".into(),
            })
        );
    }

    #[test]
    fn lifecycle_default_rejects_missing_home_before_cwd_paths() {
        let error = KeyServiceLifecycle::try_default_with_home(None)
            .expect_err("missing HOME must fail before key-service lifecycle paths are built");

        assert!(
            error
                .to_string()
                .contains("HOME is required for daemon key-service custody paths"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn lifecycle_default_rejects_blank_home_before_cwd_paths() {
        let error = KeyServiceLifecycle::try_default_with_home(Some(std::ffi::OsStr::new("")))
            .expect_err("blank HOME must fail before key-service lifecycle paths are built");

        assert!(
            error
                .to_string()
                .contains("HOME is required for daemon key-service custody paths"),
            "unexpected error: {error:#}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn owned_process_is_killed_and_reaped_before_ownership_is_released() {
        use std::process::{Command, Stdio};

        let child = Command::new("sh")
            .args(["-c", "sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn owned test child");
        let mut state = KeyServiceManagerState {
            child: Some(child),
            ..KeyServiceManagerState::default()
        };

        state
            .terminate_owned_child()
            .expect("terminate and reap owned child");

        assert!(
            state.child.is_none(),
            "ownership may be released only after the child is reaped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn daemon_shutdown_reaps_only_its_owned_key_service_and_disables_restart() {
        use std::path::PathBuf;
        use std::process::{Command, Stdio};
        use std::sync::Mutex;

        let directory = tempfile::tempdir().expect("tempdir");
        let child = Command::new("sh")
            .args(["-c", "sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn owned test child");
        let manager = KeyServiceManager {
            lifecycle: KeyServiceLifecycle {
                socket_path: directory.path().join("key-service.sock"),
                binary_path: PathBuf::from("unused"),
                log_path: directory.path().join("key-service.log"),
                lease_path: directory.path().join("key-service.start.lock"),
                owner_path: directory.path().join("key-service.owner.json"),
                ready_timeout: Duration::from_secs(1),
            },
            state: Mutex::new(KeyServiceManagerState {
                phase: KeyServiceManagerPhase::Running,
                child: Some(child),
                ..KeyServiceManagerState::default()
            }),
        };

        manager.shutdown().expect("daemon-owned child stops");

        let state = manager.lock_state();
        assert!(state.shutdown_requested);
        assert!(state.child.is_none());
        assert_eq!(state.snapshot(), None);
    }

    #[cfg(unix)]
    #[test]
    fn bootstrap_shutdown_reaps_owned_service_without_disabling_restart() {
        use std::path::PathBuf;
        use std::process::{Command, Stdio};
        use std::sync::Mutex;

        let directory = tempfile::tempdir().expect("tempdir");
        let child = Command::new("sh")
            .args(["-c", "sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn owned test child");
        let manager = KeyServiceManager {
            lifecycle: KeyServiceLifecycle {
                socket_path: directory.path().join("key-service.sock"),
                binary_path: PathBuf::from("unused"),
                log_path: directory.path().join("key-service.log"),
                lease_path: directory.path().join("key-service.start.lock"),
                owner_path: directory.path().join("key-service.owner.json"),
                ready_timeout: Duration::from_secs(1),
            },
            state: Mutex::new(KeyServiceManagerState {
                phase: KeyServiceManagerPhase::Running,
                child: Some(child),
                ..KeyServiceManagerState::default()
            }),
        };

        manager
            .shutdown_bootstrap()
            .expect("bootstrap-owned child stops");

        let state = manager.lock_state();
        assert!(
            !state.shutdown_requested,
            "bootstrap shutdown must leave the manager reusable"
        );
        assert!(state.child.is_none());
        assert_eq!(state.snapshot(), None);
    }

    #[cfg(unix)]
    #[test]
    fn lifecycle_spawn_boundary_owns_stale_socket_cleanup() {
        use std::os::unix::net::UnixListener;
        use std::path::PathBuf;
        use std::sync::Mutex;

        let directory = tempfile::tempdir().expect("tempdir");
        let socket_path = directory.path().join("key-service.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind stale socket");
        let manager = KeyServiceManager {
            lifecycle: KeyServiceLifecycle {
                socket_path: socket_path.clone(),
                binary_path: PathBuf::from("unused"),
                log_path: directory.path().join("key-service.log"),
                lease_path: directory.path().join("key-service.start.lock"),
                owner_path: directory.path().join("key-service.owner.json"),
                ready_timeout: Duration::from_secs(1),
            },
            state: Mutex::new(KeyServiceManagerState::default()),
        };

        manager
            .lifecycle
            .remove_stale_transport()
            .expect("lifecycle manager removes stale key-service socket");

        assert!(
            !socket_path.exists(),
            "service process exit must leave socket cleanup to the next lifecycle spawn boundary"
        );
        drop(listener);
    }

    #[cfg(unix)]
    #[test]
    fn healthy_service_with_live_foreign_owner_is_attached_without_supervision_or_retirement() {
        use std::io::{Read as _, Write as _};
        use std::os::unix::net::UnixListener;
        use std::path::PathBuf;
        use std::process::{Command, Stdio};
        use std::sync::Mutex;

        let directory = tempfile::tempdir().expect("tempdir");
        let socket_path = directory.path().join("key-service.sock");
        let owner_path = directory.path().join("key-service.owner.json");
        let listener = UnixListener::bind(&socket_path).expect("bind fake key service");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept health probe");
            let mut length = [0_u8; 4];
            stream.read_exact(&mut length).expect("read request length");
            let mut request = vec![0_u8; u32::from_be_bytes(length) as usize];
            stream.read_exact(&mut request).expect("read request body");
            let response = br#"{"result":"health","protocol_version":2}"#;
            stream
                .write_all(&(response.len() as u32).to_be_bytes())
                .expect("write response length");
            stream.write_all(response).expect("write response body");
        });

        let mut foreign_owner = Command::new("sh")
            .args(["-c", "sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn foreign owner");
        let mut foreign_service = Command::new("sh")
            .args(["-c", "sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn foreign service");
        std::fs::write(
            &owner_path,
            serde_json::json!({
                "protocol_version": 1,
                "key_service_pid": foreign_service.id(),
                "owner_pid": foreign_owner.id(),
            })
            .to_string(),
        )
        .expect("write owner fact");

        let manager = Box::leak(Box::new(KeyServiceManager {
            lifecycle: KeyServiceLifecycle {
                socket_path,
                binary_path: PathBuf::from("unused"),
                log_path: directory.path().join("unused.log"),
                lease_path: directory.path().join("unused.lock"),
                owner_path,
                ready_timeout: Duration::from_secs(1),
            },
            state: Mutex::new(KeyServiceManagerState::default()),
        }));

        let startup = manager
            .ensure_running(KeyServiceOwnershipIntent::Bootstrap)
            .expect("consumer attaches to live daemon-owned service");
        server.join().expect("fake key service joins");
        assert_eq!(startup, super::KeyServiceStartup::Attached);
        {
            let state = manager.lock_state();
            assert_eq!(state.phase, KeyServiceManagerPhase::Attached);
            assert!(state.child.is_none());
            assert!(
                state.supervisor.is_none(),
                "an attached consumer must not supervise another process's service"
            );
        }
        assert!(
            super::process_is_alive(foreign_service.id()),
            "attachment must not terminate the live foreign service"
        );

        foreign_service.kill().expect("kill foreign service");
        foreign_service.wait().expect("reap foreign service");
        foreign_owner.kill().expect("kill foreign owner");
        foreign_owner.wait().expect("reap foreign owner");
    }

    #[cfg(unix)]
    #[test]
    fn daemon_refuses_signer_custody_owned_by_another_live_process() {
        use std::path::PathBuf;
        use std::process::{Command, Stdio};

        let directory = tempfile::tempdir().expect("tempdir");
        let owner_path = directory.path().join("key-service.owner.json");
        let lifecycle = KeyServiceLifecycle {
            socket_path: directory.path().join("key-service.sock"),
            binary_path: PathBuf::from("unused"),
            log_path: directory.path().join("unused.log"),
            lease_path: directory.path().join("unused.lock"),
            owner_path: owner_path.clone(),
            ready_timeout: Duration::from_secs(1),
        };
        let mut foreign_owner = Command::new("sh")
            .args(["-c", "sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn foreign owner");
        std::fs::write(
            owner_path,
            serde_json::json!({
                "protocol_version": 1,
                "key_service_pid": foreign_owner.id(),
                "owner_pid": foreign_owner.id(),
            })
            .to_string(),
        )
        .expect("write owner fact");

        assert!(
            lifecycle
                .ready_service_is_attachable(KeyServiceOwnershipIntent::Bootstrap)
                .expect("bootstrap may attach"),
            "bootstrap access may consume daemon-owned custody"
        );
        let error = lifecycle
            .ready_service_is_attachable(KeyServiceOwnershipIntent::Daemon)
            .expect_err("daemon must reject foreign custody");
        assert!(
            error.to_string().contains("refuses foreign signer custody"),
            "unexpected error: {error:#}"
        );

        foreign_owner.kill().expect("kill foreign owner");
        foreign_owner.wait().expect("reap foreign owner");
    }

    #[cfg(unix)]
    #[test]
    fn unhealthy_owned_service_is_reaped_before_supervised_restart() {
        use std::io::{Read as _, Write as _};
        use std::os::unix::net::UnixListener;
        use std::path::PathBuf;
        use std::process::{Command, Stdio};
        use std::sync::Mutex;

        let directory = tempfile::tempdir().expect("tempdir");
        let socket_path = directory.path().join("key-service.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind fake key service");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept health probe");
            let mut length = [0_u8; 4];
            stream.read_exact(&mut length).expect("read request length");
            let mut request = vec![0_u8; u32::from_be_bytes(length) as usize];
            stream.read_exact(&mut request).expect("read request body");

            // Syntactically valid framing with the wrong health variant: the
            // endpoint is reachable but protocol-unhealthy.
            let response = br#"{"result":"ok"}"#;
            stream
                .write_all(&(response.len() as u32).to_be_bytes())
                .expect("write response length");
            stream.write_all(response).expect("write response body");
        });

        let child = Command::new("sh")
            .args(["-c", "sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn owned test child");
        let manager = KeyServiceManager {
            lifecycle: KeyServiceLifecycle {
                socket_path,
                binary_path: PathBuf::from("unused"),
                log_path: directory.path().join("unused.log"),
                lease_path: directory.path().join("unused.lock"),
                owner_path: directory.path().join("unused.owner.json"),
                ready_timeout: Duration::from_secs(1),
            },
            state: Mutex::new(KeyServiceManagerState::default()),
        };
        let mut state = KeyServiceManagerState {
            child: Some(child),
            ..KeyServiceManagerState::default()
        };

        let error = manager
            .try_transition_to_running(&mut state, 1, KeyServiceOwnershipIntent::Daemon)
            .expect_err("an unhealthy owned endpoint must fail this transition");
        server.join().expect("fake key service joins");

        assert!(error.to_string().contains("unhealthy"));
        assert!(
            state.child.is_none(),
            "the supervisor may restart only after the unhealthy child is reaped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn readiness_reports_owned_child_exit_without_waiting_for_timeout() {
        use std::path::PathBuf;
        use std::process::{Command, Stdio};

        let directory = tempfile::tempdir().expect("tempdir");
        let lifecycle = KeyServiceLifecycle {
            socket_path: directory.path().join("key-service.sock"),
            binary_path: PathBuf::from("unused"),
            log_path: directory.path().join("key-service.log"),
            lease_path: directory.path().join("key-service.start.lock"),
            owner_path: directory.path().join("key-service.owner.json"),
            ready_timeout: Duration::from_secs(5),
        };
        let child = Command::new("sh")
            .args(["-c", "exit 7"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn exiting child");
        let mut state = KeyServiceManagerState {
            child: Some(child),
            ..KeyServiceManagerState::default()
        };
        let client =
            KeyringClient::new(&lifecycle.socket_path).with_timeout(Duration::from_millis(100));
        let started = Instant::now();

        let error = lifecycle
            .wait_ready(&mut state, &client, started + Duration::from_secs(5))
            .expect_err("exited child cannot become ready");

        assert!(error.to_string().contains("exited before readiness"));
        assert!(
            state.child.is_none(),
            "exited child must be reaped from state"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "child exit should fail immediately, not at readiness timeout"
        );
    }
}

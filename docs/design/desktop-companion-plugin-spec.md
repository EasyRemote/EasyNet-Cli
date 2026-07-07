# Desktop Companion Plugin SPEC

**Status:** proposed.
**Date:** 2026-07-07.
**Target case:** ship the existing macOS `EasyNetMenuBar` and Windows
`EasyNetTray` as first-class EasyNet desktop companion plugins.

## 1. Problem

EasyNet already has platform companion applications:

- macOS: `platforms/macos/EasyNetMenuBar`
- Windows: `platforms/windows/EasyNetTray`

They expose local operator affordances such as:

- menu bar or tray status for `easynet-daemon`
- clipboard history popup
- global hotkey
- local user session UI

Today these companions are not part of the EasyNet plugin lifecycle. macOS is
installed through `tools/scripts/install-macos-menubar.sh`, which writes a
LaunchAgent. Windows is a standalone tray app. The daemon plugin system can
load ability plugins, but it cannot express or supervise "desktop UI process
owned by the logged-in user's graphical session".

The gap creates four product issues:

1. `easynet plugin list` cannot show whether the companion is installed,
   enabled, running, stale, or unsupported.
2. `easynet runtime start` has no well-defined hook to ensure the companion is
   running after daemon Ready.
3. `easynet runtime stop` has no policy for whether the companion should stay
   alive or stop.
4. Platform behavior is script-owned, not typed, so macOS, Windows, and Linux
   cannot converge on one lifecycle model.

## 2. Decision

Add a new plugin class: `desktop_companion`.

This class is part of the EasyNet-Cli daemon/plugin product boundary, but it is
not an Axon Invocation primitive and not a normal `AbilityImpl` provider.

```text
PluginPackage
  -> PluginLoadPlan
  -> DesktopCompanionPlan
  -> DesktopCompanionSupervisor
  -> OS user-session launcher
  -> CompanionObservedStatus
```

The existing ability plugin path remains separate:

```text
PluginPackage
  -> PluginLoadPlan
  -> PluginRuntimeHost
  -> DaemonPluginBinder
  -> AxonAbilityCatalog
```

The two paths share package discovery, platform filtering, env gates, CLI
surface, and status projection. They do not share runtime semantics.

## 3. Boundary Rules

### 3.1 What owns this feature

EasyNet-Cli owns desktop companion plugins because they manage local device UX,
local user session integration, plugin lifecycle, daemon status observation,
and local resource access.

Axon does not own this feature. Desktop companions do not define Invocation,
admission, receipt semantics, stream/bidi protocol, URA parsing, or federation
wire rules.

### 3.2 What a desktop companion is

A desktop companion is an installable plugin package that declares a local UI
process supervised by the host OS user session.

It may expose zero, one, or many abilities. The companion process itself is not
an AbilityDescriptor. If it does expose callable behavior, those abilities must
still enter the daemon through the normal descriptor and `AbilityImpl` path.

### 3.3 What it is not

A desktop companion is not:

- a caller/callee identity
- an authority root
- a daemon control socket
- an Axon runtime
- an Invocation sidecar process
- a replacement for `easynet-daemon`
- a service that must exist in headless/server environments

## 4. Goals

1. Represent macOS menu bar and Windows tray companions as installable plugins.
2. Add typed manifest metadata for user-session UI processes.
3. Add a cross-platform supervisor interface with macOS, Windows, and Linux
   adapters.
4. Add lifecycle states for installed/enabled/running/stale/error/unsupported.
5. Integrate companion status into `easynet plugin list` and
   `easynet runtime status`.
6. Add best-effort startup after daemon Ready without blocking daemon boot.
7. Preserve the current behavior that `runtime stop` does not kill the UI
   companion by default.
8. Implement the existing `EasyNetMenuBar` as the first target case.

## 5. Non-goals

1. Do not make desktop companion processes part of Axon protocol.
2. Do not require a graphical session for `easynet-daemon` to boot.
3. Do not force a companion package to declare fake ability metadata.
4. Do not make `runtime stop` uninstall or disable user-session UI.
5. Do not make LaunchAgent, Task Scheduler, or systemd user state the canonical
   runtime truth by itself. Observed process/heartbeat state wins.
6. Do not implement Linux tray UI in the first target case. Linux support should
   classify correctly and report `unsupported_package` or `unsupported_session`
   until a Linux companion package exists.

## 6. Current Code Baseline

Relevant current files:

- `src/daemon/plugins/manifest.rs`
  - Parses `PluginKind::{Declarative, Sidecar, Builtin}`.
  - Requires `entrypoint`, `abilities`, `ability_metadata`, and runtime limits.
- `src/daemon/plugins/load_plan.rs`
  - Produces `PluginLoadStatus`.
  - Filters by `platforms` and env gates.
  - Checks sidecar/declarative executability.
- `src/daemon/plugins/runtime_manager.rs`
  - Registers loaded packages into `AxonAbilityCatalog`.
  - Projects plugin runtime status through `PluginSurfaceProjector`.
- `src/cli/commands/groups/plugin.rs`
  - Provides `list/install/update/remove/activate-realtime`.
- `src/daemon/boot/lifecycle/status.rs`
  - Classifies daemon lifecycle only.
- `tools/scripts/install-macos-menubar.sh`
  - Writes the current LaunchAgent.
- `platforms/macos/EasyNetMenuBar/Sources/EasyNetMenuBar/main.swift`
  - Runs the macOS accessory app.
- `platforms/windows/EasyNetTray/Program.cs`
  - Runs the Windows tray app.

The existing plugin model should not be broken. `desktop_companion` extends the
package model and status surface.

## 7. Manifest Schema

### 7.1 Plugin kind

Add:

```rust
pub enum PluginKind {
    Declarative,
    Sidecar,
    Builtin,
    DesktopCompanion,
}
```

TOML:

```toml
kind = "desktop_companion"
```

### 7.2 Companion section

Add an optional `[companion]` table. It is required when
`kind = "desktop_companion"` and forbidden for other kinds unless explicitly
allowed by a later SPEC.

```toml
[companion]
display_name = "EasyNet Menu Bar"
lifecycle = "user_session"
boot_policy = "ensure_running_after_daemon_ready"
stop_policy = "keep_running"
health = "status_file"
status_file = "state/easynet-menubar.status.json"
```

Fields:

| Field | Values | Meaning |
|---|---|---|
| `display_name` | string | Operator-facing label. |
| `lifecycle` | `user_session` | Process belongs to logged-in desktop session. |
| `boot_policy` | `manual`, `ensure_running_after_daemon_ready` | Whether daemon startup should try to start it after Ready. |
| `stop_policy` | `keep_running`, `stop_on_runtime_stop`, `stop_on_plugin_disable` | Runtime stop behavior. |
| `health` | `process_name`, `status_file`, `local_ipc` | Observation mechanism. |
| `status_file` | relative path | Package or state relative path for heartbeat/status file. Required for `status_file`. |

Initial release supports:

- `lifecycle = "user_session"`
- `boot_policy = "manual" | "ensure_running_after_daemon_ready"`
- `stop_policy = "keep_running" | "stop_on_plugin_disable" | "stop_on_runtime_stop"`
- `health = "process_name" | "status_file"`

`local_ipc` is reserved for a later upgrade.

### 7.3 Platform-specific sections

macOS:

```toml
[companion.macos]
bundle_id = "tech.silan.easynet.menubar"
app_bundle = "dist/macos/EasyNetMenuBar.app"
supervisor = "launch_agent"
launch_agent_label = "tech.silan.easynet.menubar"
session = "aqua"
```

Windows:

```toml
[companion.windows]
exe = "dist/windows/EasyNetTray/EasyNetTray.exe"
supervisor = "startup_task"
task_name = "EasyNetTray"
session = "interactive_desktop"
```

Linux:

```toml
[companion.linux]
exe = "dist/linux/easynet-tray"
supervisor = "systemd_user"
unit_name = "easynet-tray.service"
session = "graphical"
```

Initial target case may omit `[companion.linux]`. If omitted, Linux load status
is `platform_mismatch` or `companion_platform_unsupported`, depending on
whether `platforms` includes `linux`.

### 7.4 Ability metadata for companion packages

For `desktop_companion`, `abilities` and `ability_metadata` are optional.

Valid shape with no abilities:

```toml
schema_version = "1"
id = "easynet.desktop.menubar"
version = "0.1.0"
kind = "desktop_companion"
entrypoint = "platforms/macos/EasyNetMenuBar"
abilities = []
permissions = ["clipboard_read", "clipboard_write", "global_hotkey"]
resources = ["desktop_session", "clipboard"]
platforms = ["macos", "windows"]

[limits]
max_sessions = 1
max_frame_queue = 1

[companion]
display_name = "EasyNet Menu Bar"
lifecycle = "user_session"
boot_policy = "ensure_running_after_daemon_ready"
stop_policy = "keep_running"
health = "status_file"
status_file = "state/easynet-menubar.status.json"

[companion.macos]
bundle_id = "tech.silan.easynet.menubar"
app_bundle = "dist/macos/EasyNetMenuBar.app"
supervisor = "launch_agent"
launch_agent_label = "tech.silan.easynet.menubar"
session = "aqua"

[companion.windows]
exe = "dist/windows/EasyNetTray/EasyNetTray.exe"
supervisor = "startup_task"
task_name = "EasyNetTray"
session = "interactive_desktop"
```

The `entrypoint` field remains for package hash and compatibility but does not
mean "sidecar executable" for this kind.

### 7.5 Artifact integrity

Desktop companion artifacts are part of the plugin's executable surface. The
installer and package hash must include every declared platform artifact, not
only `plugin.toml`, `abilities/`, and `bin/`.

Valid artifact locations for v1:

```text
bin/
dist/
```

The package hash must cover:

- `plugin.toml`
- `abilities/`, when present
- `bin/`, when present
- every path declared by `[companion.<platform>]`, including app bundles,
  executables, helper binaries, icons, plists, and service templates

Rule: an installed package record is invalid if its manifest points at an
artifact path that was not included in the package hash. This prevents a
desktop app bundle from changing without changing the plugin record hash.

For macOS `.app` bundles, hash the full bundle directory recursively. For
Windows, hash the executable plus adjacent runtime files required by the app.
For Linux, hash the executable plus the `.desktop` or systemd unit template if
the package supplies one.

## 8. State Model

### 8.1 Three-layer state

Desktop companion state is a composition of desired, supervisor, and observed
state.

```text
DesiredState
  enabled
  disabled

SupervisorState
  unsupported_platform
  unsupported_session
  not_installed
  installed_disabled
  installed_enabled
  install_error
  enable_error
  disable_error

ObservedState
  unknown
  not_running
  starting
  running
  stale
  exited
  version_mismatch
  health_error
```

The projected operator state is derived:

```text
Disabled
UnsupportedPlatform
UnsupportedSession
NotInstalled
InstalledDisabled
ReadyStopped
Starting
Running
Stale
Error
```

### 8.2 Full state machine

```text
NotInstalled
  -> InstalledDisabled       install()
  -> Error                   install failed

InstalledDisabled
  -> ReadyStopped            enable()
  -> NotInstalled            remove()
  -> Error                   enable failed

ReadyStopped
  -> Starting                start()
  -> InstalledDisabled       disable()
  -> NotInstalled            remove()

Starting
  -> Running                 heartbeat/process observed
  -> Error                   start timeout or supervisor error

Running
  -> Stale                   heartbeat older than threshold
  -> Exited                  process gone
  -> ReadyStopped            stop()
  -> InstalledDisabled       disable()

Stale
  -> Running                 fresh heartbeat
  -> Exited                  process gone
  -> Starting                restart()
  -> InstalledDisabled       disable()

Exited
  -> Starting                restart()
  -> InstalledDisabled       disable()

Error
  -> ReadyStopped            reconcile() repairs supervisor/install state
  -> Starting                restart() after repairable start failure
  -> InstalledDisabled       disable()
  -> NotInstalled            remove()

UnsupportedPlatform
  -> terminal for current host

UnsupportedSession
  -> ReadyStopped            graphical session appears
  -> terminal for headless boot attempt
```

### 8.3 Terminal and non-terminal states

Terminal for one plan evaluation:

- `UnsupportedPlatform`
- `UnsupportedSession`
- `Disabled`

Non-terminal:

- `ReadyStopped`
- `Starting`
- `Running`
- `Stale`
- `Exited`
- `Error`

`Error` is not terminal globally. A later `reconcile()` may repair it.

### 8.4 Indexed package versus OS installation

Do not collapse these facts:

```text
Package indexed        plugin package is present in builtin or installed index
Companion installed    platform artifact has been copied to user app dir
Supervisor installed   LaunchAgent / Run key / user unit exists
Supervisor enabled     OS should start it for this user session
Observed running       companion process/heartbeat is live
```

A builtin desktop companion package can be `Package indexed` while its OS
installation is still `NotInstalled`. `easynet plugin list` must show both
facts. This is required for the target case: `easynet.desktop.menubar` may be
compiled into the release package but still require per-user enable/install of
the LaunchAgent and app bundle.

### 8.5 Stale threshold

Initial default:

- heartbeat interval: 15 seconds
- stale threshold: 60 seconds
- start timeout: 10 seconds for immediate process observation

For initial macOS implementation, if no heartbeat exists yet, process presence
plus LaunchAgent status is acceptable. The target state should still include a
status file heartbeat so future status can distinguish "same named process" from
"this companion build and device".

## 9. Status JSON

Add companion status to plugin surfaces and runtime status.

Package-level plugin status:

```json
{
  "package_id": "easynet.desktop.menubar",
  "package_version": "0.1.0",
  "kind": "desktop_companion",
  "planned_load_status": "loaded",
  "daemon_runtime_status": "companion_running",
  "ability_count": 0,
  "runtime_published": false,
  "invokable": false,
  "companion": {
    "display_name": "EasyNet Menu Bar",
    "platform": "macos",
    "desired_state": "enabled",
    "supervisor_state": "installed_enabled",
    "observed_state": "running",
    "projected_state": "running",
    "pid": 12345,
    "version": "0.1.0",
    "last_seen_unix_ms": 1783411200000,
    "launch_method": "launch_agent",
    "health": "status_file"
  }
}
```

Runtime status extension:

```json
{
  "runtime_status": "running",
  "daemon": {},
  "desktop_companions": [
    {
      "id": "easynet.desktop.menubar",
      "state": "running",
      "platform": "macos",
      "desired_state": "enabled",
      "supervisor_state": "installed_enabled",
      "observed_state": "running"
    }
  ]
}
```

## 10. Rust Interfaces

### 10.1 Manifest model

Add:

```rust
pub enum PluginKind {
    Declarative,
    Sidecar,
    Builtin,
    DesktopCompanion,
}

pub struct PluginCompanionManifest {
    display_name: String,
    lifecycle: CompanionLifecycle,
    boot_policy: CompanionBootPolicy,
    stop_policy: CompanionStopPolicy,
    health: CompanionHealthKind,
    status_file: Option<String>,
    macos: Option<MacOsCompanionSpec>,
    windows: Option<WindowsCompanionSpec>,
    linux: Option<LinuxCompanionSpec>,
}
```

### 10.2 Plan model

Do not overload `PluginLoadStatus::Loaded` alone. Add companion-specific
metadata.

```rust
pub enum PluginLoadStatus {
    Loaded,
    DisabledByEnv { env_var: &'static str },
    PlatformMismatch { current: String },
    NotLoadableInThisRelease,
    MissingEntrypoint { path: String },
    EntrypointNotExecutable { path: String },
    MissingBuiltinBinding,
    CompanionUnsupportedPlatform { current: String },
    CompanionUnsupportedSession { reason: String },
    CompanionInvalidSpec { reason: String },
}

pub struct DesktopCompanionPlan {
    package_id: String,
    package_version: String,
    platform: String,
    spec: PlatformCompanionSpec,
    boot_policy: CompanionBootPolicy,
    stop_policy: CompanionStopPolicy,
}
```

`PluginLoadPlanEntry` should optionally carry `companion_plan`.

### 10.3 Supervisor trait

```rust
pub trait DesktopCompanionSupervisor {
    fn platform(&self) -> &'static str;
    fn probe_session(&self) -> CompanionSessionStatus;
    fn install(&self, plan: &DesktopCompanionPlan) -> anyhow::Result<CompanionActionReport>;
    fn enable(&self, plan: &DesktopCompanionPlan) -> anyhow::Result<CompanionActionReport>;
    fn disable(&self, plan: &DesktopCompanionPlan) -> anyhow::Result<CompanionActionReport>;
    fn remove(&self, plan: &DesktopCompanionPlan) -> anyhow::Result<CompanionActionReport>;
    fn start(&self, plan: &DesktopCompanionPlan) -> anyhow::Result<CompanionActionReport>;
    fn stop(&self, plan: &DesktopCompanionPlan) -> anyhow::Result<CompanionActionReport>;
    fn status(&self, plan: &DesktopCompanionPlan) -> anyhow::Result<CompanionStatus>;
}
```

Support object:

```rust
pub struct DesktopCompanionManager {
    planner: DesktopCompanionPlanner,
    supervisor: Box<dyn DesktopCompanionSupervisor + Send + Sync>,
    state_store: DesktopCompanionStateStore,
}
```

### 10.4 State store

File:

```text
~/.easynet/companions/state.toml
```

Example:

```toml
[[companion]]
id = "easynet.desktop.menubar"
version = "0.1.0"
desired_state = "enabled"
last_action = "start"
last_action_unix_ms = 1783411200000
last_error = ""
```

This store is not runtime truth. It is desired-state and last-action memory.
Observed state is always re-probed.

## 11. Platform Adapters

### 11.1 macOS adapter

Module:

```text
src/daemon/plugins/companion/macos.rs
```

Supervisor:

- writes `~/Library/LaunchAgents/<label>.plist`
- app bundle under `~/.easynet/apps/<App>.app`
- `LimitLoadToSessionType = Aqua`
- `RunAtLoad = true`
- `KeepAlive = false` for initial release
- starts via `launchctl bootstrap` and `launchctl kickstart`
- stops via `launchctl bootout` or app termination if pid known

Important macOS rule: a LaunchAgent that runs `/usr/bin/open` supervises only
the short-lived `open` process, not the app process after LaunchServices hands
off the bundle. Therefore the v1 adapter must prefer a LaunchAgent whose
`ProgramArguments` point at the app bundle executable directly:

```text
~/.easynet/apps/EasyNetMenuBar.app/Contents/MacOS/EasyNetMenuBar
```

Using `/usr/bin/open -g <app>` is allowed only as a developer fallback and must
not be treated as supervisor truth. In either mode, observed running state comes
from the companion status file or process observation, not from the LaunchAgent
job alone.

Session probe:

- `cfg(target_os = "macos")`
- user id available
- `launchctl print gui/<uid>` succeeds
- `Aqua` session assumed present when bootstrap to `gui/<uid>` works

Observed status:

- preferred: companion status file under `~/.easynet/companions/<id>/status.json`
- fallback: process observation by bundle id or executable name

### 11.2 Windows adapter

Module:

```text
src/daemon/plugins/companion/windows.rs
```

Supervisor options:

- first implementation: Startup folder shortcut or registry Run key
- later: Task Scheduler for richer status

Initial target should use the simplest durable approach:

- install exe under `%USERPROFILE%\.easynet\apps\EasyNetTray`
- enable via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
- start via `CreateProcess`
- stop via pid from status file or process name fallback

Session probe:

- `cfg(target_os = "windows")`
- interactive user session available

Observed status:

- preferred: status file under `%USERPROFILE%\.easynet\companions\<id>\status.json`
- fallback: `Process.GetProcessesByName("EasyNetTray")`

### 11.3 Linux adapter

Module:

```text
src/daemon/plugins/companion/linux.rs
```

Initial release:

- If package does not include Linux spec, report unsupported platform.
- If package includes Linux spec but no graphical session exists, report
  unsupported session.

Future options:

- `systemd --user` service
- XDG autostart `.desktop`
- tray implementation behind AppIndicator or StatusNotifierItem

Session probe:

- `DISPLAY` or `WAYLAND_DISPLAY`
- optionally `systemctl --user is-system-running`

## 12. Lifecycle Integration

### 12.1 Install

`easynet plugin install <path>`:

1. Copy source package into an install transaction directory.
2. Parse and validate package manifest.
3. Compute package hash over the full installable surface, including companion
   artifacts.
4. Validate active index compatibility against the transaction package.
5. If `kind = desktop_companion`, install platform artifacts and supervisor into
   a companion transaction, but do not publish desired state yet.
6. Commit package directory and plugin lock.
7. Commit companion desired state and supervisor enablement.
8. If daemon is running, ask daemon to reload plugin state.
9. If daemon is Ready and boot policy is `ensure_running_after_daemon_ready`,
   call `start` best-effort.

Failure rule:

- Package install plus OS supervisor install must be transactional for
  `desktop_companion`.
- OS supervisor install failure must fail the install for first release.
- If package commit succeeds but companion commit fails, rollback the package
  lock and package directory before returning the error.
- If rollback fails, return a compound error with the package id/version and
  the stale paths that require manual cleanup.
- Later releases may support `--package-only`.

### 12.2 Update

`easynet plugin update <path>`:

1. Stage replacement package and compute full artifact hash.
2. Capture previous package record, desired state, supervisor state, and
   observed status.
3. Install or update platform app artifacts into a versioned staging path.
4. Commit package lock and companion artifacts together.
5. Preserve desired state.
6. If currently running and executable artifact changed, restart through the
   supervisor.
7. If any commit step fails, restore previous package record and supervisor
   target.
8. Report old and new observed status.

### 12.3 Remove

`easynet plugin remove <id> <version>`:

1. If desktop companion, stop if running.
2. Remove OS supervisor entry if it points to that package version.
3. Remove installed package.
4. Remove desired-state record.

Removal must be idempotent. A missing process, missing LaunchAgent, missing Run
key, or missing status file is not an error if the package record is being
removed. A supervisor entry pointing at a different installed version must not
be removed.

### 12.4 Daemon boot

Daemon boot sequence should remain:

```text
start daemon
bind control
build ability registry
bind invocation endpoint
emit Ready
save runtime projection
post-ready noncritical reconcilers
```

Desktop companion `ensure_running` belongs after Ready.

It must not block:

- control socket binding
- invocation endpoint readiness
- local ability registration
- runtime projection persistence

If companion startup fails after Ready, emit an operator event and expose status.
Do not fail daemon boot.

### 12.5 Runtime stop

Default behavior:

- `stop_policy = keep_running`: do not stop companion.
- `stop_policy = stop_on_plugin_disable`: do not stop on runtime stop.
- `stop_policy = stop_on_runtime_stop`: stop after daemon stop stages.

For `EasyNetMenuBar`, use `keep_running` so the menu item remains visible and
can show daemon stopped.

### 12.6 Plugin disable

Add CLI:

```text
easynet plugin enable <id> [--version VERSION]
easynet plugin disable <id> [--version VERSION]
```

For `desktop_companion`:

- `disable` stops process and disables supervisor.
- `enable` enables supervisor and starts if boot policy requests it.

For ability plugins:

- first release may report "enable/disable not supported for this plugin kind"
  unless an existing disable model exists.

### 12.7 Self uninstall

`easynet self uninstall` must:

1. enumerate desktop companion desired-state records
2. stop running companions
3. remove OS supervisor entries
4. remove installed app artifacts under `~/.easynet/apps`
5. remove companion status files

This stage should run before deleting the rest of `~/.easynet`.

## 13. CLI Surface

Extend:

```text
easynet plugin list
easynet plugin install <path>
easynet plugin update <path>
easynet plugin remove <id> <version>
```

Add:

```text
easynet plugin enable <id> [--version VERSION]
easynet plugin disable <id> [--version VERSION]
easynet plugin status <id> [--version VERSION] [--json]
easynet companion list [--json]
easynet companion start <id>
easynet companion stop <id>
easynet companion restart <id>
```

The `companion` group may be added later. Minimum first implementation can keep
everything under `plugin`.

Table columns:

```text
package  version  kind               planned  daemon      companion  supervisor  observed
...      ...      desktop_companion  loaded   n/a         running    enabled     running
```

`daemon` remains ability-runtime registration status. For companion-only
packages, it should render `n/a`, not `not_loaded`.

## 14. Daemon Control Surface

Add daemon-local plugin control abilities:

- `plugin.companion_status`
- `plugin.companion_reconcile`

These are local daemon control-plane surfaces, not public product abilities.
They must be excluded from realm publication and remote advertisement unless a
future SPEC adds explicit AuthorityBinding rules for remote companion control.

Allowed callers in v1:

- local `easynet` CLI
- local EasyNet-Cli SDK / FFI client
- daemon self-reconciliation after Ready

Rejected callers in v1:

- remote devices
- backend-originated cross-device invocation
- plugin sidecars
- unauthenticated local processes outside the existing local IPC boundary

Inputs:

```json
{
  "package_id": "easynet.desktop.menubar",
  "version": "0.1.0"
}
```

Status output:

```json
{
  "kind": "desktop_companion_status",
  "package_id": "easynet.desktop.menubar",
  "version": "0.1.0",
  "platform": "macos",
  "projected_state": "running",
  "desired_state": "enabled",
  "supervisor_state": "installed_enabled",
  "observed_state": "running",
  "pid": 12345,
  "last_seen_unix_ms": 1783411200000,
  "error": null
}
```

Implementation rule: the control ability handlers must call the same
`DesktopCompanionManager` methods used by CLI offline operations. They must not
open-code LaunchAgent, Run key, systemd, or status-file behavior.

## 15. Unified SDK, CLI, and Control-Plane Contract

The control-plane source of truth is one Rust DTO set owned by EasyNet-Cli:

```text
src/protocol/companion_contract.rs
sdk/schemas/desktop-companion-status.schema.json
sdk/schemas/desktop-companion-action.schema.json
```

Initial contract slice implemented in this branch:

- `src/protocol/companion_contract.rs` projects and validates
  `DesktopCompanionStatus` and `DesktopCompanionActionResult`.
- `sdk/schemas/desktop-companion-status.schema.json` defines the stable status
  JSON shape.
- `sdk/schemas/desktop-companion-action.schema.json` defines the stable action
  result JSON shape.

The remaining implementation phases must consume these DTOs instead of
redefining companion status fields in CLI, daemon control handlers, FFI, or SDK
bindings.

The same DTOs must back:

- `easynet plugin list --json`
- `easynet plugin status <id> --json`
- `easynet runtime status --json`
- `plugin.companion_status`
- `plugin.companion_reconcile`
- libeasynet_cli C ABI companion functions
- generated Python / Swift / Java / Go SDK projections

### 15.1 Stable DTOs

`DesktopCompanionStatus`:

```json
{
  "kind": "desktop_companion_status",
  "package_id": "easynet.desktop.menubar",
  "package_version": "0.1.0",
  "display_name": "EasyNet Menu Bar",
  "platform": "macos",
  "desired_state": "enabled",
  "supervisor_state": "installed_enabled",
  "observed_state": "running",
  "projected_state": "running",
  "boot_policy": "ensure_running_after_daemon_ready",
  "stop_policy": "keep_running",
  "health": "status_file",
  "pid": 12345,
  "version": "0.1.0",
  "last_seen_unix_ms": 1783411200000,
  "launch_method": "launch_agent",
  "error": null
}
```

`DesktopCompanionActionResult`:

```json
{
  "kind": "desktop_companion_action_result",
  "package_id": "easynet.desktop.menubar",
  "action": "enable",
  "status_before": {},
  "status_after": {},
  "changed": true,
  "error": null
}
```

All JSON statuses must use snake_case wire strings. CLI table labels may be
friendlier, but JSON must remain stable.

### 15.2 CLI contract

CLI commands should use the local Rust manager directly when daemon is offline
and use daemon-local control abilities when daemon is online. The JSON result
shape must be identical in both paths.

```text
CLI online path:
  easynet plugin status
    -> local invoke plugin.companion_status
    -> DesktopCompanionStatus

CLI offline path:
  easynet plugin status
    -> DesktopCompanionManager::status
    -> DesktopCompanionStatus
```

If online and offline observations disagree, the command should show the local
manager observation and include a warning that daemon plugin state may be stale.

### 15.3 SDK and FFI contract

Axon SDKs do not expose companion lifecycle. EasyNet-Cli SDKs may expose
companion lifecycle because it is daemon/product control-plane behavior.

Minimum C ABI target:

```c
easynet_companion_list(handle, out_json);
easynet_companion_status(handle, package_id, version_or_null, out_json);
easynet_companion_enable(handle, package_id, version_or_null, out_json);
easynet_companion_disable(handle, package_id, version_or_null, out_json);
easynet_companion_start(handle, package_id, version_or_null, out_json);
easynet_companion_stop(handle, package_id, version_or_null, out_json);
```

Language SDKs should wrap those functions or call daemon-local control
abilities through the generic daemon invocation/client transport. They must not
shell out to `launchctl`, registry tools, `systemctl`, or `easynet`.

### 15.4 Schema ownership

The protocol projection module owns wire compatibility. Platform adapters own
OS details only. CLI and SDKs render or wrap DTOs; they do not classify
supervisor or observed state themselves.

## 16. Companion Process Contract

Companion processes should write a status file.

Path:

```text
~/.easynet/companions/<package-id>/status.json
```

Shape:

```json
{
  "schema_version": "1",
  "package_id": "easynet.desktop.menubar",
  "package_version": "0.1.0",
  "app": "EasyNetMenuBar",
  "pid": 12345,
  "started_at_unix_ms": 1783411100000,
  "last_seen_unix_ms": 1783411200000,
  "daemon": {
    "runtime_status": "running",
    "control_accepting": true,
    "invocation_accepting": true
  }
}
```

Write rule:

- update on launch
- update at least every 15 seconds
- atomic write through temp file + rename
- remove or mark terminal on clean exit when possible

The companion should read daemon status through the same local lifecycle/status
source the CLI uses, not by raw process-name checks. If that is too large for
the first Swift/C# step, process-name fallback is acceptable only as a
transitional state.

## 17. Security and Permissions

Desktop companion permissions are local host permissions:

- clipboard read/write
- global hotkey
- notifications
- screen/window access, if future companion needs it

They do not grant Axon invocation authority. If a companion exposes an ability,
that ability must still use normal descriptor, authority, invocation, and
receipt semantics.

Installation must never write system-wide launchers without explicit user
request. First release uses user-level launchers only:

- macOS: `~/Library/LaunchAgents`
- Windows: `HKCU` Run or user startup folder
- Linux: `systemd --user` or XDG user autostart

## 18. Error Semantics

Use typed error classes:

```text
manifest_invalid
platform_unsupported
session_unsupported
supervisor_install_failed
supervisor_enable_failed
supervisor_disable_failed
start_failed
stop_failed
health_stale
version_mismatch
status_file_invalid
```

Rules:

- `platform_unsupported` is not a daemon boot error.
- `session_unsupported` is not a daemon boot error.
- `supervisor_install_failed` fails `plugin install`.
- `start_failed` after daemon Ready is warning/status, not daemon boot failure.
- `health_stale` is degraded status, not automatic remove.

## 19. Target Case: EasyNet Menu Bar

### 19.1 Target package

Add builtin package:

```text
plugins/desktop-menubar/plugin.toml
```

Initial package id:

```text
easynet.desktop.menubar
```

Supported platforms:

- macOS: production target
- Windows: use existing `EasyNetTray` as second target
- Linux: not included in first package unless a tray app is implemented

### 19.2 macOS artifact

Current source:

```text
platforms/macos/EasyNetMenuBar/Sources/EasyNetMenuBar/main.swift
```

Build script:

```text
tools/scripts/build-macos-menubar.sh
```

Migration:

1. Keep the Swift source in `platforms/macos/EasyNetMenuBar`.
2. Build artifact into package dist path during release packaging.
3. Replace `install-macos-menubar.sh` as public install path with
   `easynet plugin install` or builtin reconciliation.
4. Keep script as developer helper only, or remove after plugin install covers
   the same behavior.

### 19.3 macOS lifecycle

Manifest:

```toml
schema_version = "1"
id = "easynet.desktop.menubar"
version = "0.1.0"
kind = "desktop_companion"
entrypoint = "dist/macos/EasyNetMenuBar.app"
abilities = []
permissions = ["clipboard_read", "clipboard_write", "global_hotkey"]
resources = ["desktop_session", "clipboard"]
platforms = ["macos"]

[limits]
max_sessions = 1
max_frame_queue = 1

[companion]
display_name = "EasyNet Menu Bar"
lifecycle = "user_session"
boot_policy = "ensure_running_after_daemon_ready"
stop_policy = "keep_running"
health = "status_file"
status_file = "state/easynet-menubar.status.json"

[companion.macos]
bundle_id = "tech.silan.easynet.menubar"
app_bundle = "dist/macos/EasyNetMenuBar.app"
supervisor = "launch_agent"
launch_agent_label = "tech.silan.easynet.menubar"
session = "aqua"
```

Expected behavior:

- `easynet plugin list` shows `easynet.desktop.menubar` as
  `desktop_companion`.
- `easynet runtime start` starts daemon; after Ready, it ensures the menu bar
  app is running if enabled.
- `easynet runtime stop` stops daemon but keeps the menu bar app alive.
- menu bar app switches to "daemon stopped".
- `easynet plugin disable easynet.desktop.menubar` stops the app and disables
  LaunchAgent.
- `easynet plugin enable easynet.desktop.menubar` enables LaunchAgent and
  starts app.
- `easynet self uninstall` removes the LaunchAgent and app bundle.

### 19.4 Windows lifecycle

Use the same cross-platform package id:

```text
easynet.desktop.menubar
```

The platform-specific section selects the Windows tray executable. Do not split
macOS and Windows into separate package ids unless their user-facing product
contracts diverge.

Expected behavior:

- install copies `EasyNetTray.exe` under `%USERPROFILE%\.easynet\apps`
- enable registers user startup
- start creates process if not running
- stop terminates known pid
- status uses status file, falling back to process name

## 20. Implementation Plan

### Phase 1: Model and parser

Files:

- `src/daemon/plugins/manifest.rs`
- `src/daemon/plugins/load_plan.rs`
- `src/daemon/plugins/surface.rs`
- `src/daemon/plugins/package.rs`
- `src/daemon/plugins/install.rs`

Tasks:

1. Add `PluginKind::DesktopCompanion`.
2. Add `PluginCompanionManifest` and platform specs.
3. Relax `ability_metadata` requirement for `desktop_companion`.
4. Validate `[companion]` is present for desktop companion.
5. Add companion load statuses.
6. Extend surface projection with `companion` record and `daemon = n/a`.
7. Extend installable package hashing to include declared companion artifacts.
8. Add package + supervisor transaction boundaries for companion install/update.

Tests:

- parse valid companion manifest with zero abilities
- reject companion without `[companion]`
- reject sidecar/declarative with `[companion]`
- platform mismatch works
- `plugin list --json` includes companion block
- package hash changes when `.app` or Windows executable artifact changes
- supervisor install failure rolls back package record

### Phase 2: Companion manager

New files:

```text
src/daemon/plugins/companion/mod.rs
src/daemon/plugins/companion/status.rs
src/daemon/plugins/companion/state_store.rs
src/daemon/plugins/companion/planner.rs
src/daemon/plugins/companion/macos.rs
src/daemon/plugins/companion/windows.rs
src/daemon/plugins/companion/linux.rs
```

Tasks:

1. Define supervisor trait.
2. Implement state store.
3. Implement current-platform supervisor factory.
4. Implement process/status-file observation.
5. Implement macOS LaunchAgent adapter.
6. Stub Windows and Linux adapters with correct status classification if full
   launch support is not ready.

Tests:

- pure state projection tests
- state store roundtrip
- fake supervisor transition tests
- macOS plist render test

### Phase 3: CLI integration

Files:

- `src/cli/commands/groups/plugin.rs`
- possibly `src/cli/commands/groups/companion.rs`
- `src/cli/commands/mod.rs`

Tasks:

1. Add `plugin enable/disable/status`.
2. Extend install/update/remove to call companion manager.
3. Render companion columns in `plugin list`.
4. Add JSON output stable shape.

Tests:

- install companion package invokes fake supervisor
- disable stops and disables fake supervisor
- remove stops, removes supervisor, then removes package

### Phase 4: Unified control-plane and SDK projection

Files:

- `src/protocol/companion_contract.rs`
- `sdk/schemas/desktop-companion-status.schema.json`
- `sdk/schemas/desktop-companion-action.schema.json`
- `src/ffi/daemon/mod.rs` or a dedicated FFI companion module
- SDK projection generators/tests for Python, Swift, Java, and Go
- `src/daemon/ability/builtins/integrations/plugins.rs`

Tasks:

1. Define `DesktopCompanionStatus` and `DesktopCompanionActionResult`.
2. Add JSON schema fixtures.
3. Add daemon-local `plugin.companion_status` and
   `plugin.companion_reconcile`.
4. Add FFI functions over the same DTOs.
5. Add language SDK wrappers or projections.
6. Ensure CLI online/offline paths return identical JSON.
7. Mark companion control abilities local-only and exclude them from realm
   publication.

Tests:

- protocol projection roundtrip for status/action DTOs
- CLI offline JSON equals daemon-local online JSON for the same fake status
- FFI companion status returns the same schema
- SDK generated DTOs include all required state fields
- remote invocation cannot reach companion control abilities

### Phase 5: Daemon lifecycle integration

Files:

- `src/daemon/ability/catalog/build.rs`
- `src/daemon/plugins/runtime_manager.rs`
- `src/daemon/boot/lifecycle/service.rs`
- `src/cli/commands/start.rs`
- `src/cli/commands/status.rs`
- `src/cli/commands/stop.rs`

Tasks:

1. Add post-Ready companion reconciliation hook.
2. Add runtime status companion projection.
3. Respect `stop_policy` in runtime stop.
4. Emit operator events for post-Ready companion failures.

Tests:

- daemon Ready path does not fail when companion start fails
- runtime stop keeps `keep_running` companion alive
- runtime stop stops `stop_on_runtime_stop` companion

### Phase 6: EasyNetMenuBar package

Files:

- `plugins/desktop-menubar/plugin.toml`
- `tools/scripts/build-macos-menubar.sh`
- release packaging scripts
- `platforms/macos/EasyNetMenuBar/Sources/EasyNetMenuBar/main.swift`

Tasks:

1. Add status-file heartbeat to Swift app.
2. Build and package `.app` into companion package dist.
3. Add builtin package registration.
4. Replace install script behavior with plugin install path.
5. Keep existing menu bar UI.

Tests:

- build script succeeds
- plugin manifest parses
- macOS LaunchAgent plist points to installed app
- status file heartbeat is accepted

## 21. Acceptance Criteria

For macOS:

1. Fresh install:
   - `easynet plugin list --json` includes `easynet.desktop.menubar`.
   - State is `not_installed` or `installed_disabled` before enable/install.
2. Enable:
   - `easynet plugin enable easynet.desktop.menubar` writes LaunchAgent.
   - The app appears in the menu bar.
3. Boot:
   - `easynet runtime start` reaches daemon Ready even if companion launch
     fails.
   - If enabled and Aqua session exists, companion is running after Ready.
4. Status:
   - `easynet runtime status --json` includes desktop companion state.
   - stale heartbeat becomes `stale`, not `running`.
5. Stop:
   - `easynet runtime stop` stops daemon.
   - menu bar remains running and shows daemon stopped.
6. Disable:
   - `easynet plugin disable easynet.desktop.menubar` stops app and disables
     LaunchAgent.
7. Uninstall:
   - self uninstall removes LaunchAgent and app artifact.

For Windows:

1. Equivalent status model exists.
2. Tray can be installed/enabled/started/stopped by companion supervisor.
3. Status uses status file or process fallback.

For Linux:

1. No false success in headless environments.
2. Unsupported platform/session is visible and non-fatal.

For unified control plane and SDK:

1. `easynet plugin status --json`, `easynet runtime status --json`,
   `plugin.companion_status`, FFI, and generated SDK DTOs expose the same
   `DesktopCompanionStatus` fields.
2. CLI online and offline paths produce the same JSON schema.
3. Axon SDKs do not expose companion lifecycle APIs.
4. EasyNet-Cli SDKs expose companion status and lifecycle through daemon control
   APIs or FFI, not through shell commands.
5. Companion control abilities are local-only and not remotely invokable.
6. Package hash changes when any declared companion artifact changes.
7. A failed supervisor install cannot leave an active plugin lock pointing at a
   partially installed companion package.

## 22. Review Checklist

Before merging implementation:

1. Does `desktop_companion` avoid fake ability descriptors?
2. Does daemon boot remain independent from GUI session availability?
3. Is platform support split into compile target and runtime session checks?
4. Does `runtime stop` honor `stop_policy`?
5. Does `plugin disable` stop and disable the companion?
6. Does status distinguish desired, supervisor, and observed state?
7. Are macOS/Windows launchers user-level only?
8. Does the companion status file include package id, version, pid, and
   heartbeat time?
9. Does `plugin list` render `daemon = n/a` for companion-only packages?
10. Are all post-Ready companion failures non-fatal to daemon lifecycle?
11. Do CLI, daemon-local control abilities, FFI, and SDKs share one DTO/schema?
12. Are companion control abilities excluded from remote advertisement and
    invocation?
13. Does package hashing cover declared companion artifacts?
14. Can package/supervisor install rollback leave the active plugin lock
    consistent?

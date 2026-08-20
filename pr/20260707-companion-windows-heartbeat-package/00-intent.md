# Companion Windows Heartbeat And Package Dist

## Goal

Complete the Windows Phase 6 companion process contract by writing the shared status-file heartbeat from `EasyNetTray` and making the release script publish the tray artifact into the plugin package `dist` path declared by `plugins/desktop-menubar/plugin.toml`.

## Boundary

- The Windows tray app owns only local UI and heartbeat emission.
- The daemon companion manager owns heartbeat classification and lifecycle DTO projection.
- Release packaging owns plugin package artifact layout; SDKs and CLI do not build or inspect Windows tray internals.

## Invariants

- The heartbeat path is `%USERPROFILE%\.easynet\companions\easynet.desktop.menubar\status.json`.
- Heartbeat writes include package id, version, pid, started time, last-seen time, and daemon status fields.
- Heartbeat writes are atomic through a temporary file and replace.
- The release script writes `EasyNetTray.exe` under `plugins\desktop-menubar\dist\windows\EasyNetTray`.

## Verification

- C# build or publish check should run when the local .NET Windows-targeting toolchain permits it.
- Static file audit must confirm the heartbeat fields and dist path.
- Touched-file terminology audit must pass.

## Results

- Static audit confirmed heartbeat fields: `package_id`, `package_version`, `pid`, `started_at_unix_ms`, `last_seen_unix_ms`, daemon status, and `status.json`.
- Static audit confirmed Windows release script publishes to `plugins\desktop-menubar\dist\windows\EasyNetTray` and verifies `EasyNetTray.exe`.
- `cargo test -q daemon::plugins::manifest` passed.
- `cargo test -q daemon::plugins::package` passed.
- `cargo test -q plugin_host_install` passed.
- `git diff --check` passed.
- Touched-file terminology audit passed.
- `dotnet --info` failed because `dotnet` is not installed in this environment.
- `pwsh` is not installed in this environment, so `packaging/release/build-windows-tray.ps1` could not be executed here.

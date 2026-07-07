# Companion Daemon Status Source

## Goal

Remove the transitional process-name daemon probe from the macOS and Windows desktop companions. The companion heartbeat should report daemon facts from the local daemon lifecycle discovery projection instead of scanning for a process named `easynet-daemon`.

## Boundary

- The companion app may read local daemon lifecycle discovery facts from the user's EasyNet state directory.
- The companion app does not own daemon lifecycle, plugin lifecycle, endpoint probing policy, or status DTO projection.
- The daemon remains the authority for plugin lifecycle and companion status classification.

## Invariants

- Missing or malformed daemon discovery reads as stopped.
- A discovery file only counts as running when its advertised PID is alive.
- Control and invocation heartbeat facts are derived from the advertised local endpoints, never from a process-name scan.
- Heartbeat payload shape remains the SPEC-defined companion process contract.

## Verification

- Static audit must show no `pgrep` or process-name daemon scan remains in companion apps.
- Static audit must show both apps read `control.json`.
- Touched-file terminology audit must pass.
- Native Swift/C# build checks should run when the local platform toolchains permit them.

## Results

- Static audit confirmed no `pgrep`, `GetProcessesByName`, or `easynet-daemon` process-name scan remains in the macOS and Windows companion apps.
- Static audit confirmed both companion apps read `control.json`.
- `swiftc -parse platforms/macos/EasyNetMenuBar/Sources/EasyNetMenuBar/main.swift` passed.
- `cargo test -q daemon::plugins::companion` passed.
- `git diff --check` passed.
- Touched-file terminology audit passed.
- `dotnet --info` failed because `dotnet` is not installed in this environment.

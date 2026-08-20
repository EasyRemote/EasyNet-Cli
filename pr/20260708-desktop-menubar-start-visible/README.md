# Desktop Menubar Start Visibility

## Goal

Make the builtin `easynet.desktop.menubar` companion produce a visible macOS menu
bar app after ordinary local builds and `easynet start` reconciliation.

## Boundary Proof

- The desktop companion app is owned by the EasyNet-Cli plugin runtime, not by
  Axon invocation or SDK protocol logic.
- The provider owns its installable package surface. The daemon package index
  must consume that surface instead of stitching product-specific paths into
  supervisor code.
- The supervisor owns only OS session installation and launch behavior. It must
  not build or discover package artifacts.

## Invariants

- Builtin package hash covers the same root that install/start consumes.
- macOS startup opens the installed `.app` bundle through LaunchServices.
- `easynet start` remains non-fatal when companion launch fails.
- Desired state remains authoritative: Ready reconciliation starts enabled
  companions and does not silently auto-enable fresh installs.

## Verification Plan

- Focused Rust tests for provider materialized package root and macOS plist.
- Build script smoke check for the macOS app artifact.
- `cargo build` must materialize the companion package root on macOS.
- Runtime check should prove the installed LaunchAgent points at the installed
  `.app` bundle and the status file heartbeat can be observed.

## Decisions

- Builtin desktop companion packages use a provider-owned materialized package
  root under Cargo `OUT_DIR`; runtime install/start never falls back to
  developer-only `target/macos` artifacts.
- macOS `.app` bundles are ad-hoc signed after `Info.plist` and resources are
  copied, so LaunchServices sees the bundle identifier and sealed resources.
- The menu bar companion uses the packaged EasyNet PNG as a narrow fixed-width
  status item. The heartbeat records status-item window geometry and notch
  obstruction so crowded/notched menu bars become diagnosable.
- The macOS supervisor treats LaunchAgent state as version-specific: stale
  plists or old script-installed apps do not count as current package enablement.

## Verified

- `cargo build -q`
- `cargo fmt --check`
- `cargo test -q launch_agent`
- `cargo test -q plugin_host_builtin_package_uses_provider_installable_root`
- `git diff --check`
- URA terminology scan over touched runtime/plugin files
- Runtime: after killing `EasyNetMenuBar`, `target/debug/easynet start`
  relaunched the installed app from
  `~/.easynet/apps/easynet.desktop.menubar/0.1.0/EasyNetMenuBar.app`.
- Runtime: heartbeat reported `window_visible=true`,
  `obscured_by_notch=false`, and a status-item frame in the macOS right
  auxiliary top area.

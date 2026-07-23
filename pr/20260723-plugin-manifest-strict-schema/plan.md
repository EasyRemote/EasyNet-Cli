# Plugin manifest strict schema convergence

## Goal

Remove the plugin package manifest compatibility behavior where `plugin.toml`
could carry unknown fields or retired plugin kind aliases that the daemon would
silently ignore or normalize.

## Root abstraction problem

`plugin.toml` is the package contract for plugin load, route publication,
permissions, realtime activation, companion lifecycle, and sidecar/declarative
execution. Permissive parsing creates a split-brain product story: plugin
authors can believe an undeclared field affects runtime behavior while the
daemon drops it. Retired kind aliases also keep a second taxonomy alive at the
plugin boundary.

## Invariants

1. Package manifests reject unknown top-level fields.
2. Ability metadata rejects unknown fields.
3. Realtime capability declarations reject unknown fields.
4. Companion lifecycle/platform sections reject unknown fields.
5. Declarative binding and runtime limits reject unknown fields.
6. `PluginKind` accepts only canonical kind values.
7. Valid existing package manifests continue to parse unchanged.

## Verification plan

- Focused plugin manifest tests for unknown top-level, nested metadata,
  declarative binding, companion platform, realtime capability, runtime limit,
  and retired kind alias rejection.
- Existing plugin manifest/module tests.
- Parse real `plugins/remote-desktop/plugin.toml` and
  `plugins/desktop-menubar/plugin.toml` through existing package tests.
- `cargo fmt --check`.
- SPEC v2 convergence gate.
- Architecture convergence gate.
- codegraph sync/status.


# Plugin Independent Project SPEC

**Status:** proposed.
**Date:** 2026-07-08.
**Target cases:** migrate `easynet.remote_desktop` and desktop companion
packages away from daemon-owned business implementation while allowing the
top-level build to compile all shipped packages together.

## 1. Problem

EasyNet-Cli has a plugin package model, but the shipped preset plugins are not
cleanly separated from daemon implementation ownership.

Current examples:

- `plugins/remote-desktop/plugin.toml` and ability descriptors live in a
  plugin package folder, but the executable implementation lives under
  `src/daemon/resources/remote_desktop`.
- `plugins/desktop-menubar/plugin.toml` describes the companion package, while
  the companion lifecycle framework lives under `src/daemon/plugins/companion`
  and the UI apps live under `platforms/macos/EasyNetMenuBar` and
  `platforms/windows/EasyNetTray`.
- `src/daemon/plugins/mod.rs::builtin_bindings()` names preset bindings in
  daemon code.

This creates four architecture defects:

1. A package can look like a plugin while its implementation is still owned by
   daemon resource modules.
2. Adding a shipped plugin requires editing daemon runtime code instead of
   adding a package plus provider implementation.
3. Ability descriptor ownership and ability handler ownership diverge.
4. The boundary between generic plugin runtime and product-specific plugin
   behavior is not mechanically checkable.

The objective is not to ban monorepo builds. A shipped plugin may still compile
   with the daemon binary. The objective is to make the plugin an independent
   project with explicit provider boundaries.

## 2. Decision

Move shipped preset plugin implementations into independent plugin projects.

The daemon plugin host remains the generic runtime owner:

```text
PluginPackage
  -> PluginManifest
  -> PluginLoadPlan
  -> PluginProviderRegistry
  -> PluginProvider
  -> AbilityImpl bindings
  -> AxonAbilityCatalog
```

Each shipped plugin owns its own package directory and implementation project:

```text
plugins/remote-desktop/
  plugin.toml
  abilities/*.ability.toml
  Cargo.toml
  src/lib.rs
  src/handlers/*
  src/runtime/*
```

The top-level build may compile `plugins/remote-desktop` together with
`easynet-daemon`. That is a build decision, not an ownership decision.

The daemon must not contain remote-desktop business handlers after migration.
It may contain only generic plugin host abstractions and a small provider
registry adapter.

## 3. Boundary Rules

### 3.1 Daemon-owned code

EasyNet-Cli daemon owns:

- package discovery and install state
- manifest parsing and validation
- package hash and artifact integrity
- plugin load planning
- generic provider registry
- generic sidecar/declarative/native provider adapters
- ability registration into the daemon catalog
- local daemon policy, admission boundary, and local-only restrictions
- runtime status projection

Daemon code must not own:

- remote desktop session business state
- remote desktop WebRTC signaling semantics
- remote desktop permission request workflow
- desktop UI app behavior
- product-specific package layout beyond generic package rules
- plugin-specific ability handler tables outside provider-owned code

### 3.2 Plugin-project-owned code

Each independent plugin project owns:

- `plugin.toml`
- ability descriptor files
- implementation source
- plugin-specific runtime state machines
- plugin-specific tests
- plugin-specific packaging scripts
- provider export function or sidecar executable

The plugin project may depend on daemon plugin SDK crates or shared protocol
types. It must not depend on private daemon modules.

### 3.3 Axon boundary

This migration does not add Axon concepts. Plugins remain EasyNet-Cli daemon
product/runtime extensions. Axon remains owner of Invocation, receipt,
admission, stream/bidi protocol, signing, and URA semantics.

No plugin project may redefine Invocation tuple construction or receipt
semantics. If a plugin exposes an ability, the ability enters through the
normal daemon `AbilityImpl` path.

## 4. Goals

1. Make preset plugin implementations independently owned projects.
2. Keep the top-level build able to compile shipped plugins with the daemon.
3. Remove plugin-specific business implementation from `src/daemon/resources`.
4. Replace daemon hardcoded binding lists with a typed provider registry.
5. Preserve public plugin behavior and ability names.
6. Preserve existing install/index/status behavior.
7. Keep external sidecar/declarative plugin support compatible.
8. Make plugin ownership mechanically testable.

## 5. Non-Goals

1. Do not require dynamic library loading in the first phase.
2. Do not move plugin lifecycle into Axon.
3. Do not rename public ability names.
4. Do not change the Invocation public contract.
5. Do not keep compatibility wrappers whose only purpose is old module layout.
6. Do not leave duplicate handler tables in daemon and plugin project.
7. Do not introduce product-specific SDK abstractions.

## 6. Target Project Layout

### 6.1 Repository layout

```text
plugins/
  remote-desktop/
    Cargo.toml
    plugin.toml
    abilities/
      remote_desktop.create_session.ability.toml
      ...
    src/
      lib.rs
      registration.rs
      handlers/
      runtime/
      transport/
      media/
      permissions.rs
      schema.rs

  desktop-menubar/
    plugin.toml
    companion/
      macos/
        EasyNetMenuBar/
      windows/
        EasyNetTray/
    scripts/
      build-macos.sh
      build-windows.ps1
    dist/
```

### 6.2 Daemon layout after migration

```text
src/daemon/plugins/
  manifest.rs
  package.rs
  index.rs
  load_plan.rs
  provider.rs
  provider_registry.rs
  runtime_manager.rs
  host_api.rs
  sidecar/
  companion/
```

Allowed daemon resources after migration:

```text
src/daemon/resources/
  files/
  media/
  pages/
  skills/
```

`src/daemon/resources/remote_desktop` must be removed after the provider cutover.

## 7. Provider Model

### 7.1 Provider identity

A native shipped plugin provider is identified by package id and provider kind:

```rust
pub struct PluginProviderId {
    pub package_id: &'static str,
    pub provider_kind: PluginProviderKind,
}

pub enum PluginProviderKind {
    NativeStatic,
    Sidecar,
    Declarative,
    DesktopCompanion,
}
```

`NativeStatic` means the provider is linked into the daemon binary. It does not
mean daemon owns plugin business logic.

### 7.2 Provider trait

```rust
pub trait PluginProvider: Send + Sync {
    fn package_id(&self) -> &'static str;
    fn provider_kind(&self) -> PluginProviderKind;
    fn manifest_body(&self) -> &'static str;
    fn manifest_path(&self) -> &'static str;
    fn ability_specs(&self) -> Vec<BuiltinPluginAbilitySpec>;
    fn contribute(
        &self,
        builder: &mut PluginContributionBuilder,
        limits: PluginRuntimeLimits,
    ) -> Result<()>;
}
```

The daemon calls this interface. It does not call plugin-specific handlers
directly.

### 7.3 Provider registry

```rust
pub struct PluginProviderRegistry {
    providers: BTreeMap<&'static str, Arc<dyn PluginProvider>>,
}
```

The registry owns:

- uniqueness by package id
- provider lookup for builtin/native-static packages
- conversion from provider to `BuiltinPluginBinding`
- validation that provider manifest package id matches provider id

The registry must be the only location where shipped native-static providers
are listed.

## 8. Build Model

### 8.1 Workspace build

First release target:

```toml
[workspace]
members = [
  ".",
  "plugins/remote-desktop",
]
```

The daemon depends on shipped native-static plugin crates behind features:

```toml
[features]
remote-desktop = ["easynet-plugin-remote-desktop"]

[dependencies]
easynet-plugin-remote-desktop = {
  path = "plugins/remote-desktop",
  optional = true
}
```

This keeps one build command while preserving project ownership.

### 8.2 Provider registration adapter

Daemon code may contain:

```rust
#[cfg(feature = "remote-desktop")]
registry.register(easynet_plugin_remote_desktop::provider());
```

Daemon code must not contain:

```rust
crate::daemon::resources::remote_desktop::contribute(...)
```

The adapter is compile wiring only. The plugin crate owns all business
implementation.

### 8.3 External package build

External packages keep using existing sidecar/declarative packaging:

```text
my-plugin/
  plugin.toml
  abilities/*.ability.toml
  bin/my-plugin
```

External native dynamic providers are reserved for a later SPEC.

## 9. Manifest Model

No product-specific manifest fields are added for this migration.

The existing package fields remain:

```toml
schema_version = "1"
id = "easynet.remote_desktop"
version = "0.1.0"
kind = "builtin"
entrypoint = "easynet_plugin_remote_desktop::provider"
abilities = ["abilities/*.ability.toml"]
```

For native-static packages, `entrypoint` names a provider export symbol, not a
daemon module path.

Validation rules:

1. The package id in `plugin.toml` must equal `PluginProvider::package_id()`.
2. The `entrypoint` must match the provider export expected by the registry.
3. Every ability in `ability_metadata` must have a descriptor file.
4. Every provider ability spec must match `ability_metadata`.
5. A provider may not contribute abilities absent from the manifest.
6. A manifest may not point at `src/daemon/resources/*`.

## 10. Remote Desktop Migration

### 10.1 Current state

Current implementation:

```text
plugins/remote-desktop/
  plugin.toml
  abilities/*.ability.toml

src/daemon/resources/remote_desktop/
  handlers/*
  runtime/*
  transport/*
  registration.rs
```

### 10.2 Target state

Target implementation:

```text
plugins/remote-desktop/
  Cargo.toml
  plugin.toml
  abilities/*.ability.toml
  src/
    lib.rs
    registration.rs
    handlers/*
    runtime/*
    transport/*
    media/*
```

### 10.3 Provider export

```rust
pub fn provider() -> Arc<dyn PluginProvider> {
    Arc::new(RemoteDesktopProvider::new())
}
```

### 10.4 Public behavior preservation

The following ability names must not change:

- `remote_desktop.create_session`
- `remote_desktop.show_session`
- `remote_desktop.set_description`
- `remote_desktop.add_ice_candidate`
- `remote_desktop.watch_events`
- `remote_desktop.refresh_lease`
- `remote_desktop.end_session`
- `remote_desktop.attach`
- `remote_desktop.permission_status`
- `remote_desktop.request_permission`

All existing session lifecycle, permission, transport, and event behavior must
remain byte-compatible at public API boundaries.

### 10.5 Removal rule

After cutover, `src/daemon/resources/remote_desktop` must be deleted. A module
that re-exports the new plugin crate from the old path is not allowed unless a
separate public API compatibility SPEC explicitly requires it.

## 11. Desktop Companion Migration

The companion lifecycle framework remains daemon-owned because it is generic
plugin runtime infrastructure:

```text
src/daemon/plugins/companion/*
```

The UI apps should move under the package project:

```text
plugins/desktop-menubar/companion/macos/EasyNetMenuBar
plugins/desktop-menubar/companion/windows/EasyNetTray
```

The package owns app code and build scripts. The daemon owns install,
supervision, desired state, observed state, and status projection.

The old platform paths may be removed after build scripts and docs point at the
package-owned paths.

## 12. Install and Index Behavior

Installed packages continue to use:

```text
~/.easynet/plugins/installed/<id>/<version>
~/.easynet/plugins/state/plugins.toml
~/.easynet/plugins/state/plugin-lock.toml
```

The index continues to load:

```text
builtin/native-static providers
  + installed package lock entries
```

Installed sidecar/declarative packages remain package-first and do not require
daemon rebuilds.

Native-static provider packages are compiled into the daemon but still indexed
from their package manifests.

## 13. Runtime Manager Behavior

`PluginRuntimeManager` must depend on generic provider interfaces only.

Forbidden:

```rust
if package.id() == "easynet.remote_desktop" {
    remote_desktop::register(...)
}
```

Required:

```rust
let provider = provider_registry.provider_for(package.id())?;
provider.contribute(builder, package.manifest().limits())?;
```

Sidecar, declarative, desktop companion, and native-static packages must remain
separate runtime paths with shared package discovery and status projection.

## 14. State Machines

### 14.1 Provider load state

```text
Unseen
  -> ManifestIndexed
  -> ProviderResolved
  -> ContributionBuilt
  -> Registered
  -> Failed
```

Terminal for one load attempt:

- `Registered`
- `Failed`

Failures must include:

- `manifest_invalid`
- `provider_missing`
- `provider_id_mismatch`
- `ability_spec_mismatch`
- `contribution_failed`
- `catalog_registration_failed`

### 14.2 Package origin state

```text
NativeStatic
InstalledSidecar
InstalledDeclarative
DesktopCompanion
Unsupported
```

Origin is not a behavior shortcut. It only selects the provider adapter. Ability
semantics remain descriptor and implementation binding semantics.

## 15. Error Semantics

Use typed error classes:

```text
provider_missing
provider_id_mismatch
provider_manifest_mismatch
provider_ability_mismatch
provider_contribution_failed
provider_registry_duplicate
plugin_project_boundary_violation
```

Rules:

- Missing native-static provider for a builtin package is a load error, not a
  daemon boot panic.
- Installed package corruption is projected as package index error and must not
  poison builtin package loading.
- Provider contribution failure must not partially register abilities.
- Duplicate ability ownership remains fatal for the conflicting package plan.

## 16. Tests

### 16.1 Boundary tests

- No `src/daemon/resources/remote_desktop` module after cutover.
- No daemon code imports plugin-specific handler modules.
- Provider registry is the only native-static provider list.
- Every native-static provider manifest package id matches provider id.
- Every native-static provider ability spec matches manifest metadata.

### 16.2 Behavior tests

- `easynet plugin list --json` shows `easynet.remote_desktop` unchanged.
- `meta.list_abilities` shows unchanged remote desktop ability names.
- Remote desktop create/show/update/end session tests pass unchanged.
- Realtime activation plans remain unchanged.
- Companion package list/status remains unchanged.

### 16.3 Build tests

- Default workspace build succeeds.
- Build without `remote-desktop` feature succeeds and reports the package as
  unavailable or absent according to release profile.
- External sidecar plugin install still succeeds.
- Installed package lock hash validation still rejects tampered package files.

### 16.4 SDK tests

- Go and Python capability matrices remain aligned.
- No SDK adds product-specific remote desktop lifecycle abstractions outside
  generic runtime/profile surfaces.
- Public ability names and DTOs stay stable.

## 17. Migration Plan

### Phase 1: Provider abstraction

Files:

- `src/daemon/plugins/provider.rs`
- `src/daemon/plugins/provider_registry.rs`
- `src/daemon/plugins/package.rs`
- `src/daemon/plugins/runtime_manager.rs`

Tasks:

1. Introduce `PluginProvider` and `PluginProviderRegistry`.
2. Convert existing `BuiltinPluginBinding` construction to registry-backed
   binding projection.
3. Keep public plugin behavior unchanged.
4. Add provider identity and manifest matching tests.

### Phase 2: Remote desktop crate extraction

Files:

- `plugins/remote-desktop/Cargo.toml`
- `plugins/remote-desktop/src/*`
- `src/daemon/resources/remote_desktop/*`

Tasks:

1. Move remote desktop code into `plugins/remote-desktop/src`.
2. Replace private daemon imports with plugin SDK/shared crate imports.
3. Export `provider()`.
4. Register provider through the registry.
5. Delete old daemon remote desktop module.

### Phase 3: Desktop app project relocation

Files:

- `platforms/macos/EasyNetMenuBar`
- `platforms/windows/EasyNetTray`
- `plugins/desktop-menubar/companion/*`
- packaging scripts

Tasks:

1. Move app source under `plugins/desktop-menubar/companion`.
2. Update build scripts to write `plugins/desktop-menubar/dist`.
3. Keep daemon companion lifecycle unchanged.
4. Delete old platform app paths after scripts and docs are migrated.

### Phase 4: Registry hardening

Tasks:

1. Add tests rejecting daemon imports of plugin-specific handler modules.
2. Add package ownership report in plugin status diagnostics.
3. Add CI check for plugin project boundaries.
4. Document native-static versus sidecar packaging.

## 18. Acceptance Criteria

1. `easynet.remote_desktop` implementation lives under
   `plugins/remote-desktop/src`.
2. `src/daemon/resources/remote_desktop` is removed.
3. `builtin_bindings()` no longer hardcodes plugin-specific business bindings.
4. Native-static plugin provider registration goes through
   `PluginProviderRegistry`.
5. Public ability names, descriptor paths, and runtime behavior are preserved.
6. External sidecar/declarative plugin install remains compatible.
7. Desktop companion package lifecycle remains generic daemon plugin
   infrastructure.
8. Desktop UI app source is package-owned or has a tracked migration exception.
9. Boundary tests prevent plugin business code from re-entering daemon modules.
10. No Axon SDK exposes plugin product lifecycle APIs.

## 19. Review Checklist

Before merge:

1. Does daemon code own only generic plugin runtime behavior?
2. Does each shipped plugin own its manifest, descriptors, source, tests, and
   packaging?
3. Is native-static linking treated as build wiring only?
4. Is provider registration centralized and typed?
5. Are ability names and public DTOs unchanged?
6. Are old daemon plugin business modules deleted?
7. Are there no compatibility re-export layers from old module paths?
8. Are sidecar/declarative packages still installable?
9. Are package hash and lock semantics unchanged?
10. Does the SDK capability matrix remain language-aligned?


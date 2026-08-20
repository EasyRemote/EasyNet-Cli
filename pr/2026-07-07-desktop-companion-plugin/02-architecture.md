# Architecture

## Boundary

Desktop companion packages extend the EasyNet-Cli plugin model. They share package discovery, platform filtering, install transactions, CLI visibility, and status projection with ability plugins. They do not share ability runtime semantics.

## Ownership

- `manifest.rs`: typed package schema and companion metadata validation.
- `load_plan.rs`: platform/session load planning, with companion plans separate from runtime ability loading.
- `companion/*`: desired-state store, state projection, platform supervisors, heartbeat/process observation.
- `install.rs`: transactional package plus supervisor commit/rollback.
- `surface.rs`: plugin list projection, including `daemon = n/a` for companion-only packages.
- `protocol/companion_contract.rs`: single DTO projection for CLI, daemon-local controls, FFI, and SDKs.
- SDK language folders: product SDK wrappers over daemon control/FFI transport, not shell commands.

## State Model

`DesktopCompanionManager` composes:

- desired state from `~/.easynet/companions/state.toml`
- supervisor state from the platform adapter
- observed state from status file or process fallback

The manager is the only component that projects those facts into the operator-facing companion state.

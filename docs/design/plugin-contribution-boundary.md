# Plugin Contribution Boundary

**Status:** implemented in `runtime/plugin_host/contribution.rs`,
`runtime/plugin_host/broker.rs`, and `runtime/plugin_host/realtime.rs`.
**Date:** 2026-07-02.

## Clean Definition

A plugin is an installable `AbilityImpl` provider package. It is not an
authority root, not an invocation protocol, and not a caller/callee identity.

The minimum plugin shape is:

```text
my-plugin/
  plugin.toml
  abilities/
    some.ability.toml
  bin/
    my-plugin
```

`plugin.toml` declares package identity, load model, requirements, and ability
metadata. `abilities/*.ability.toml` declares descriptor-facing schema and
description. The executable or builtin Rust binding supplies handlers.

`easynet plugin init <path>` creates the default Python developer version of
this shape: a declarative exec Hello World package with one governed echo
ability. The generated Python package is intentionally installable as-is so a
contributor can run the full product loop before adding product-specific logic:

```bash
easynet plugin init hello-plugin
cd hello-plugin
easynet plugin install .
```

`easynet plugin init --language go <path>` creates a compiled Go source
project. It must be built before install:

```bash
easynet plugin init --language go hello-go-plugin
cd hello-go-plugin
make build
easynet plugin install .
```

The plugin runtime is not Python-only. Declarative exec plugins are executable
sidecar processes; Python, Go, Rust, Node, Java, C++, or another runtime can be
used if the package supplies the executable declared by `[declarative].argv`.

Generated executables do not hand-write sidecar JSON frames. Python templates
import `easynet_sdk.providers.runtime.plugin_exec`; Go templates import
`easynet.run/cli/sdk/go/provider/runtime/pluginexec`. Each template implements
only a `SidecarInvocation -> result` handler. The daemon/provider frame grammar
remains owned by CLI SDK provider helpers; plugin code should not construct
`call_id`, `result`, `error`, stream, or bidi protocol frames directly.

The scaffold separates the two load-bearing versions:

- `plugin.toml` `version` is the package lifecycle version used by
  install/update/remove.
- `abilities/*.ability.toml` `descriptor_version` is the governed interface
  version that enters descriptor refs, authority bindings, implementation
  bindings, and receipts.

## Runtime Boundary

The runtime path is:

```text
PluginPackage
  -> PluginPackageContribution
  -> PluginResourceBroker / PluginPolicyBroker / RealtimeTransportAdapter
  -> DaemonPluginBinder
  -> AxonAbilityCatalog / LocalRuntime
```

The contribution contains:

- package id/version/kind
- permission and resource requirements
- realtime capability metadata
- ability registry manifests
- implementation source/runtime environment/handler

The contribution deliberately does not contain:

- `OwnerKind`
- `AuthorityBinding`
- admission policy
- caller/callee identity
- invocation tuple construction
- receipt semantics

Those remain daemon/Axon responsibilities.

## Collision Rule

Plugin templates reduce accidental naming collisions, but they are not the
authority. Collision prevention is enforced in the daemon path:

1. Installer/index rejects duplicate package identity: `package_id@version`.
2. Active package state keeps one active version per package id for update
   semantics.
3. Runtime bind rejects duplicate ability ownership under the same daemon owner.
4. Descriptor binding rejects conflicting descriptor facts for the same
   ability/version/call-mode identity.
5. Resource and permission brokers report blocked/partial readiness instead of
   granting authority or silently publishing an unsafe ability.

A plugin package never becomes an authority root. The daemon binds
`PluginAbilityImpl` through the device-sponsored `plugin-management`
SystemAgent; callers only see the resulting governed `AbilityDescriptor`.
The Device remains the plugin runtime execution host and custody substrate, not
the public descriptor owner/callee.

## Authority Rule

The daemon binder applies `OwnerKind::plugin_management_system()` when it binds
a plugin contribution. This keeps plugin packages from becoming authority roots
by accident while also preventing plugin abilities from falling back to direct
Device ownership.

If a future plugin needs a distinct authority projection, that must be added as
a daemon policy decision in the binder or policy broker, not as arbitrary
metadata inside `plugin.toml`.

## Broker Rule

Resource and policy readiness are daemon-owned read models:

- `PluginResourceBroker` checks a plugin's declared resource kinds against the
  local `resources.json` table.
- `PluginPolicyBroker` checks whether the daemon currently exposes permission
  status/request actions for the declared permissions.
- `PluginActivationBroker` composes resource, policy, transport, and publish
  readiness into the `plugin.activate_realtime` outcome.
- Neither broker grants authority, constructs invocations, or rewrites Axon
  admission. They only explain why activation is ready, partial, blocked, or
  unknown.

This keeps plugin manifests declarative while preventing lifecycle APIs from
owning resource or permission policy.

## Builtin Plugins

Builtin plugins follow the same boundary as sidecar and declarative plugins.
Their compiled tables now write into `PluginContributionBuilder`; they do not
write directly to `AxonAbilityCatalog`.

That means builtin implementations enter the control plane as
`AbilityImplSource::BuiltinPlugin`, while sidecar/declarative implementations
enter as their own implementation sources.

## Realtime Rule

Realtime is not a plugin lifecycle API. It is represented as ability call mode
and transport metadata:

```text
mode = rpc | stream | bidi
transport = invoke_stream | invoke_bidi | webrtc
```

Package-level realtime capabilities are activation and UI metadata only. The
callable surface remains the declared `AbilityDescriptor` plus its bound
`AbilityImpl`.

The generic transport adapter registry maps declared transports onto daemon
ability readiness:

- `invoke_stream`
- `invoke_bidi`
- `webrtc`

For WebRTC, the adapter checks both the declared activation abilities and the
required signaling roles:

- `session_create`
- `description_exchange`
- `ice_trickle`
- `session_end`

The plugin implementation still owns SDP, ICE, endpoint handles, media capture,
and data channels. `easynet.remote_desktop` is the current production WebRTC
plugin and declares `webrtc` with `invoke_bidi` fallback.

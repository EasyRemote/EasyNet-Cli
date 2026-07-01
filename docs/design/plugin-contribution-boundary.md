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

## Authority Rule

The daemon binder applies the current owner policy (`OwnerKind::Device` today)
when it binds a plugin contribution. This keeps plugin packages from becoming
authority roots by accident.

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

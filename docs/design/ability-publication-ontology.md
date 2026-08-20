# Ability Publication Ontology

Status: architecture note.
Date: 2026-08-17.
Scope: EasyNet/Axon runtime ontology for principals, agents, services, devices, abilities, dynamic publication, routing, and executable bindings.

This document pins the semantic model used by daemon catalog assembly,
owner projection, route resolution, frontend/backend invocation, and plugin
ability publication. It is intentionally not a static/dynamic ability taxonomy:
`static` and `dynamic` describe implementation sources, not the core protocol
semantics.

## One-line model

An `Ability` is a governed, invokable behavior contract advertised by a
callable owner. An `Invocation` calls an `AbilityDescriptor`. An `AbilityImpl`
is only the executable binding that satisfies one descriptor version. A
`Device` may host execution, but it is not the semantic owner of ordinary
abilities.

The catalog should therefore model ability exposure as:

```text
AbilityPublication =
    DescriptorContract
  + OwnerBinding
  + AuthorityBinding
  + ImplementationBinding
  + RouteBinding
```

This is the root abstraction. It replaces flat reasoning such as "published
system ability name equals descriptor path equals local handler key".

## Core ontology

| Term | Semantic responsibility |
|---|---|
| `Principal` | Accountability root. Current concrete kinds are `User` and `Service`. A Principal is not automatically an Agent. |
| `User` | Human/account principal. It authenticates, owns, delegates, and is accountable. It is not a routable invocation callee. |
| `Service` | Non-human principal or service identity, usually under a user or realm, such as `service/<user>.pages`. It may expose a callable service surface. |
| `Agent` | Routable logical actor that can advertise ability descriptors and appear as an invocation `callee`. |
| `ServiceSurface` | Callable surface for a Service. It satisfies the callee role without pretending the user account itself is an Agent. |
| `SystemAgent` | Restricted Agent sponsored by a Device for device-native abilities. The Device remains sponsor/host/custodian, not public owner. |
| `HostedAgent` | User/service-owned dynamic Agent currently hosted by some Device/daemon lifecycle. |
| `Device` | Execution substrate: host, local resource boundary, key custody, transport reachability, and possible receipt attestor. |
| `Daemon` | EasyNet product runtime that owns projection, dispatch, local policy, plugin lifecycle, local resources, routing, and receipt emission. |
| `AbilityDescriptor` | Versioned governed interface: public name, ability URA, schema, call mode, subject contract, admission action, receipt semantics, version, and hashes. |
| `AuthorityBinding` | Governance predicate over both advertisement and invocation. It answers who may publish and who may invoke for a given subject and causal context. |
| `AbilityImpl` | Versioned executable binding: implementation kind, entrypoint, implementation hash, runtime environment, and supported call mode. |
| `AbilityPublication` | Complete catalog fact binding descriptor, owner, authority, implementation, and route into one publishable/invokable surface. |
| `RouteBinding` | Deterministic mapping from callable owner and descriptor to execution host, descriptor ref, transport, and local dispatch key. |
| `Invocation` | Signed causal call with `caller`, `callee`, `ability`, `subject`, `nonce`, `causal_context`, and `args`. |
| `Receipt` | Auditable execution fact binding invocation, descriptor version, schema hash, implementation hash, runtime environment, authority proof, inputs, outputs, parents, terminal state, and signature. |
| `Resource` | Object being read, written, streamed, or observed. It is a noun, not an Ability. |
| `StateObject` | Resource with canonical state-transition semantics and receipt requirements. |
| `Session` | Stateful interaction channel with explicit lifecycle and terminal closure. |
| `Plugin` | Implementation packaging/lifecycle unit. A plugin is not itself an Ability; it may provide `AbilityImpl` bindings. |

## Non-negotiable identity rules

1. A `User` account is a `Principal`, not an `Agent`.
2. A `Service` is a `Principal`; if it exposes callable behavior, that behavior is reached through a `ServiceSurface` / callable owner URA.
3. An `Agent` is a routable callable actor, not an account, host process, socket, or plugin instance.
4. A `Device` is an execution substrate. It may sponsor `SystemAgent`s and host implementations; it must not be treated as the public callee for ordinary device-native abilities.
5. A `Daemon` is runtime infrastructure. It dispatches and projects abilities, but it is not the business principal that owns them.
6. `AbilityDescriptor` and `AbilityImpl` are separate. Discovery of a descriptor does not imply executable binding.
7. `AuthorityBinding` governs both advertise and invoke. Publication-only proof is incomplete.
8. `RouteBinding` must not collapse `callee_ura`, `execution_host_ura`, `descriptor_ref`, and local `dispatch_key` into one string.

## AbilityPublication

`AbilityPublication` is the canonical catalog unit.

```text
AbilityPublication {
  descriptor_contract: DescriptorContract
  owner_binding: OwnerBinding
  authority_binding: AuthorityBinding
  implementation_binding: ImplementationBinding
  route_binding: RouteBinding
}
```

It answers five different questions that must stay separate:

| Question | Field |
|---|---|
| What is being called? | `descriptor_contract` |
| Who publicly advertises it? | `owner_binding` |
| Who is allowed to advertise/invoke it? | `authority_binding` |
| How is it executed? | `implementation_binding` |
| Where/how is it routed? | `route_binding` |

This is the key distinction behind pages/files/API-key catalog convergence:
many of these abilities have static descriptor contracts but dynamic
principal-projected owners.

## DescriptorContract

`DescriptorContract` describes the governed interface. Its source can vary:

```text
DescriptorContractSource =
  StaticBuiltinTemplate
| PluginManifest
| HostedAgentManifest
| RuntimeGeneratedManifest
```

`StaticBuiltinTemplate` means the schema and descriptor semantics are stable in
the daemon source tree. It does not imply a fixed system owner.

Required descriptor facts:

```text
AbilityDescriptor {
  ability_ura
  public_name
  version
  schema_hash
  call_mode: rpc | stream | bidi
  admission_action
  subject_contract
  receipt_semantics
}
```

## OwnerBinding

`OwnerBinding` describes how the public callable owner/callee is derived.

```text
OwnerBinding =
  FixedSystemAgentOwner
| PrincipalProjectedUserOwner
| PrincipalProjectedServiceOwner
| HostedAgentOwner
| PluginDeclaredOwner
| RealmAuthorityOwner
```

Examples:

| Ability family | Correct owner binding |
|---|---|
| `runtime-introspection.meta.list_resources` | `FixedSystemAgentOwner` |
| `service/<user>.pages.project_list` | `PrincipalProjectedServiceOwner` |
| `service/<user>.pages.pages.publish` | `PrincipalProjectedServiceOwner` |
| `user/<user>.files.put` | `PrincipalProjectedUserOwner` or explicitly service-projected if the file store is service-owned |
| `user/<user>.api_key.create` | `PrincipalProjectedUserOwner` |
| hosted agent `.chat` / `.discover` | `HostedAgentOwner` |
| plugin-declared remote desktop ability | `PluginDeclaredOwner` or `FixedSystemAgentOwner`, depending on product ownership |

The owner binding must produce a canonical callable owner URA. That owner URA
is the invocation `callee` when it advertises the selected descriptor.

## AuthorityBinding

`AuthorityBinding` is not metadata decoration. It is part of the publication's
validity proof.

```text
AuthorityBinding {
  advertise_predicate(owner, descriptor, context)
  invoke_predicate(caller, callee, ability, subject, causal_context)
  authority_proof
}
```

The invocation predicate must inspect the complete invocation tuple. A call
that hides or silently defaults `subject`, `nonce`, or `causal_context` is not
semantically complete enough to produce a durable receipt.

## ImplementationBinding

`ImplementationBinding` connects a descriptor version to executable code.

```text
ImplementationBinding =
  NativeDaemonImpl
| PluginImpl
| HostedAgentImpl
| RemoteForwardedImpl
| CompositeMissionImpl
| DescriptorOnly
```

`DescriptorOnly` is explicit: discoverable but not invokable. It must not
create a runtime handler row or execution index.

Implementation facts must include at least:

```text
AbilityImpl {
  descriptor_version
  impl_hash
  runtime_env
  call_mode
  entrypoint
}
```

## RouteBinding

`RouteBinding` selects execution without redefining identity.

```text
RouteBinding {
  callee_ura
  execution_host_ura
  descriptor_ref
  dispatch_key
  locality: local | remote
  transport
}
```

These fields are intentionally distinct:

| Field | Meaning |
|---|---|
| `callee_ura` | Callable owner that advertises the descriptor. |
| `execution_host_ura` | Device/daemon/runtime host where implementation executes. |
| `descriptor_ref` | Versioned descriptor selected for this call mode and admission action. |
| `dispatch_key` | Local registry key used to find the executable handler. |

A correct implementation may map multiple owner-projected ability URAs to one
native daemon handler. That mapping must be represented by
`ImplementationBinding` / `RouteBinding`, not by forcing the public ability
name, descriptor file path, and handler key to be identical.

## Invocation and receipt

Every public call preserves the seven-field invocation form:

```text
invoke(caller, callee, ability, subject, nonce, causal_context, args) -> receipt
```

The resulting `Receipt` must bind:

```text
Receipt {
  invocation_hash
  descriptor_version
  schema_hash
  impl_hash
  runtime_env
  authority_proof
  input_hash
  output_hash
  parent_receipts
  terminal_state
  signature
}
```

For stream and bidi/session-oriented abilities, terminal state is not optional.
The session path must emit explicit lifecycle facts such as opened, frame
accepted, closed, failed, or cancelled.

## Canonical classifications by concrete use case

### Runtime introspection

```text
meta.list_resources =
  StaticBuiltinTemplate
+ FixedSystemAgentOwner
+ NativeDaemonImpl
+ local RouteBinding
```

The public owner is a device-sponsored `SystemAgent`, not the Device itself.

### Pages

```text
project_list / pages.publish / pages.get / pages.unpublish =
  StaticBuiltinTemplate
+ PrincipalProjectedServiceOwner(service/<user>.pages)
+ NativeDaemonImpl
+ local-or-remote RouteBinding
```

The pages service is a user-owned service surface. The descriptor contract can
be static, but the owner/callee is projected from the principal. Static catalog
tests must therefore validate the template and the projection separately.

### Files

```text
files.put / files.get / files.list =
  StaticBuiltinTemplate
+ PrincipalProjectedUserOwner or PrincipalProjectedServiceOwner
+ NativeDaemonImpl
+ local-or-remote RouteBinding
```

The chosen owner must match the product semantics of the file store. The
important invariant is that files are not fixed system-agent abilities merely
because their native implementation lives in the daemon.

### API keys

```text
api_key.create / api_key.list / api_key.revoke =
  StaticBuiltinTemplate
+ PrincipalProjectedUserOwner
+ NativeDaemonImpl
+ governance AuthorityBinding
```

These are governance abilities. The owner projection and subject contract must
make clear which user's keyring state is being managed.

### Remote desktop / RemoteApp

```text
remote-desktop.* =
  PluginManifest or StaticBuiltinTemplate
+ PluginDeclaredOwner or FixedSystemAgentOwner
+ PluginImpl
+ Session RouteBinding
+ lifecycle receipts
```

If the capability is device-native and security-sensitive, a device-sponsored
`SystemAgent` owner is appropriate. If the capability is contributed by an
installed plugin with its own product identity, use `PluginDeclaredOwner`.
Either way, the plugin is implementation packaging, not the public callee.

## Implementation consequences

The daemon catalog should enforce these rules:

1. Publish `AbilityPublication` records, not flat system ability names.
2. Validate descriptor templates separately from owner-projected publication instances.
3. Resolve descriptor paths from `DescriptorContractSource`, not from rendered owner-prefixed ability names.
4. Resolve handlers through `ImplementationBinding` / `dispatch_key`, not by assuming public ability name equals registry key.
5. Resolve routes through `RouteBinding`, preserving `callee_ura` and `execution_host_ura` as separate facts.
6. Keep descriptor-only rows discoverable but non-invokable.
7. Require real invoke coverage for executable publications, including projected-owner native daemon abilities.
8. Require stream/bidi lifecycle tests for session-oriented publications.

The current class of failures around pages/files/API-key publication is
explained by violating rules 2 through 4: static descriptor templates with
principal-projected owners were treated as fixed system abilities, so rendered
owner names leaked into descriptor path and handler lookup.

## Current code mapping

The current implementation exposes a first-class local
`AbilityPublication` read model for committed callable catalog rows. It is
still assembled from lower-level domain objects, but the publication gate now
keeps the five facets together before route and directory code can consume a
row:

| Ontology field | Current code owner | Notes |
|---|---|---|
| `DescriptorContract` | `AbilityDescriptor`, generated TOML contracts, and `system_manifest` | Static template identity must be validated independently from owner-projected publication instances. |
| `OwnerBinding` | `OwnerKind`, `AuthorityScope.owner_projection`, and owner URA construction helpers | Service, SystemAgent, HostedAgent, and projected user/service owners must stay explicit; do not infer owner from ability prefix. |
| `AuthorityBinding` | `AuthorityBinding`, `AuthorityBindingRegistry`, session/delegation authority admission | Both advertise and invoke must be checked. Publication-only success is not enough. |
| `ImplementationBinding` | `AbilityImplBinding`, `ControlPlaneImplementation`, handler slot registration | Native daemon, plugin, hosted agent, MCP, and descriptor-only implementations are implementation facts, not public identities. |
| `RouteBinding` | `SelectedInvokeRoute`, `SelectedAbilityRoute`, owner projection rows, presence/session routing | Route resolution owns `callee_ura`, `execution_host_ura`, `descriptor_ref`, transport locality, and local `dispatch_key`. |
| Local publication read model | `AbilityPublication` and `LocalAbilityPublicationSnapshot` | Captured view of committed, bound local publications. It resolves by owner, public name, and `call_mode`; public name alone is insufficient. |
| Executable committed row | `AbilityControlPlaneRecord` plus the authority-keyed execution binding index | `AbilityControlPlaneRecord` aggregates descriptor, authority, and implementation. The execution binding index records either local handler slots or external daemon-invocation route modes, so daemon exact routes such as `identity.register_pubkey` do not appear as `unbound`. |

Tests must pin the split:

1. descriptor contract path lookup is template/source-driven;
2. owner projection is explicit and may be principal-scoped;
3. implementation lookup uses control-plane binding and handler slot facts;
4. route resolution selects descriptor geometry by `call_mode`;
5. service owners route through a host Device without becoming Agent rows.

## Anti-patterns

Reject these designs:

```text
User == Agent
Device == public callee
Plugin == Ability
AbilityDescriptor == AbilityImpl
published ability name == descriptor path == local handler key
advertise authorization == invoke authorization
ability + args == complete Invocation
```

Each of these collapses a boundary that must remain explicit for routing,
authorization, receipt proof, multi-tenant safety, and session lifecycle
correctness.

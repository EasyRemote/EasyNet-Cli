# EasyNet-Cli Architecture State

**Status:** current architecture index.

This file states the current ownership boundaries. It deliberately does not
preserve superseded L2 mission-dispatch claims as active architecture rules.
Historical designs belong in dated RFCs and plan packs, never in this source of
truth.

## Normative ownership

```text
Axon             URA/DescriptorRef grammar, canonical Invocation, signatures,
                 receipt verification, call modes and cross-node wire semantics
EasyNet daemon   device/Hub lifecycle, descriptor projection, authority and
                 implementation binding, local policy, plugins, Mission/EAL,
                 resources, and local dispatch
CLI / FFI / SDK  product-neutral lifecycle, Addressing and complete-Invocation
                 clients of the daemon
Backend          EasyNet product DTOs, read models, HTTP/UI/session/DB policy
EasyRemote       Mission, hosting, publication and remote-control ergonomics
```

The full ontology and control-plane rules are in
[`design/ability-control-plane-model.md`](design/ability-control-plane-model.md)
and [`easynet_ontology.tex`](easynet_ontology.tex). The daemon/runtime split is
defined by [`design/daemon-layers-v1.md`](design/daemon-layers-v1.md) and the
current project layout by [`spec/project-structure-v1.md`](spec/project-structure-v1.md).

## Ability control plane

- `AbilityDescriptor` is the versioned governed interface.
- `AuthorityBinding` governs both advertisement and invocation.
- `AbilityImplBinding` is the local executable binding.
- `AbilityManifest` is daemon import/persistence data. It may describe an
  executable package, boot probe, or health probe; it is not a second public
  interface model. Static catalog metadata and manifests are normalized before
  the control-plane commit; discovery never retains or re-reads a manifest
  side table.
- `ManifestAccessScope`, descriptor `Visibility`, and `PageVisibility` are
  distinct concepts with distinct names.
- Descriptor `CallMode` is the daemon's single transport-mode vocabulary and
  maps to Axon only at the Axon boundary. Transition semantics are receipt and
  state-machine facts, not a fourth transport mode.
- Catalog registration always names its owner explicitly. Daemon assembly is
  fallible: durable-agent or plugin indexing/registration errors abort the
  build instead of publishing a plausible but incomplete catalog.
- Hot Agent lifecycle uses a controlled authority inventory. A durable,
  validated lifecycle record is enrolled before runtime/catalog publication;
  failure rolls back completed stages and stop revokes the enrollment. A
  boot-window no-op or restart instruction is not a successful transition.

## Invocation and process boundaries

Every product call submitted to the daemon carries the complete signed
Invocation tuple. `daemon.sock` is the product Invocation surface;
`control.sock` is lifecycle/diagnostics only. The runtime-dispatch socket is a
private Axon-to-daemon execution bridge and must preserve, rather than replace,
the calling Invocation context.

Cross-shard Axon transport uses `InvocationRelay` and forwards one complete
`InvokeRequest` unchanged. Product-shaped forwarding wrappers, inner JSON
envelopes, and caller reconstruction are not protocol paths.

## SDK boundary

Language facades consume the generic daemon runtime model: daemon lifecycle,
complete Invocation, stream/bidi, typed error, Axon-delegated Addressing,
authority and opaque terminal receipt facts. Product-shaped convenience APIs,
directory views and receipt-history projections belong in EasyNet backend,
EasyRemote or another downstream product—not in the runtime SDK—and must not
introduce another transport, URA grammar or canonical model.

The current native surface is generic C ABI v5 with an exact 54-symbol
allowlist. Go and Python expose the same product-neutral capability-state
matrix. Node, Java and Swift expose supported subsets; they do not publish
placeholder product clients. Removed product profiles have no aliases and no
binding probes a retired C symbol.

The normative SDK contract is
[`spec/daemon-sdk-requirements-v1.md`](spec/daemon-sdk-requirements-v1.md); the
public object graph is [`../sdk/SDK_INTERFACE_SPEC.md`](../sdk/SDK_INTERFACE_SPEC.md)
and the machine state is
[`../sdk/conformance/sdk-parity-matrix.json`](../sdk/conformance/sdk-parity-matrix.json).
No other document is a current SDK architecture ledger.

## Mechanical guards

- `tools/scripts/check-kernel-boundary.sh`
- `tools/scripts/check-invocation-unity.sh`
- `tools/scripts/check-dispatch-boundary.sh`
- `tools/scripts/check-subservice-isolation.sh`
- `tools/scripts/check-ffi-abi-v5-header.sh`
- `tools/scripts/check-ability-model-convergence.sh`
- `tools/scripts/check-sdk-parity-matrix.sh`
- `tools/scripts/check-sdk-product-neutrality.sh`

Architecture changes must update the relevant normative document and add a
targeted invariant test or repository guard.

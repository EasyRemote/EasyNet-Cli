# EasyNet-Cli Architecture State

**Status:** current architecture index.

This file states the current ownership boundaries. It deliberately does not
preserve superseded L2 mission-dispatch claims as active architecture rules.
Historical designs belong in dated RFCs and plan packs, never in this source of
truth.

## Normative ownership

```text
Axon             canonical Invocation, signatures, receipt verification,
                 protocol lifecycle, and cross-node wire semantics
EasyNet daemon   device/Hub lifecycle, descriptor projection, authority and
                 implementation binding, local policy, plugins, Mission/EAL,
                 resources, and local dispatch
CLI / FFI / SDK  lifecycle and complete-Invocation clients of the daemon
Backend          product HTTP/UI/session/DB policy; invokes the daemon only
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
  interface model.
- `ManifestAccessScope`, descriptor `Visibility`, and `PageVisibility` are
  distinct concepts with distinct names.
- Descriptor `CallMode` is the daemon's single transport-mode vocabulary and
  maps to Axon only at the Axon boundary. Transition semantics are receipt and
  state-machine facts, not a fourth transport mode.

## Invocation and process boundaries

Every product call submitted to the daemon carries the complete signed
Invocation tuple. `daemon.sock` is the product Invocation surface;
`control.sock` is lifecycle/diagnostics only. The runtime-dispatch socket is a
private Axon-to-daemon execution bridge and must preserve, rather than replace,
the calling Invocation context.

## SDK boundary

Language facades consume the generic daemon runtime model: daemon lifecycle,
complete Invocation, stream/bidi, typed error, and stable neutral DTOs.
Product-shaped convenience APIs live above the C ABI and must not introduce
another transport, receipt, directory, or lifecycle model.

## Mechanical guards

- `tools/scripts/check-kernel-boundary.sh`
- `tools/scripts/check-invocation-unity.sh`
- `tools/scripts/check-dispatch-boundary.sh`
- `tools/scripts/check-subservice-isolation.sh`

Architecture changes must update the relevant normative document and add a
targeted invariant test or repository guard.

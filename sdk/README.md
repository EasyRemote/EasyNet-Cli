# Canonical Runtime SDK

This directory contains product-neutral language bindings for the governed
runtime hosted by native runtime providers. The SDK is not a collection of
product APIs. Downstream applications build typed local facades on top of
complete generic Invocation.

## Current model

| Layer | Responsibility |
| --- | --- |
| Axon | URA and descriptor-reference grammar, canonical bytes, admission, call modes and receipt cryptography |
| Rust runtime | native provider implementation and generic handle semantics |
| C ABI v6 | exact generic lifecycle, Invocation, stream/bidi, health, Addressing and memory boundary |
| Go/Python | provider-backed projections of the same runtime object graph |
| Node/Java/Swift | supported subsets of that graph; absent capabilities are explicitly unsupported |
| downstream products | ability names, request/result DTOs and workflows outside the SDK |

The object graph and prohibitions are defined in
[`SDK_INTERFACE_SPEC.md`](SDK_INTERFACE_SPEC.md). The normative requirements
are in
[`../docs/spec/daemon-sdk-requirements-v1.md`](../docs/spec/daemon-sdk-requirements-v1.md).

## Runtime surface

- environment and daemon/runtime lifecycle;
- canonical Addressing delegated to Axon;
- runtime identity and sign-only capabilities;
- managed-signing lifecycle through the runtime key service;
- product-neutral PrincipalLifecycle and public-key bindings;
- canonical Directory resolution/subscription;
- governed AbilityDescriptor and authority metadata projection;
- complete Invocation build, prepare, sign, invoke, submit and handle state;
- ordered server streams and bidirectional sessions;
- typed errors, runtime health and diagnostics;
- receipt/causal facts and runtime event cursors;
- product-neutral runtime administration.

The SDK does not expose downstream workflow clients, application account
policy, UI-specific directory or receipt pages, product event presentations,
route/model/page/file helpers, desktop application lifecycle, or convenience
wrappers. Those concepts live with the product that owns their behavior and
consume the generic capabilities above.

## ABI

ABI v6 is a hard major cut. The authoritative header and export list are:

- [`../include/easynet_cli.h`](../include/easynet_cli.h)
- [`../include/easynet_cli.exports.v6`](../include/easynet_cli.exports.v6)
- [`../docs/spec/ffi-abi-v6.md`](../docs/spec/ffi-abi-v6.md)

There is no v4 domain-symbol compatibility path.

## Capability state

[`conformance/sdk-parity-matrix.json`](conformance/sdk-parity-matrix.json) is
the machine source of truth for Go/Python capability state. Every row is one of
`unsupported`, `seam`, `provider-backed` or `cutover-ready` and cites
executable evidence. [`SDK_PARITY.md`](SDK_PARITY.md) is the readable summary.

## Repository layout

- `go/`, `python/`, `node/`, `java/`, `swift/`: language bindings;
- `schemas/`: generic public DTO schemas only;
- `conformance/cases/`: language-neutral runtime cases only;
- `conformance/fixtures/`: canonical runtime fixtures;
- `CONFORMANCE_SUITE.md`: runner and evidence contract.

Downstream product conformance belongs in the downstream repository. SDK tests
may verify that a downstream import boundary is clean, but may not reintroduce
its product model as an SDK profile.

## Package metadata

Package names, module coordinates, language versions and typed-package markers
are machine-checked so local and CI builds resolve the same SDK. This metadata
is not stable release evidence: publish credentials, registry availability and
consumer release stability remain separate delivery concerns.

## Non-negotiable boundaries

- no local URA or DescriptorRef grammar;
- no parallel Invocation or call-mode taxonomy;
- no product ability literals in public bindings;
- no product-domain C symbols;
- no parallel PrincipalLifecycle, trust store, key inventory or recovery truth;
- no service locator or optional-provider fallback;
- no load-error-to-empty/default behavior;
- no legacy address-era names for URA semantic identity.

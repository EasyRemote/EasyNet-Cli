# EasyNet Daemon SDK

This directory is the public Daemon SDK workspace for EasyNet-Cli. The SDK
controls and calls `easynet-daemon`; it is not a command-line wrapper around
the `easynet` binary.

The semantic implementation lives in the Rust crate and is projected through
the C ABI. Go, Python, Node, Java, Swift, and future language packages are
facades over the same object model, DTO schemas, and conformance cases.

## Current Status

| Area | Status |
| --- | --- |
| Rust Runtime Core | partial: lifecycle attach, runtime client DTOs, draft/prepare/sign/submit objects |
| C ABI | partial ABI v4 Runtime Core projection |
| Schemas | scaffold: public DTO names and required files exist |
| Conformance | scaffold: shared case and fixture format exists |
| Go facade | placeholder |
| Python facade | placeholder |
| Node facade | placeholder |
| Java facade | placeholder |
| Swift facade | placeholder |

No language package should claim stable Daemon SDK support until its declared
profiles pass `sdk/conformance` cases and expose no raw Axon/proto/runtime
types.

## Files

- `SDK_INTERFACE_SPEC.md`: implementation-facing object model and lifecycle
  contract.
- `SDK_PARITY.md`: language tiers, profile status, and known gaps.
- `CONFORMANCE_SUITE.md`: fixture and runner contract.
- `schemas/`: public JSON DTO projections shared by bindings.
- `conformance/`: language-neutral cases and golden fixtures.

## Ownership Rule

Axon owns protocol semantics such as URA grammar, DescriptorRef
canonicalization, Invocation canonical bytes, admission, and receipt
verification. EasyNet-Cli SDK owns daemon lifecycle, local transport, typed
Daemon SDK DTOs, profile clients, and language binding projection.

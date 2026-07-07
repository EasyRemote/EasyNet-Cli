# EasyNet Daemon SDK

This directory is the public Daemon SDK workspace for EasyNet-Cli. The SDK
controls and calls `easynet-daemon`; it is not a command-line wrapper around
the `easynet` binary.

The semantic implementation lives in the Rust crate and is projected through
the C ABI. Go, Python, Node, Java, Swift, and future language packages are
facades over the same object model, DTO schemas, and conformance cases.

## Current Status

The authoritative capability state is `sdk/conformance/sdk-parity-matrix.json`
and the human-readable summary is `SDK_PARITY.md`. This README is only the
workspace entrypoint; it must not drift into a second status ledger.

| Area | Current state |
| --- | --- |
| Rust Runtime Core | source-of-truth implementation substrate for daemon SDK semantics and C ABI projection |
| C ABI | ABI v4 projection with opaque handles, feature discovery, runtime core, and shipped profile carrier/projection entry points |
| Schemas | shared DTO schema set for Runtime Core, profile clients, stream/bidi terminal projections, and conformance fixtures |
| Conformance | shared cases, fixtures, manifest runner, Rust/C ABI/Go/Python action-adapter reports, and Node/Java/Swift seam action-adapter reports |
| Go facade | provider-backed for the shipped P0 Hub/backend profiles listed in `SDK_PARITY.md` |
| Python facade | provider-backed for the shipped P0 EasyRemote/local automation profiles listed in `SDK_PARITY.md` |
| Node / TypeScript facade | Runtime Core seam over injected transports with Health, Authority, Directory + Identity, Receipt, Publication, Host Binding, Mission, Admin + Gateway, Events, Surface, Compatibility, and Wrappers profile seams; daemon providers remain unsupported |
| Java / JVM facade | Runtime Core seam as a Maven package for typed errors, feature discovery, complete Invocation drafts, injected runtime transport, and bounded stream/bidi state; daemon providers remain unsupported |
| Swift facade | Runtime Core seam as a Swift Package Manager package for typed errors, feature discovery, complete Invocation drafts, injected runtime transport, and bounded stream/bidi state; daemon providers remain unsupported |

P0 consumer cutover readiness is tracked by
`tools/scripts/check-sdk-cutover-readiness.sh` and summarized by
`tools/scripts/check-sdk-completion-audit.sh`. Language profile rows remain
provider-backed evidence; product cutover claims must cite the aggregate gates.

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

# Java and Swift Health Seam Plan

## Goal

Converge the Java and Swift SDK packages one step beyond Runtime Core by adding
the shared Health seam already exercised by Rust, C ABI, Go, Python, and Node.

## Scope

- Add generic Runtime Health DTOs and diagnostics DTOs to Java and Swift.
- Add narrow injected health transports and clients for decoding shared health
  JSON payloads.
- Add failure-path tests for missing transport capability, transport failure,
  malformed payloads, closed clients, and control-only readiness state.
- Update conformance reports, scaffold guards, and status documents.

## Non-Goals

- No daemon or C ABI provider for Java or Swift.
- No product health model.
- No product lifecycle, directory, receipt, identity, or profile expansion.
- No legacy input aliases.

## Capability State

Java and Swift Health move from `unsupported` to `seam`. Provider-backed and
cutover-ready states remain unsupported until a daemon or C ABI transport is
implemented and proven.


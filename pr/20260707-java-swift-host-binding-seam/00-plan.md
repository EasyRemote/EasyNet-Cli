# Java/Swift Host Binding Seam Plan

## Goal

Converge Java and Swift P1 facades with the shared Host Binding profile seam
from the daemon SDK SPEC.

This iteration covers:

- `host_binding/codec_hash`

## Scope

- Add Java and Swift Host Binding DTOs for binding requests, binding
  projections, request envelopes, decoded host requests, item/error/terminal
  frames, terminal summaries, and output hash state.
- Add `HostBindingClient` and `HostBindingTransport` seams over injected
  transports.
- Add explicit host-stream lifecycle controller/provider state machines for
  readiness and idempotent cleanup.
- Validate absolute endpoints, fixed frame schema, frame variants, hash state
  cursor invariants, and sequence ordering before transport calls.
- Update Java/Swift conformance reports, scaffold checks, and status docs.

## Non-Goals

- No Java/Swift daemon/C ABI provider transport.
- No user-code execution or product host process supervision.
- No plugin lifecycle, package scanning, or product catalog behavior.
- No product-specific host naming or EasyRemote lifecycle in SDK code.

## Verification

- `tools/scripts/check-java-sdk-seam.sh`
- `tools/scripts/check-swift-sdk-seam.sh`
- `tools/scripts/check-sdk-conformance-reports.sh`
- `tools/scripts/check-sdk-scaffold.sh`
- `tools/scripts/check-sdk-ura-naming.sh`
- `tools/scripts/check-sdk-package-metadata.sh`
- `git diff --check`

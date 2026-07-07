# Java/Swift Events Seam Plan

## Goal

Converge Java and Swift P1 facades with the shared Events profile seam from the
daemon SDK SPEC.

This iteration covers:

- `events/directory_stream`
- `events/device_invocation_history`
- `events/session_stream`

## Scope

- Add Java and Swift Events profile DTOs for carrier bases, filters, cursors,
  event frames, device history pages, projection inputs, and event streams.
- Add `EventClient` and `EventTransport` seams over injected transports.
- Require complete Invocation carrier fields for event subscription/history
  requests.
- Preserve daemon-owned `session_id` semantics and reject product
  `session_ura` parsing.
- Project drop reports, terminal frames, and bounded device history pages from
  shared fixtures.
- Update conformance reports, scaffold checks, and Java/Swift status docs.

## Non-Goals

- No provider-backed Java/Swift daemon transport.
- No backend SSE/WebSocket subscriber registry.
- No product-specific event filtering or authorization.
- No SDK-local event fanout loop or polling-only replacement for live streams.

## Verification

- `tools/scripts/check-java-sdk-seam.sh`
- `tools/scripts/check-swift-sdk-seam.sh`
- `tools/scripts/check-sdk-conformance-reports.sh`
- `tools/scripts/check-sdk-scaffold.sh`
- `tools/scripts/check-sdk-ura-naming.sh`
- `tools/scripts/check-sdk-package-metadata.sh`
- `git diff --check`

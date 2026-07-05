# Decisions Log

## 2026-07-04

- Decision: implement Admin + Gateway carrier/status partial before Surface and
  Compatibility.
- Rationale: the daemon already owns lifecycle status, control discovery, agent
  registry lifecycle, and `session.list`. This creates a real SDK boundary
  instead of inventing product/backend admin semantics.

## 2026-07-04

- Decision: keep pairing-token minting, ACME/TLS policy, browser session UX,
  and full device-session CRUD out of this slice.
- Rationale: those require backend product state or daemon abilities not yet
  present as stable runtime contracts. Marking them complete through SDK
  projection would violate the SPEC's ownership boundary.

## 2026-07-05

- Decision: align the Go C ABI Admin transport with Python for gateway status by
  attaching a daemon lifecycle handle, reading `easynet_daemon_status`, and
  delegating the readiness DTO projection to Rust.
- Rationale: `GatewayStatus` is a daemon lifecycle read model, so the Go facade
  can retrieve status facts without owning readiness semantics or fabricating
  trust/pairing mutation results.

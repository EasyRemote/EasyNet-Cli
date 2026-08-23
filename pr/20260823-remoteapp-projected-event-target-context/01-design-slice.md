# RemoteApp projected event target-context boundary

## Intent

Make every session-owned RemoteApp lifecycle/transport event independently
auditable against the selected Resource/window/application. Terminal receipts
already carry target evidence, but terminal lifecycle events such as
`SESSION_CLOSING`, `SESSION_CLOSED`, and timeout closure were still emitted as
generic event payloads, leaving event-log top-level target fields null.

## Boundary decision

The session aggregate owns the live selected target. Therefore target context
must be attached at the aggregate's `push_projected_event` boundary, not copied
piecemeal by callers or inferred by frontend verifiers.

Target-tracking events remain separate because their payloads may intentionally
describe pending or previous target state. The aggregate only enriches
session-owned `RemoteDesktopEventProjection` rows.

## Invariants

1. The event projection module may format payloads, but it does not own target
   truth.
2. `RemoteDesktopSession::push_projected_event` attaches current binding
   evidence before writing the event log.
3. Existing explicit target-bound projections remain compatible.
4. Terminal lifecycle events must carry selected target evidence in both
   payload and event-log top-level fields.

## Verification

- Focused session tests for cancel/close and timeout terminal events.
- Session event projection tests for terminal payload compatibility.
- `check-remoteapp-product-closure-audit.sh` must require the aggregate-level
  target-context boundary.


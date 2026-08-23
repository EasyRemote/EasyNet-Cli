# RemoteApp WebRTC diagnostic target-context slice

## Intent

Bind WebRTC diagnostic events to the selected RemoteApp target. Network
fallback evidence depends on ICE/WebRTC diagnostics, but this path writes
directly through `push_event` because the event type is dynamic. That bypasses
the aggregate-level projected-event target enrichment.

## Boundary decision

The session aggregate remains the owner of the selected target. The event
projection module provides a reusable payload enrichment helper, while
`RemoteDesktopSession::record_webrtc_diagnostic` decides when to attach current
binding evidence before committing the event log row.

## Invariants

1. WebRTC diagnostics must keep their dynamic event type.
2. WebRTC diagnostics must carry current `subject_ura`, binding epoch, target
   identity epoch, geometry revision, media source epoch, consent epoch, and
   nested `target_binding`.
3. The signaling state update still sees the raw diagnostic payload before
   projection metadata is added.
4. The event log top-level target fields must be populated from the enriched
   payload.

## Verification

- Add a focused `record_webrtc_diagnostic_projects_target_binding_context`
  session aggregate test.
- Extend the RemoteApp product closure audit so WebRTC diagnostic evidence
  cannot regress to target-less event rows.


# Intent

## Goal

Replace admission-layer string classification of resolver owner-offline route negatives with an explicit typed route failure fact.

## Non-goals

- Do not change public gRPC status messages or route-negative JSON shape.
- Do not modify product spec documents owned by the user.
- Do not add compatibility parsing for legacy route-negative messages.

## Acceptance criteria

- Owner-offline route negatives are identified from resolver-owned typed state.
- Admission does not inspect human diagnostic text to choose transport status.
- Existing external diagnostics remain visible.
- Focused route/admission tests, formatting, architecture gates, SPEC v2 gate, and codegraph checks pass.

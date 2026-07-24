# Intent

## Goal

Promote the remote-desktop product contract boundary into the canonical runtime
convergence v2 gate so retired product wire aliases cannot return when engineers
run only the SPEC gate.

## Non-goals

- Do not reintroduce compatibility aliases for remote-desktop transport names.
- Do not change the canonical `webrtc` product wire spelling.
- Do not add a product-specific SDK abstraction.

## Acceptance criteria

- The SPEC v2 gate invokes the remote-desktop contract boundary.
- The SPEC v2 self-test proves the gate rejects the retired `web_rtc` alias.
- Existing standalone boundary script and script-check coverage remain valid.

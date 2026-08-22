# RemoteApp diagnostic bidi input telemetry

RemoteApp diagnostic InvokeBidi input uses the same parser and effective policy
as the WebRTC input data channel, but its response payloads did not project the
frontend `client_sequence` / `sent_at_ms` telemetry. That leaves probes unable
to correlate a diagnostic input frame with daemon policy/application outcomes.

This change keeps session authority unchanged and only closes the observability
gap:

- parse and validate input telemetry in the shared input parser;
- project telemetry in diagnostic bidi input applied/warn responses;
- keep production data-channel event projection as the authoritative session
  event path.

# Invariants

1. Diagnostic InvokeBidi input must not define a second input schema.
2. Diagnostic responses may echo client telemetry for correlation, but must not
   use it for authority, session state, or target binding decisions.
3. The same `RemoteDesktopInputFrame` parser validates `client_sequence` and
   `sent_at_ms` before diagnostic or production input policy application.
4. `target_input_not_ready` responses must preserve input telemetry because
   they are the most important diagnostic case for window/application tracking.

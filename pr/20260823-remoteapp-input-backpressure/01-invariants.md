# Invariants

1. RemoteApp high-rate input must not create an unbounded browser-side
   data-channel backlog.
2. Input frames remain session/data-channel scoped; session ownership is not
   moved into frontend JSON.
3. `client_sequence` is frontend telemetry only. It does not replace daemon
   session state, Invocation receipts, transport epoch, or target geometry
   checks.
4. The plugin rejects malformed sequence values before input policy or OS
   injection.
5. Applied and rejected daemon events project both `client_sent_at_ms` and
   `client_sequence` so latency/loss diagnosis can correlate browser and host
   observations.

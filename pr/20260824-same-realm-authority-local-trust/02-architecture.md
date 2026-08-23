# Architecture

`DeviceTrustSync` owns caller classification before admission. It now models a
same-realm Authority separately from external callers:

- `LocalAuthority` consults the existing `SharedTrustAnchor` and never enters
  the session-backed resolver.
- `ExternalCaller` retains the Hub-attested ephemeral projection used for
  cross-realm User and Authority signatures.

This keeps trust direction explicit: pairing/session setup provisions the local
Authority; federation resolves foreign authorities. Invocation dispatch only
consumes those trust facts.

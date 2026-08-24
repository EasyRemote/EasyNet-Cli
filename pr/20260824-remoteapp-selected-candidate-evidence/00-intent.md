# Intent

Real RemoteApp WebRTC sessions selected and nominated a successful ICE
candidate pair, but the media stats projected both candidate types and the
route class as null. The `rtc` report stores candidate entries under
side-qualified report IDs while candidate-pair rows reference the underlying
candidate IDs. Exact-key lookup therefore could never join the rows.

This slice restores the deterministic stats join so direct, STUN and relay
paths can be classified from the selected pair without exposing candidate
addresses or credentials.

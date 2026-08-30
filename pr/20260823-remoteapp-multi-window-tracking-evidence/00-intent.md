# RemoteApp multi-window tracking evidence gate intent

RemoteApp product readiness requires independent window/application tracking as
an execution effect. Source data structures, unit tests, and state-machine
contracts are necessary but do not prove that real streams remain bound to the
right OS targets during window churn.

This batch adds a runner-agnostic verifier for live app/window churn artifacts.
It preserves the architecture boundary:

- the selected display/window/application Resource URA remains the Invocation
  subject;
- the RemoteDesktop plugin owns native target observation, capture, and media
  rebind execution;
- WebRTC/native media remains session transport;
- target lifecycle events and terminal receipts remain visible through public
  RemoteApp session abilities.

Self-test evidence only proves the verifier contract. It is not product
readiness evidence.

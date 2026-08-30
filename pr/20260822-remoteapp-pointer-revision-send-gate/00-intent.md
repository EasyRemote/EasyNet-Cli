# RemoteApp pointer revision send gate

## Intent

Record and gate the product rule that browser RemoteApp pointer frames must
carry the current daemon-projected target geometry revision before they are sent
over the WebRTC input data channel.

## Boundary

- EasyNet frontend performs pre-send UI gating.
- EasyNet-Cli daemon remains the authoritative input policy and OS injection
  boundary.
- This does not alter Axon invocation, receipt, or stream semantics.

## Product effect

The RemoteApp input path moves closer to product behavior by preventing stale
target-local pointer floods from leaving the browser when the active session
view already proves the frame cannot map to the current target geometry.

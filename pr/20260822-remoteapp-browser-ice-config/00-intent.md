# RemoteApp browser ICE config convergence

## Intent

Close the RemoteApp WebRTC configuration seam where the device endpoint reads
deployment STUN/TURN/EasyNet relay route configuration, but the browser-side
RTCPeerConnection is created with an empty `iceServers` list.

## Boundary

- Axon invocation semantics do not change.
- EasyNet-Cli RemoteApp plugin owns the transport/session projection.
- EasyNet frontend consumes the session view and configures the browser WebRTC
  transport; it does not invent relay policy.

## Product effect

Configured STUN/TURN/EasyNet relay routes can enter both sides of ICE
negotiation. This improves the NAT/relay product path, but it is not itself
proof that real deployed relay reachability has been validated.

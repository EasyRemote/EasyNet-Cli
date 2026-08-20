# RemoteApp configured ICE routes

## Intent

Converge the direct WebRTC transport route layer from a host-only seam into a provider-backed route model that can represent host, STUN, TURN, and EasyNet relay routes without changing the RemoteApp targeted-session architecture.

## Scope

- Keep `remote_desktop.*` as device-native abilities owned by the device-sponsored RemoteDesktop SystemAgent.
- Keep selected display/window/application Resource URA as the invocation subject.
- Add typed route configuration and evidence for STUN/TURN/EasyNet relay paths.
- Feed configured WebRTC ICE servers into direct WebRTC endpoint construction.
- Keep local host bind candidates separate from ICE server URLs.

## Non-goals

- Do not model remote desktop as a user-owned service.
- Do not introduce a default public STUN/TURN dependency.
- Do not leak TURN credentials in answer/event evidence.
- Do not report production route readiness just because local host candidates exist.

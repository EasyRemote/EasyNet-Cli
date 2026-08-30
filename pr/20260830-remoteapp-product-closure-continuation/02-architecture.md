# Architecture

```text
Browser RemoteApp surface
  -> EasyNet backend complete Invocation
  -> Hub/paired easynet-daemon routing
  -> RemoteDesktop SystemAgent descriptor
  -> RemoteDesktopPlugin session state machines
  -> native-host target observation/input
  -> media-host exact capture + encode
  -> leased shared-memory frame lane
  -> bounded WebRTC RTP/SRTP
  -> direct / STUN / TURN / EasyNet relay
```

The control plane and media plane meet only at session-owned identifiers,
generations, policy, and terminal settlement. FFI v9 is a generic binary Ability
stream ownership extension and is not a substitute media tunnel.

```text
native-platform (private RemoteApp platform port)
  -> ProcessInstanceProvider
       Windows: pid + process creation FILETIME
       Linux: XRes local-client pid + boot-id + /proc starttime
  -> CaptureEligibleSurface
       inventory and capture consume one predicate

Frontend RemoteDesktopSessionCoordinator
  -> creation generation / cancellation intent
  -> active session aggregate
  -> closing aggregate / terminal reconciliation
  -> replaying event watch cursor
```

The private platform port is product infrastructure, not an Axon SDK concept.
The Frontend coordinator owns client operation ordering; the daemon remains the
authority for session lifecycle and terminal receipts.

Implementation work proceeds from the owning layer: native protocol/host,
plugin session state, daemon routing, backend projection, frontend lifecycle,
then live evidence tooling.

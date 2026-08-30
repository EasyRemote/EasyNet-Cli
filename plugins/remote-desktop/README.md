# Remote Desktop Runtime Configuration

The direct WebRTC endpoint always gathers host candidates. Operators may add
explicit STUN and generic TURN providers through process environment. EasyNet
relay credentials are short-lived, session-bound leases issued by the Hub to
the daemon and injected through the plugin port; they are never static plugin
environment configuration. There are no public third-party ICE server defaults.

| Variable | Value |
|---|---|
| `EASYNET_REMOTE_DESKTOP_STUN_URLS` | Comma-separated `stun:` or `stuns:` URLs |
| `EASYNET_REMOTE_DESKTOP_TURN_URLS` | Comma-separated `turn:` or `turns:` URLs |
| `EASYNET_REMOTE_DESKTOP_TURN_USERNAME` | Standard TURN username |
| `EASYNET_REMOTE_DESKTOP_TURN_CREDENTIAL` | Standard TURN credential |

Generic TURN URL sets require both their username and credential. Credentials
must be supplied through the dedicated generic TURN fields, not embedded in the
URL. Hub-issued EasyNet relay credentials remain only in the live RemoteApp
session and are removed after terminal commit. Public route evidence reports
whether credentials are configured but never projects their values. Production
readiness remains derived from gathered ICE candidates and active media state,
not from configuration presence.

## Native process boundary

Target inventory and per-input target guards execute in the required sibling
`easynet-remoteapp-native-host` process. The daemon owns two bounded lanes,
deadlines, kill/reap, and conversion into the session aggregate. The helper owns
OS window/display observation and depends only on
`easynet-remoteapp-native-protocol` plus platform libraries; it is not a
Runtime, Agent, Service, or public Ability provider.

The helper manifest name is `native_target_observation`. It intentionally does
not claim capture or media ownership.

The canonical `easynet-remoteapp-media-host` owns capability probing and active
capture/encode generations under one stable executable identity and the
`remoteapp_media_host_v1` schema. macOS may advertise host audio only because
that same helper captures ScreenCaptureKit audio and emits validator-checked
Opus. Windows WASAPI and Linux PipeWire primitives are not product capability:
until their hosted session adapters emit Opus, device projection and SDP
admission report `active_media_session_audio_unavailable`. No daemon-local PCM
capture fallback is retained.

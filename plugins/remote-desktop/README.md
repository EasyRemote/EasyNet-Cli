# Remote Desktop Runtime Configuration

The direct WebRTC endpoint always gathers host candidates. Deployments may add
explicit STUN, TURN, and EasyNet relay providers through process environment.
There are no public third-party ICE server defaults.

| Variable | Value |
|---|---|
| `EASYNET_REMOTE_DESKTOP_STUN_URLS` | Comma-separated `stun:` or `stuns:` URLs |
| `EASYNET_REMOTE_DESKTOP_TURN_URLS` | Comma-separated `turn:` or `turns:` URLs |
| `EASYNET_REMOTE_DESKTOP_TURN_USERNAME` | Standard TURN username |
| `EASYNET_REMOTE_DESKTOP_TURN_CREDENTIAL` | Standard TURN credential |
| `EASYNET_REMOTE_DESKTOP_EASYNET_RELAY_URLS` | Comma-separated EasyNet-owned `turn:` or `turns:` URLs |
| `EASYNET_REMOTE_DESKTOP_EASYNET_RELAY_USERNAME` | EasyNet relay username |
| `EASYNET_REMOTE_DESKTOP_EASYNET_RELAY_CREDENTIAL` | EasyNet relay credential |

TURN and EasyNet relay URL sets require both their username and credential.
Malformed or incomplete configuration fails endpoint creation closed. Public
route evidence reports whether credentials are configured but never projects
their values. Production readiness remains derived from gathered ICE candidates
and active media state, not from configuration presence.

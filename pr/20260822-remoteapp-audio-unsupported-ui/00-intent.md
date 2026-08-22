# RemoteApp host-audio unsupported UI projection

## Intent

Gate the cross-repo product contract that the frontend must surface the daemon
RemoteApp host-audio unsupported state instead of letting users infer that
video/WebRTC readiness includes audio.

## Boundary

- EasyNet-Cli daemon owns the product truth: current RemoteApp media is
  video-only and host audio is not implemented.
- EasyNet frontend owns product UI projection.
- Axon invocation and stream semantics do not change.

## Product effect

Host audio remains incomplete, but the product surface now carries the explicit
blocked state end-to-end from daemon session view to frontend session details.

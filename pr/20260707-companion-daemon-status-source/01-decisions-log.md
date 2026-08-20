# Decisions Log

## Discovery Projection Over Process Names

`control.json` is the local daemon lifecycle discovery projection used by CLI and FFI clients. Companion apps now read that projection and validate the advertised PID instead of inferring daemon liveness from a global process-name scan.

## Endpoint Facts

The native companion app does not classify plugin state or own lifecycle policy. It only publishes lightweight daemon facts into its own heartbeat: runtime status plus whether the daemon advertised control and invocation endpoints for the current user.

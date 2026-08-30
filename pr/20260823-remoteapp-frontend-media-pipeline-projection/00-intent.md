# RemoteApp frontend media pipeline projection

## Intent

Make the frontend consume the daemon-owned `media_pipeline_support` projection
instead of inferring RemoteApp audio/video product readiness from media stats or
codec source presence.

The daemon remains the capability authority. The frontend renders the projected
video-only scope, codec, drop/backpressure policy, and product blockers so the
user can distinguish current video transport progress from full audio/video
product completion.

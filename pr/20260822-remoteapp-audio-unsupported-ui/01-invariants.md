# Invariants

1. `host_audio_not_implemented` remains a stable daemon blocker.
2. Frontend parses `audio` and `production_readiness.audio_*` separately from
   video transport readiness.
3. Session details show audio as blocked when the daemon says audio is not
   ready.
4. This does not count as host-audio capture/encode/WebRTC implementation.

# RemoteApp media pipeline support matrix

## Intent

Close the product-facing media capability seam without claiming unfinished
RemoteApp audio/video readiness.

The daemon already has H.264/WebRTC paths, native macOS bitrate adaptation,
bounded queues, and stale-frame drop behavior. The product surface still needs a
single capability projection that tells the frontend and Hub what the current
media pipeline can actually promise.

This task adds that projection. It does not implement host audio, degraded
network E2E, or a new media transport.

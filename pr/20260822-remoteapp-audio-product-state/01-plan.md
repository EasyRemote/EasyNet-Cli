# RemoteApp audio product state projection

## Invariant

RemoteApp video transport readiness must not imply host audio readiness. Until
the plugin owns a real host-audio capture/encode/WebRTC path, daemon session
views must expose audio as an explicit unsupported product state.

## Change

- Add an `audio` projection to the RemoteApp session/device capability view.
- Mark host audio as unsupported with a stable blocked reason instead of
  omitting it from the media contract.
- Keep existing `production_media_ready` video semantics unchanged; project
  `production_readiness.audio_ready=false` so product/E2E evidence cannot
  confuse video readiness with full audio/video readiness.
- Gate the projection in the RemoteApp performance boundary checker.

## Product effect

This closes a product-state observability seam for the `audio_video_adaptation`
row. It does not implement audio capture, codec negotiation, or audio E2E.

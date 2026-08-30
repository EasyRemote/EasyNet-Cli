# Decisions log

- Do not delete the macOS multi-display guard: one ScreenCaptureKit display filter cannot represent the complete application.
- Do not use display capture plus crop/mask as an application fallback because it can leak unrelated content.
- Reuse the bounded exact-window xcap compositor for Windows/Linux; strengthen executable proof instead of adding a second compositor.
- Treat macOS `MultiAppSurface` as a separate media-source implementation slice because it requires multi-stream capture/composition and rebind semantics.
- Update the stale target-binding gate to validate platform-specific target-model projection; the implementation already moved from a generic target model to `target_model_for_platform`.

# Decisions log

## 2026-08-24

- Use ScreenCaptureKit system audio, not microphone capture: RemoteApp must
  reproduce the selected display/application/window audio context.
- Use Opus at 48 kHz stereo because it is the browser-interoperable WebRTC
  audio codec and ScreenCaptureKit natively supports that format.
- Keep audio and video on one peer connection/session lifecycle to avoid a
  second authorization and recovery state machine.
- Keep product completion false until real decoded audio/video evidence exists.

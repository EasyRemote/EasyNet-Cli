# Invariants

1. RemoteDesktopPlugin remains the AbilityImpl; the SystemAgent remains the
   public descriptor owner.
2. Audio is transport representation inside one admitted RemoteApp session;
   it never bypasses the session token, subject binding, transport epoch, or
   terminal lifecycle.
3. One ScreenCaptureKit stream owns video and system-audio capture for one
   selected Resource URA.
4. Audio buffering is bounded and drops stale audio rather than growing
   without limit.
5. Only 48 kHz, two-channel float PCM accepted from ScreenCaptureKit is encoded
   as Opus; unexpected native formats fail explicitly.
6. The Opus encoder and WebRTC audio sender are owned by the media-loop thread
   and terminate with the existing session stop/peer-close state machine.
7. Pipeline readiness means the audio sender and encoder are negotiated and
   healthy; positive media observation is reported separately so a silent host
   does not permanently block session readiness.
8. Non-macOS platforms retain explicit unsupported product states.

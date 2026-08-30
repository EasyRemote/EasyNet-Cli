# API contract

The public RemoteApp abilities and Invocation tuple are unchanged.

The WebRTC answer gains a negotiated Opus sender when the browser offer
contains a recv-only audio transceiver. Runtime media stats gain:

- `audio_codec = "opus"`
- `audio_payload_content_type = "audio/opus"`
- `audio_sample_rate_hz = 48000`
- `audio_channels = 2`
- `audio_packets_written`
- `audio_bytes_written`
- `audio_capture_chunks_dropped`
- `audio_sender_backpressure_drops`
- `audio_ready`
- `audio_media_observed`
- `audio_blocker` only when capture/encode/send is not ready

Failures use the existing RemoteApp media failure/session event path. A missing
audio m-line is a negotiation failure for the production audio-enabled path,
not permission to silently claim audio readiness.

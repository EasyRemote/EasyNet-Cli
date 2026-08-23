# Architecture

```text
RemoteApp session / selected Resource URA
  -> one SCStream(content filter)
      -> Screen samples -> VideoToolbox H.264 -> WebRTC video track
      -> Audio samples  -> bounded PCM queue -> Opus -> WebRTC audio track
  -> existing session transport epoch / stop / receipt lifecycle
```

`screencapturekit_capture` owns platform sample extraction. A focused
`screencapturekit_audio` module owns PCM format validation, 20 ms framing, and
Opus packetization. `webrtc_endpoint` owns codec registration and track
negotiation. `webrtc_native_media` owns the single media-loop lifecycle and
projects real pipeline counters.

No policy moves into Axon or the EasyNet frontend. The frontend only offers a
recv-only audio transceiver and presents the remote MediaStream.

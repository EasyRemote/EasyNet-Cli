# Intent

Keep RemoteApp native bitrate telemetry identical to the bitrate that the
VideoToolbox encoder actually accepted. An encoder property failure must not
advance adaptation state or emit a successful bitrate-change event.

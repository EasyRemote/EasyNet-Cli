# RemoteApp media render-probe artifact gate

## Product seam

The media adaptation verifier required codec negotiation, FPS/bitrate metrics,
adaptation events, bounded queue/drop policy, host audio, and rendered media
after adaptation. It did not yet require the rendered media counters to be tied
to decoded video/audio payloads from the same RemoteApp media pipeline. A runner
could report plausible frame/audio counters without proving the rendered payload
belonged to the selected Resource URA, session, pipeline, codec, and transport.

## Slice

- Require a `render_probe` object for every media scenario.
- Bind the render probe to selected Resource URA, session id,
  `media_pipeline_id`, video codec, video transport, and audio codec.
- Require decoded video-frame and audio-sample packet counts plus non-empty
  payload fingerprints.
- Require degraded-network/backpressure render probes to observe media after the
  latest non-steady adaptation event.

## Expected impact

This still does not implement host audio or a live media runner. It closes the
evidence seam where metrics-only or synthetic counters could be accepted as
RemoteApp audio/video data-plane product evidence.

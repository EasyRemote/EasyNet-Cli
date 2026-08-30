# RemoteApp media adaptation event anchoring gate

## Product seam

The media-adaptation verifier already required baseline/degraded/backpressure
statistics, but event evidence could still be too loose: a runner could provide
correct-looking bitrate/FPS/drop numbers without proving that adaptation events
belonged to the same selected Resource, session, and media pipeline, or that
rendering continued after the impairment-triggered adaptation.

## Slice

- Require each scenario to record `scenario_started_at_ms`.
- Require every adaptation event to carry timestamp, selected Resource URA,
  session id, and media pipeline id matching the scenario.
- Require degraded-network/backpressure scenarios to record
  `impairment_applied_at_ms`.
- Require non-steady adaptation/drop/backpressure events to occur after the
  impairment timestamp.
- Require `frames_rendered_after_adaptation_at_ms` to be after the last
  adaptation event when rendered-after-adaptation evidence is claimed.

## Expected impact

This still does not prove live media product completion without a real runner
artifact. It closes the evidence seam where source-only or aggregate-only media
statistics could be confused with a causally ordered adaptation event stream.

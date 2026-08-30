# RemoteApp capture frame-source artifact gate

## Product seam

The cross-platform capture verifier rejected display fallback and required
selected/unrelated sentinel booleans, but the booleans did not prove that the
rendered frame came from the selected window/application frame source. A runner
could report `selected_sentinel_rendered=true` without binding the decoded frame
probe to the same Resource, session, target kind, frame source, and geometry
revision.

## Slice

- Require every passing capture scenario to include `target_identity`.
- Require every passing capture scenario to include `rendered_frame_probe`.
- Bind the probe to selected Resource URA, session id, target kind, capture
  scope, frame source id, and target geometry revision.
- Require selected sentinel id/hash evidence from the decoded-frame probe.
- Keep window/application unrelated-sentinel absence as a scoped capture proof.

## Expected impact

This still does not prove live capture without a real host artifact. It closes
the evidence seam where target-model or boolean-only evidence could be mistaken
for decoded frames from the selected target source.

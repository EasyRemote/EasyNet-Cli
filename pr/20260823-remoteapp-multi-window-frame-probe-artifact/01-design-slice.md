# RemoteApp multi-window decoded-frame probe artifact gate

## Product seam

The multi-window tracking verifier required distinct Resource URAs, session ids,
stream ids, frame source ids, media epochs, and sentinel leakage booleans. That
still allowed a weak artifact to prove only tracker metadata while not proving
that each independent stream rendered decoded frames from its own selected
window/application frame source.

## Slice

- Require every independent stream to include `rendered_frame_probe`.
- Bind each probe to selected Resource URA, session id, stream id,
  frame source id, media source epoch, and selected sentinel id.
- Require decoded-frame probe source, positive observed timestamp, selected
  sentinel hash, selected sentinel rendered, and no foreign sentinel rendering.
- Keep the existing distinct stream/session/source/epoch/sentinel checks.

## Expected impact

This does not replace a real live multi-window runner. It closes the verifier
seam where stream identity metadata could be mistaken for decoded per-window
rendering evidence.

# Python transport stream content-type alias removal

## Goal

Remove the Python runtime transport dict adapter's legacy `content_type` projection for stream events.

Canonical stream events expose payload media through `payload_content_type`. Keeping a second `content_type` key lets downstream products continue treating the stream event as a legacy wire shape and creates divergence from the Go SDK model.

## Boundary proof

- Typed `StreamEvent` parsing already rejects legacy `content_type` as a payload alias.
- Go stream event projection does not expose a parallel legacy `content_type` output.
- Python `RuntimeInvocationTransport.stream().recv()` is a facade projection over the typed event and should not add fields that are absent from the canonical event model.
- Bidi stream descriptors still use `content_type`; that is a distinct descriptor fact, not the stream event payload fact removed here.

## Invariants

1. Stream event dict projection includes `payload_content_type`.
2. Stream event dict projection does not include legacy `content_type`.
3. Python transport tests prove the dict adapter follows the canonical field.
4. SPEC v2 gate fails if `_stream_event_dict` reintroduces `content_type`.

## Verification plan

- Python transport focused test.
- SPEC v2 gate.
- SDK product-neutrality and public API gates.
- codegraph sync/status.

## Delta log

- Removed the Python transport stream event dict adapter's legacy `content_type` projection.
- Updated transport tests to require `payload_content_type` and reject `content_type` on stream event dicts.
- Added SPEC v2 structural and mutation coverage for the Python stream event projection.
- Verified focused Python transport tests, fmt, SPEC v2, SDK product-neutrality, architecture convergence, public API, and codegraph.

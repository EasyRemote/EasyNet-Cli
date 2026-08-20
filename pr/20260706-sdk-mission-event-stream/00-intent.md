# Intent

Goal: complete Mission profile live event stream facade coverage for Go and Python without moving Runtime Core stream semantics into Mission.

Non-goals:
- Do not change the daemon Mission protocol.
- Do not add a second stream state machine in Mission.
- Do not change the daemon SDK requirements spec.

Acceptance criteria:
- Go and Python expose Mission event stream adapters over Runtime Core stream handles.
- Shared Mission conformance declares and exercises `open_event_stream`.
- MEMC ownership maps assign the public methods to the Mission profile.
- Go/Python focused tests and SDK parity gates pass.

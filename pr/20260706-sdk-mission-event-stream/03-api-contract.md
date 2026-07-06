# API Contract

Go:
- `MissionEventStreamTransport.OpenEventStream(ctx, requestJSON) (*StreamHandle, error)`
- `MissionClient.OpenEventStream(ctx, MissionEventListRequest) (*MissionEventStream, error)`
- `MissionEventStream.Next/Cancel/Close/State/StreamID`

Python:
- `MissionEventStreamTransport.open_event_stream(request_json) -> StreamHandle`
- `MissionClient.open_event_stream(request) -> MissionEventStream`
- `MissionEventStream.next/cancel/close/state/stream_id`

Errors use existing SDK invalid-argument/profile transport classification.

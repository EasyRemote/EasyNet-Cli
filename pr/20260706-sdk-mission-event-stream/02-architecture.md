# Architecture

`MissionClient.open_event_stream` / `MissionClient.OpenEventStream` are profile-level adapters. They ask the Mission transport for a Runtime Core stream handle and wrap it as a typed Mission event stream.

The wrapper performs only Mission payload validation:
- frame must not carry an error payload,
- frame must carry `payload_json`,
- `payload_json` must decode to a valid MissionEvent.

The underlying stream state remains in `StreamHandle`.

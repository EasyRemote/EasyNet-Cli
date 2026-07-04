# Test plan

- Add a C ABI fake stream test proving `CompatibilityClient.stream_chat_completion` builds the carrier, opens Runtime Core stream, aggregates chunk payloads, projects through the C ABI projector, and closes the stream.
- Add a timeout test proving the profile stream path closes the C ABI stream before surfacing `TimeoutError`.
- Run focused Python C ABI/compatibility/environment tests.
- Run full Python SDK tests, C ABI scaffold check, formatting, whitespace check, SPEC drift check, and SDK conformance runner.

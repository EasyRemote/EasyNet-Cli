# ABI v9 leased streams

The existing `RuntimeClient.invoke_stream()` API remains an owned-event API. It
uses ABI v8 binary frames when available and copies payload bytes before the
native callback returns. ABI v9 is additive and is selected only by
`RuntimeClient.invoke_leased_stream()` or
`RuntimeClient.open_signed_leased_stream()`.

Each non-empty `LeasedStreamEvent.payload` owns one native lease reference.
Consumers must use one of these deterministic ownership paths:

- `payload.release()` (idempotent);
- `with payload:` or `with event:`;
- `payload.to_bytes()`, which copies and releases;
- `payload.write_into(destination)` or `payload.write_to(destination)`, which
  copy/write and release even when the operation raises.

`payload.retain()` creates a separately releasable reference. The SDK never
exposes the native pointer or a memory view over leased memory. Closing a stream
releases unread queued events and all still-live payload objects owned by that
stream. Queue overflow, malformed callback frames, and callback teardown also
release their lease references.

A finalizer emits `ResourceWarning` and attempts release as a diagnostic safety
net. It is not a correctness mechanism; production code must use an explicit
ownership path.

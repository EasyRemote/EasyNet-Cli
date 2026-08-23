# Invariants

1. ABI v7 remains byte-for-byte compatible and keeps its exact export list.
2. ABI v8 is additive and discoverable before a binding invokes its symbol.
3. Raw bytes are a transport representation only; Runtime Core remains the
   sole owner of sequence, lifecycle, admission, receipts, errors and terminal
   closure.
4. Metadata is canonical and complete for data, terminal, transport-error and
   EOF paths.
5. Payload pointers are borrowed only for callback duration and bindings copy
   before returning.
6. Callback queues, retained payloads and shutdown behavior remain bounded.
7. Python and Go prefer v8 only when both symbol and runtime feature are
   present; otherwise they use the existing v7 base64 representation.
8. Product readiness requires a real RemoteApp/EasyRemote consumer path and
   raw-payload evidence, not merely ABI unit tests.

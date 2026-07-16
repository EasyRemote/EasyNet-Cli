# Invariants

1. `InvokeStreamChunk` is a protocol transport detail; a public server-stream
   frame never exposes `kind: "chunk"`.
2. Every non-terminal public server-stream event is projected as `data`.
3. The C ABI projects `terminal` only after
   `InboundReceiptCheckpointVerifier` verifies the terminal receipt; an
   unproven transport terminal remains `data` with `terminal: false`.
4. A verified terminal receipt is the unique C ABI runtime terminal boundary
   and projects as `kind: "terminal"` with `terminal: true`.
5. A transport closure remains a distinct `error` with
   `transport_terminal: true`; it is not a runtime terminal event.
6. Go direct runtime, Python direct runtime, and the C ABI use the same
   public event vocabulary without aliases or language-specific adaptation.

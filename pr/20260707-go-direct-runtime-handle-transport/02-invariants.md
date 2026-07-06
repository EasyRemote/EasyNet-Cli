# Invariants

1. Direct runtime must not implement canonical signing material itself.
2. Prepare, submit, await, cancel, events, and free-handle operations must
   delegate through `RuntimeTransport`.
3. Handshake facts must not claim prepare/submit support unless the handle
   transport is configured.
4. Closing a direct runtime transport may close the delegated handle transport
   only when ownership is explicit.
5. Existing direct unary, stream, and bidi semantics must remain unchanged.

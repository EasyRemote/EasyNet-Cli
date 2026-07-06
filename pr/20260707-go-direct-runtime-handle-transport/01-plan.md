# Go Direct Runtime Handle Transport

## Objective

Close the Go SDK direct runtime gap where unary, stream, and bidi use the SDK
direct daemon transport but prepare/submit/handle operations require a
backend-local adapter.

## Boundary

- Axon still owns Invocation wire shape and canonical protocol semantics.
- EasyNet-Cli Rust/C ABI Runtime Core owns prepared signing material,
  signed submission handles, and handle lifecycle.
- Go SDK owns only the product facade and transport composition.
- Backend must eventually consume this as an SDK object rather than importing
  backend-local daemon gRPC/protobuf packages.

## Slice

1. Let `DirectDaemonRuntimeConnector` carry an SDK-owned handle transport.
2. Project prepare/submit capability in the direct runtime handshake from the
   configured handle transport.
3. Make direct transport lifecycle ownership explicit so composite transports
   can close without leaking C ABI/runtime handles.
4. Cover delegation and ownership with focused Go tests.

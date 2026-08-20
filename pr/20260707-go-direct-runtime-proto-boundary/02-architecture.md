# Architecture

The Go SDK root package contains DTOs, profile clients, runtime state machines,
and transport interfaces. A concrete direct daemon transport needs generated
Axon protobuf types to speak `axon.v1.Invocation` gRPC over the daemon endpoint.

Compiling that concrete transport into the root package by default makes every
SDK consumer initialize the SDK's private protobuf descriptors, even if the
consumer only uses DTO/profile code. Backend still has a legacy
`internal/pb/axon/v1` package; loading both packages registers the same
`axon/v1/types.proto` file twice and panics before tests can run.

The boundary is:

```text
default Go SDK facade
  -> no generated Axon protobuf initialization

Go SDK with easynet_direct_runtime tag
  -> DirectDaemonRuntimeConnector / DirectDaemonRuntimeTransport
  -> private sdk/go/internal/axonpb
  -> daemon axon.v1.Invocation gRPC
```

This is not a long-term excuse for backend protobuf ownership. It is the
correct compilation boundary for a concrete transport with global protobuf
registration side effects.

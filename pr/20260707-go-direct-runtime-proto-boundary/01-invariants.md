# Invariants

1. Axon owns protobuf wire shape and Invocation gRPC semantics.
2. Go SDK may contain generated Axon protobuf adapters only as private concrete
   transport implementation detail.
3. Root SDK facade imports must not force every product process to load private
   Axon protobuf descriptors.
4. Backend remains required to delete its private generated Axon protobuf before
   the final SDK-only boundary can pass.
5. Build tags must not create an alternate semantic implementation; they may
   only control whether the concrete direct transport adapter is compiled.

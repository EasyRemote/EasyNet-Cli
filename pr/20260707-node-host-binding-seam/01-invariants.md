# Invariants

1. DescriptorRef validation/canonicalization is delegated through an injected
   canonicalizer; Node does not parse or own DescriptorRef grammar.
2. Host-stream frame and output-hash behavior is Host Binding SDK profile
   behavior, not Axon Invocation canonicalization.
3. Lifecycle providers are explicit product-host delegates. The Node facade
   records state transitions and validation, but does not start or supervise
   host processes.
4. Cleanup is idempotent after success and close requires cleanup first.
5. The Node action-adapter report must stay closed over every Node-declared
   shared conformance case.

# Architecture

The Rust daemon invocation client exposes two views:

- `InvocationOutcome`: canonical aggregate with terminal result plus receipt stages.
- `InvocationResult`: terminal projection for callers that do not need the checkpoint chain.

The result projection remains public for compatibility, but its architectural meaning is canonical terminal projection, not legacy DTO compatibility.

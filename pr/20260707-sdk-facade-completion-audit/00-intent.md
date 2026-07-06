# SDK Facade Completion Audit

## Objective

Continue converging `docs/spec/daemon-sdk-requirements-v1.md` without editing the
spec. The implementation target is the daemon SDK facade architecture:

```text
Axon protocol truth -> EasyNet-Cli daemon/Rust/C ABI -> language SDK facades
```

Language SDKs may provide ergonomic objects and typed DTOs, but they must not
own URA, DescriptorRef, Invocation canonicalization, receipt verification,
stream, or bidi semantics.

## This Slice

This audit focuses on remaining facade-boundary risks after Runtime Core,
typed errors, Python/Go live daemon smokes, and backend/EasyRemote boundary
checks are green:

1. direct Axon imports in language SDKs are isolated to explicit bridge files;
2. bridge files are named by semantic ownership, not legacy compatibility;
3. shipped error codes remain the current canonical schema;
4. remaining incomplete profiles are tracked as product-scope work, not hidden
   inside Runtime Core.

## Non-goals

This slice does not relax profile requirements and does not change the
normative spec. It also does not introduce fallback behavior for historical ABI
or error-code aliases.

# Go Direct Runtime Proto Boundary Intent

## Objective

Keep the public Go Daemon SDK root facade importable by product consumers that
have not yet deleted legacy generated Axon protobuf packages, while preserving
the SDK-owned direct daemon Invocation transport for explicit direct-runtime
builds.

The target remains:

```text
Backend / host app -> EasyNet-Cli Go SDK facade -> easynet-daemon -> Axon
```

## Non-Goals

- Do not change `daemon-sdk-requirements-v1.md`.
- Do not move Axon protocol semantics into backend or Go facade code.
- Do not make backend's generated Axon protobuf package acceptable long term.
- Do not hide process or transport lifecycle behind product handlers.

## Acceptance Criteria

1. Default `easynet.run/cli/sdk/go` import does not register SDK internal Axon
   protobuf descriptors.
2. Direct daemon Invocation gRPC transport remains available behind an explicit
   SDK build surface.
3. Existing Go SDK default tests continue to pass.
4. Direct runtime tests continue to pass when the explicit direct runtime build
   tag is enabled.
5. Backend `internal/svc` SDK tests no longer panic at process init from double
   `axon/v1/types.proto` registration.

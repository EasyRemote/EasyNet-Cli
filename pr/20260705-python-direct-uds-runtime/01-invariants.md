# Invariants

- Public SDK interfaces stay JSON/dataclass oriented.
- Generated Axon protobuf code remains private under `easynet_sdk._axon_pb`.
- The generated protobuf modules mirror EasyNet-Axon's `axon/v1/types.proto`
  and `axon/v1/invoke.proto`; they are wire bindings, not protocol authority.
- The direct runtime transport accepts the same `InvocationDraft` JSON consumed
  by the existing `RuntimeClient`.
- Unary invocation maps to daemon `axon.v1.Invocation/Invoke` over UDS.
- Transport failures are normalized to `SDKError` with stable `ErrorCode`
  values.
- Unsupported modes fail explicitly with `NOT_IMPLEMENTED`; no silent fallback
  to an unrelated runtime path.
- Tests must use a fake gRPC daemon endpoint and verify the actual wire request
  projection.

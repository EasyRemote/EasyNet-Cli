# Canonical Runtime Convergence V2 - Initial Evidence

- `sdk/rust/src/invocation/axiom.rs` retains plain
  `canonical_invocation_bytes` signing/verification helpers beside the
  descriptor-bound path.
- `sdk/rust/src/invocation/admission.rs` still publicly exposes
  `verify_signature` and `run_admission` next to descriptor-bound admission.
- `sdk/python/easynet_axon/invocation/admission.py` has the equivalent plain
  public admission functions.
- `core/proto/axon/v1/mission.proto` and `types.proto::MissionState` keep
  product orchestration state in Axon core.
- `sdk/java/.../Axiom.java::ReceiptBody` compatibility constructors create
  empty proof facts.
- `core/runtime-rs/client-sdk/src/domain/easynet/semantic.rs` contains a
  process-local default signer fallback.
- `src/daemon/invocation/dispatch/local_runtime_invoker.rs` still derives
  daemon-system subject defaults for explicitly classified system calls.
  `src/daemon/invocation/routing/target.rs` now separates resolver plans into
  daemon-system and public-ingress tuple sources, but direct
  `InvocationTarget` construction sites still need migration before RF-8 can
  close.
- `src/support/platform/local_daemon_grpc.rs` remains an adapter boundary to
  retire from direct envelope construction in favour of an Axon-owned builder.

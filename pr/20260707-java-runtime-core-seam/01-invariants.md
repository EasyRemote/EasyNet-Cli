# Invariants

- Java exposes SDK-owned DTOs and interfaces only; it must not import Axon,
  protobuf, daemon internals, or product facades.
- Feature discovery reports generic SDK profiles and symbols only; it must not
  expose Axon protobuf/provider flags in the public Java seam.
- Invocation construction preserves caller, callee, descriptor, subject, nonce,
  causal context, and args before dispatch.
- RuntimeClient owns transport connection lifecycle only; closing it never
  implies daemon process ownership.
- Stream and bidi retained histories are bounded and terminal overflow is typed.
- Java remains `seam`; no provider-backed, package-stable, or cutover-ready
  claim is made in this slice.

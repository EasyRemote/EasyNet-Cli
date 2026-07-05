# Python Receipt History Facade Plan

## Objective

Expose daemon-owned receipt history and trace read models through the Python
Receipt profile so EasyRemote-style callers do not need raw daemon transport or
Axon protocol imports for invocation ledger reads.

## Boundary Proof

- Rust daemon SDK contract owns the fixed daemon ability names and complete
  Invocation carrier construction.
- C ABI projects those Rust contract functions to language bindings.
- Python owns only DTO ergonomics, typed result decoding, and transport
  delegation.
- Python must not synthesize DescriptorRefs, claim cryptographic receipt
  verification, or parse Axon receipt internals for this slice.

## Invariants

- Every history request carries caller, callee, subject, descriptor version,
  nonce, causal context, and bounded arguments explicitly.
- `timeout_ms` is carrier metadata, not an Invocation tuple parameter.
- Local receipt projection transport remains projection-only and fails closed
  for daemon history reads.
- C ABI-backed history calls execute through Runtime Core invoke and return the
  daemon read-model JSON unchanged to the Python facade.

## Verification

- Python receipt tests cover DTO validation, build invocation, list/get/trace
  facade methods, and local fail-closed behavior.
- Python C ABI tests cover symbol delegation for build and live read paths.
- Ruff and scaffold checks remain clean.

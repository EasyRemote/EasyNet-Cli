# Node Conformance Report

## Objective

Bring the existing Node/TypeScript P1 seam into the shared SDK conformance
evidence chain without overstating its capability level.

The target architecture remains:

```text
Axon protocol truth -> EasyNet-Cli daemon/Rust/C ABI -> language SDK facades
```

Node currently exposes Runtime Core plus Directory + Identity seam objects over
injected transports, and this slice adds a Receipt seam for fetch/projection/
causal-ref carriers without claiming Axon-backed verification. The shared
conformance report records the MEMC cases plus the two Receipt cases that the
Node seam now proves.

## Scope

1. Add Node Receipt facade objects over injected transports:
   `ReceiptClient`, `ReceiptRef`, and `ReceiptChain`.
2. Add tests for receipt fetch carriers, projection/causal refs, malformed
   receipt anchors, history carriers, and verification delegation.
3. Add a Node action-adapter report for shared MEMC plus Node-declared Receipt
   cases.
4. Make scaffold validation require and JSON-validate the Node report.
5. Document the Node runner command beside Rust, C ABI, Go, and Python.
6. Close existing conformance runner gaps by validating internal schema `$ref`
   targets, keeping authority schema definitions single-sourced, and aligning
   feature-discovery schema with the authority profile exposed by C ABI feature
   discovery.
7. Do not claim Node Axon-backed chain verification, provider-backed daemon
   transport, or product cutover.
8. Do not edit the normative daemon SDK requirements spec.

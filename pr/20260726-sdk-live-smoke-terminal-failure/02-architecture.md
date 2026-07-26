# Architecture

## Boundary

The daemon invocation service owns Axon admission and local route execution. The C ABI owns projection into the language-neutral Runtime SDK wire contract. Language SDKs should not infer daemon-specific failure semantics.

## Target Shape

```text
daemon admission failure
  -> typed daemon invocation failure
  -> C ABI Runtime InvokeResult JSON
  -> Go/Python RuntimeClient typed SDK error or failed InvocationResult
```

The old failure is an ambiguous receipt-free payload that is neither a receipt-bearing terminal result nor an explicitly admitted pre-admission failure. That ambiguity belongs at the projection boundary, not in downstream language SDKs.

## Cutover Shape

- `DaemonRuntimeAdmissionCoordinator` converts admission `Status` values into canonical Axon error facts before C ABI projection.
- Device-mode boot writes the paired Device→User owner fact through `RuntimeTrustContext`, so ordinary policy evaluation still reads only the trust-anchor model.
- Stream readers reaching terminal mark the FFI stream resource as terminal-drained; `Close` remains the single owner-authorized release path and is valid after terminal drain.

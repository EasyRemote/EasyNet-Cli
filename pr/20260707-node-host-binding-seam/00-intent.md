# Node Host Binding Seam

## Objective

Extend the Node/TypeScript P1 facade with Host Binding seam coverage while
preserving the daemon SDK boundary:

```text
Axon protocol truth -> EasyNet-Cli daemon/Rust/C ABI -> Node facade
```

Node must expose host-stream binding DTOs, frame codec helpers, output-hash
folding, and readiness/cleanup lifecycle state as facade behavior. It must not
claim daemon provider-backed status, start host processes, or own Axon protocol
semantics.

## Scope

1. Add HostBindingClient and LocalHostBindingTransport facade objects.
2. Add TypeScript declarations for host-stream binding, lifecycle provider, and
   hash-state DTOs.
3. Cover binding creation, request decoding, item/error/terminal frames,
   deterministic output-hash folding, corrupted hash-state rejection, and
   explicit readiness/cleanup lifecycle behavior.
4. Declare Node for the shared `host_binding/codec_hash` case and add the Node
   action-adapter report record.
5. Update Node status docs and parity summary without changing the normative
   daemon SDK requirements spec.

## Non-goals

- No daemon transport provider.
- No host process execution.
- No plugin policy or host binding bridge claim.
- No Axon DescriptorRef grammar implementation inside Node.

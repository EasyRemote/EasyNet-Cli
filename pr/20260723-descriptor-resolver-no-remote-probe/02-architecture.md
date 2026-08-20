# Architecture

## Boundary

The FFI descriptor resolver is an SDK/provider seam. It returns a
descriptor-bound reference for a requested `(callee_ura, ability, call_mode)`
selector.

It is not a route resolver and not an invocation issuer.

## Ownership

- Axon/runtime descriptor catalog owns canonical descriptor identity.
- EasyNet daemon owns product route locality and online presence.
- SDK facades construct complete invocation drafts using descriptor refs
  returned by the provider.

## Removed Legacy Path

The previous fallback path performed:

```text
catalog miss
  -> build remote meta.list_abilities invocation
  -> load caller signer
  -> invoke remote target
  -> reinterpret result as descriptor catalog
```

That path crossed from descriptor lookup into invocation dispatch and produced
misleading signer/owner/timeout failures for what should be a catalog miss.

## Clean Path

```text
descriptor request
  -> parse callee/ability/call_mode
  -> local runtime-owner catalog lookup when callee is local owner
  -> local provider-backed realm catalog lookup when callee is remote
  -> descriptor ref or typed catalog miss
```

# Architecture

## Boundary

The FFI descriptor resolver belongs to the SDK/runtime catalog boundary. It
should project known descriptor facts; it should not perform product or runtime
ability invocation to discover facts.

## Removed Legacy Surface

The removed path was:

```text
runtime_resolve_descriptor_ref_json
  -> runtime_descriptor_catalog_entries
  -> runtime_meta_descriptor_catalog_entries
  -> DaemonInvocation(meta.list_abilities)
  -> daemon invoke
```

That path made descriptor lookup depend on signer custody, online owner state,
route resolution, and invocation timeout behavior.

## Clean Target

The resolver now follows:

```text
runtime_resolve_descriptor_ref_json
  -> runtime_descriptor_catalog_entries
  -> runtime_system_descriptor_catalog_entries
```

Future provider-backed catalogs must be added as an explicit provider object
with its own capability matrix and failure semantics.

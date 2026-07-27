# Decisions Log

## Self-target catalogue reads

Decision: treat `--node <local device URA>` as an explicit `LocalRuntime` catalogue read state instead of sending it through canonical remote invocation.

Reason: a local target is not a remote authority problem. Remote invocation requires caller signer and owner authority facts; using it for the local catalogue recreates a second route for the same runtime owner and surfaces product-facing `AUTHORITY_DENIED`/descriptor errors even though the local daemon owns the descriptor catalogue.

Boundary: peer device catalogue reads still use canonical remote invocation. No product-directory fallback or node-id repair was introduced.

## FFI descriptor source naming

Decision: keep local runtime-owner descriptor resolution sourced from `runtime_local_descriptor_catalog`, and update the stale test expectation that still called it `runtime_receipt_provider`.

Reason: descriptor catalogue ownership and receipt history ownership are separate provider states. Calling local catalogue rows receipt-provider rows collapses the owner boundary and weakens SPEC conformance.

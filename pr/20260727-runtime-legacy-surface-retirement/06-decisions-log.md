# Decisions Log

## Self-target catalogue reads

Decision: treat `--node <local device URA>` as an explicit `LocalRuntime` catalogue read state instead of sending it through canonical remote invocation.

Reason: a local target is not a remote authority problem. Remote invocation requires caller signer and owner authority facts; using it for the local catalogue recreates a second route for the same runtime owner and surfaces product-facing `AUTHORITY_DENIED`/descriptor errors even though the local daemon owns the descriptor catalogue.

Boundary: peer device catalogue reads still use canonical remote invocation. No product-directory fallback or node-id repair was introduced.

## FFI descriptor source naming

Decision: keep local runtime-owner descriptor resolution sourced from `runtime_local_descriptor_catalog`, and update the stale test expectation that still called it `runtime_receipt_provider`.

Reason: descriptor catalogue ownership and receipt history ownership are separate provider states. Calling local catalogue rows receipt-provider rows collapses the owner boundary and weakens SPEC conformance.

## Rejected Device-owned local read issuer

Decision: do not commit the `LocalRuntimeDeviceReadIssuer` prototype.

Reason: the current SPEC v2 gate explicitly treats `discover.rs`, `doctor.rs`, `groups/device.rs`, `status.rs`, `invocation_watch.rs`, the agent state gateway, and `llm-api` model catalogue discovery as runtime-state read paths that must enter through `LocalRuntimeStateReadIssuer`. The prototype would have changed that contract rather than converging to it.

Boundary: this does not bless subject-owner conflation elsewhere. It only records that this repository's current authoritative gate defines those CLI paths as user runtime-state reads. Future changes must first update the SPEC/gate contract, not silently diverge from it.

## Node receipt-history governance subject parity

Decision: align Node SDK receipt-history admission with the Go/Python canonical runtime model by accepting two explicit subject states:

- user-owned runtime-state read subject;
- exact callee runtime-owner subject for Device/Authority governance reads.

Reason: product history views sometimes need a device-owned ledger query. Go and Python already admit this when the subject equals the callee runtime owner and authority is bound to that exact tuple. Node only admitted user runtime-state subjects, forcing product code toward placeholder user sessions or divergent provider behavior.

Boundary: this is not a fallback. Non-callee runtime-owner subjects still fail before provider dispatch, all-zero principals remain rejected, and session-authority subject rules remain strict. Device-owned history requires authority that actually admits the device subject, e.g. exact delegation authority.

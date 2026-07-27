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

## Swift receipt canonicalizer fail-closed parity

Decision: make Swift `RuntimeReceipt.canonicalReceiptType` throw on unknown canonical lifecycle states instead of returning an empty string.

Reason: receipt validation is proof-fact validation, not presentation formatting. Returning an empty string leaves a permissive internal helper that can be reused incorrectly even though the current constructor first canonicalizes `state`. Go, Java, Python, and Node either operate on already validated lifecycle state or fail explicitly; Swift should not preserve a silent empty receipt-type sentinel.

Boundary: public receipt behavior remains compatible for valid receipts. Invalid canonical lifecycle states now fail with an explicit `INVALID_ARGUMENT` validation error before any proof-fact path can treat an empty receipt type as data.

## Java receipt canonicalizer fail-closed parity

Decision: make Java `RuntimeReceipt.canonicalReceiptType` throw on unknown canonical lifecycle states instead of returning an empty string.

Reason: Java had the same internal fail-open sentinel as Swift. Even if current construction validates `state` before binding `receipt_type`, proof-fact validation helpers should not encode unknown lifecycle states as data. Receipt type derivation must be total only over known lifecycle states and explicit-failing otherwise.

Boundary: no new Java public API was introduced. The regression test reaches the private helper by reflection only to lock the internal invariant; valid receipt behavior and public interfaces remain unchanged.

## Device directory user-binding state machine

Decision: model `device list` directory reads as three explicit states:

- bound user credentials: read `federation.discover` through the user-scoped directory path;
- unbound federation-native credentials: fail closed at the CLI boundary because no user-scoped directory principal exists;
- local Authority daemon: read the operator/audit directory path.

Reason: a clean Hub-URA join can intentionally produce a federation-native device credential without a user binding. Treating that as a missing legacy `user_id` sent the product path into an unauthorized operator/audit invocation from a Device daemon and surfaced daemon-internal `AUTHORITY_DENIED`/`LOCAL_BOOTSTRAP_OWNER_UNAVAILABLE`. The runtime state itself is valid; the unsupported capability is the user-scoped product directory read.

Boundary: this does not add a compatibility fallback, does not synthesize a user id, and does not allow a Device daemon to use the Authority operator/audit directory. A user-facing product device directory still requires either a bound User principal or an Authority daemon.

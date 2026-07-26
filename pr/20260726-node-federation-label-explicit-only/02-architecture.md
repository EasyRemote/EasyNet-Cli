# Architecture

`src/support/platform/node.rs` owns shared node read-model projection for CLI
surfaces. It must remain a small semantic boundary:

- state projection: canonical string states only;
- federation display projection: explicit `axon.federation.runtime_label` only.

`axon.federation.runtime_id` may remain part of the raw label map for routing or
audit purposes, but this module will not reinterpret it as a product display
label.

The existing shared-node-state SPEC v2 gate is extended because this is the same
projection ownership boundary.

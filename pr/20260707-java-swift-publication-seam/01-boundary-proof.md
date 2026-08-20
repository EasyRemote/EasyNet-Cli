# Boundary Proof

## Layer Ownership

The Publication profile is a daemon SDK profile seam. Java and Swift own typed
DTOs, client lifecycle, error mapping, and transport delegation. The daemon owns
resource-ref construction, package validation, deploy/unpublish carrier
construction, publication policy, and executable binding state.

## Runtime Boundary

`PublicationClient` methods delegate every daemon-authored fact to injected
`PublicationTransport` instances. The facades validate request/projection shape
and do not inspect package directories, hash manifests, publish plugins, or
derive descriptor refs from product naming rules.

## Invocation Boundary

Deploy/unpublish request DTOs preserve complete runtime identity fields:

- `caller_ura`
- `callee_ura`
- `subject_ura`
- `descriptor_version`
- `nonce_base64`
- `causal_context`

The request-specific fields are descriptor-owned args, not hidden tuple
defaults.

## Product Boundary

The seam does not expose EasyRemote decorators, Python function introspection,
warm host process lifecycle, plugin marketplace policy, or product catalog
state. Those remain downstream product concerns built on top of the generic
daemon runtime model.

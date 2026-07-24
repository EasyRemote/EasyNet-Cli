# Architecture

The canonical SDK root owns the shared runtime model. Product providers live
below provider namespaces and are consumers/adapters of that model.

Keeping `@easynet/daemon-sdk` as the Node root package name fuses the SDK root
with one product/daemon deployment. The Java root already uses
`run.runtime:canonical-runtime-sdk`, and Swift uses `RuntimeSDK`. The Node root
should follow the same neutral package identity.

The migration does not alter JavaScript exports, TypeScript declarations, or
provider subpaths. It only changes the private package identity and the metadata
gate.

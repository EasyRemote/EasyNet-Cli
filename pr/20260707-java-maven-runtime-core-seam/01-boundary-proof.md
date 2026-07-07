# Java Maven Runtime Core Seam Boundary Proof

Maven metadata is a packaging boundary, not a provider boundary. The Java package remains dependency-free and exposes only generic Runtime Core concepts: feature discovery, typed errors, complete Invocation draft construction, injected runtime dispatch, and bounded stream/bidi lifecycle state.

The package does not add daemon process ownership, JNI bindings, C ABI loading, generated protocol classes, product profile clients, or product cutover behavior. The seam guard builds the Maven jar and then runs the direct Runtime Core seam test so packaging and behavior are both checked.

The Maven artifact is intentionally versioned as `0.0.0-seam` to prevent a stable release claim.

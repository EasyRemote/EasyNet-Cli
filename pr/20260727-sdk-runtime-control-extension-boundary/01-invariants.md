Canonical runtime SDK invariants:
- Runtime lifecycle state includes control endpoint, Invocation endpoint, process/version facts, IPC version facts, capability flags, and runtime-host identity.
- Product HTTP surfaces such as Pages are provider extension metadata, not canonical runtime lifecycle state.
- Provider extension fields may be tolerated for wire interoperability only when they are not interpreted as SDK state.

Boundary invariants:
- Go and Python SDKs must converge on the same field ownership.
- Hand-written SDK APIs must not expose EasyNet product directory or Pages lifecycle fields.
- Current daemon wire compatibility is preserved by accepting known provider extension fields without projecting them.

Failure invariants:
- Unknown control discovery fields still fail closed.
- Required canonical discovery fields remain required.
- Malformed provider extension metadata must not block canonical runtime attach because the SDK does not own its semantics.

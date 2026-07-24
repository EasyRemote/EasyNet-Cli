# Boundary proof

## Root abstraction problem

The provider sidecar helper packages decoded daemon-admitted invocation frames
and silently repaired missing `args` or `causal_context` to empty objects. That
created a second tuple construction authority at the language-helper boundary.

Canonical tuple completeness belongs to the daemon/runtime invocation pipeline.
Provider helpers may project an already-admitted frame into ergonomic language
objects, but they must not synthesize missing tuple fields.

## Owning boundary

The owning boundary is the provider helper ingress in:

- `sdk/go/provider/easynet/pluginexec`
- `sdk/python/easynet_sdk/providers/easynet/plugin_exec.py`
- `sdk/rust/provider/easynet/pluginexec`
- `sdk/java/.../provider/easynet/pluginexec`
- `sdk/node/provider/easynet/pluginexec.js`

These packages are provider-scoped consumers of the canonical runtime model.
They are not part of the canonical SDK root and must not introduce
EasyNet-specific tuple semantics into the SDK root.

## Canonical invariant

- `caller_ura`, `callee_ura`, `ability_ura`, `subject_ura`,
  `invocation_nonce`, `causal_context`, and `args` are required frame fields.
- Unknown tuple aliases remain rejected.
- Unknown frame fields remain rejected.
- Missing required frame fields reject before invoking the plugin handler.
- Helper behavior remains aligned across Go, Python, Rust, Java, and Node.

## Compatibility removed

The removed compatibility layer is helper-side defaulting:

- missing `args` -> `{}`
- missing `causal_context` -> `{}`
- null `args`/`causal_context` accepted as empty object

Those defaults are invalid because they mask incomplete runtime tuples.

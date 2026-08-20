# Prepared Tuple Latest-Only Plan

Goal: remove the prepared-invocation tuple compatibility normalizer and enforce the latest Runtime Core DTO shape in both SDKs.

## Scope

- Decode `PreparedInvocation.tuple` directly as the canonical `InvocationDraft`.
- Reject signing-material fields when they appear inside `tuple`.
- Keep signing material owned by `signing_material`.
- Preserve the public prepared/sign/sign-submit interfaces.

## Non-goals

- No legacy prepared tuple aliases.
- No product-specific prepared invocation model.
- No language-specific divergence between Go and Python.

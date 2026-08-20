# Architecture

Introduce one Java SDK `InvocationAuthorityBindingValidator` object.

Ownership:

- `InvocationBuilder` owns draft construction only.
- `AuthoritySupport` owns shared metadata decoding and authority helper predicates.
- `InvocationAuthorityBindingValidator` owns tuple-bound authority semantics.

This avoids procedural accumulation in the builder and mirrors the existing cohesive Node validator.


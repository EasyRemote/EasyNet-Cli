# Architecture

## Ownership

- Axon/runtime model owns the canonical invocation tuple, descriptor-bound admission, lifecycle states, and receipt proof facts.
- EasyNet-Cli owns daemon/product policy and may use canonical runtime primitives internally.
- Product compatibility protocols are allowed only outside the canonical SDK/runtime abstraction.

## Boundary Proof

This slice must prove that any removed code was not the canonical owner. The replacement must already exist in the canonical path, or the old path must be an obsolete public edge that now fails closed.

## Discovery Method

Use `codegraph` for semantic navigation, then confirm with `rg`, tests, and gates. Searches prioritize active runtime/SDK source over documentation and negative tests.

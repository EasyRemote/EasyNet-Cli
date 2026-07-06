# Architecture

## Root Abstraction Problem

The Node SDK root was a README placeholder, so the architecture had no public
object seam for a P1 language. That left the canonical runtime model expressed
only in Go/Python language facades.

## Target Architecture

- `sdk/node/index.js` is the product-neutral Runtime Core facade.
- `sdk/node/index.d.ts` is the public TypeScript contract.
- `sdk/node/test/runtime-core.test.mjs` is the executable seam proof.
- Transports are structural objects supplied by integrations.

## Capability State

This slice moves Node Runtime Core from `unsupported` to `seam`. It does not
claim provider-backed or cutover-ready status.

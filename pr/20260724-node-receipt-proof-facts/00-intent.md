# Intent

## Goal

Converge the Node SDK runtime receipt validator with the canonical receipt proof-facts model already enforced by Go, Python, and Java.

## Non-goals

- Do not add product-specific receipt concepts.
- Do not preserve a field-shape-only compatibility path.
- Do not change public Node SDK command or class names unless required by the canonical model.

## Acceptance criteria

- Node receipt validation rejects mismatched authority proof hashes.
- Node receipt validation accepts binding-projection proof hashes when proof payload is empty.
- Node receipt validation enforces signer/issuer proof-fact topology.
- The v2 convergence gate recognizes and verifies the Node proof-facts boundary.

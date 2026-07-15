Python SDK static contract gate

## Objective

Close the A57 enforcement gap by turning the existing Python SDK static contract
into an executable SDK gate. The contract must prove that the public Python SDK
model can be imported and type-checked under a strict checker, and Ruff must run
over the SDK sources that define the public facade and tests.

## Expected effect

| Dimension | Expected convergence |
|---|---|
| Architecture convergence | Python SDK exported runtime models gain a repeatable static contract instead of relying only on behavior tests. |
| Architecture cleanliness | The static contract becomes a named gate in cutover readiness, with one owner and one script. |
| Product acceleration | Future SDK changes fail early on unresolved symbols, broken exports and model type drift before live smoke runs. |
| Risk | This does not broaden public API or change runtime behavior; it only makes existing evidence executable. |

## Non-goals

- Do not introduce a new Python type system or generated SDK model.
- Do not rewrite Python exports in this slice.
- Do not run type checking over external downstream product repositories.

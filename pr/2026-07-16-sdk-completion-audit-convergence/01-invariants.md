# Invariants

## Capability state machine

The SDK parity matrix has one state machine for every capability/language cell:

1. The only legal cell states are `unsupported`, `seam`, `provider-backed`, and
   `cutover-ready`.
2. `unsupported` cells must carry no execution evidence, shape evidence, step
   shape evidence, or provider proof reference.
3. `seam` cells must carry at least one real seam signal:
   - public shape evidence plus step shape evidence, or
   - execution evidence for that language/capability.
4. `seam` cells must not carry provider proof references.
5. `provider-backed` and `cutover-ready` cells must carry execution evidence,
   public shape evidence, step shape evidence, and a provider proof reference.

## Gate topology

`check-sdk-completion-audit.sh` owns completion semantics. It must expose a
matrix-only mode that can be called by cutover readiness without recursively
calling cutover readiness again.

`check-sdk-cutover-readiness.sh` must include the matrix-only completion audit so
SDK cutover readiness cannot pass with an invalid generated completion matrix.

## Boundary

This change is a gate convergence slice. It does not change public SDK behavior
or SDK runtime APIs.

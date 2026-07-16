# Decisions Log

## 2026-07-16

- Selected EAL agent dispatch because it is an execution path still reading the durable Agent registry outside the aggregate owner.
- Preserved the existing fail-open-to-empty-registry behavior because it is an intentional EAL first-run/degraded-mode contract, while changing only the source owner.
- Kept `dispatch_to_agent` registry-shaped input for this slice. The root fork is dispatcher construction ownership, not pure dispatch helper shape.
- Restored the convergence script's Rule 41 indentation while adding Rule 40, because the self-test must execute all active architecture gates.

# Boundary Proof

## Storage boundary

The storage boundary reads bytes and reports `Loaded` or `Missing`. It owns
strict JSON parsing and I/O errors.

## Runtime identity projection boundary

First boot can project missing hosted-agent identity storage into an empty
projection so bootstrap can mint stable URAs and persist them. Existing corrupt
files still block lifecycle operations.

## Public read API boundary

The public `load() -> LocalAgentsFile` shape remains stable as a read
projection. Internal production paths use the explicitly named projection
helper so the policy boundary is visible in code review.

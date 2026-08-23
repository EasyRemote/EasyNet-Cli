# Architecture

Layering:

1. Cross-platform capture verifier validates native/backend evidence.
2. Its report emits a compact per-platform/per-target summary.
3. Product-completion gate validates summary sufficiency across the matrix.

This keeps the aggregate gate from becoming a second capture verifier while preventing weak platform-only reports from being treated as product evidence.

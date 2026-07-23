# Intent

Centralize explicit-authority catalog test construction.

The previous convergence slices correctly removed ambient catalog fixtures from
metadata and invocation-history tests, but each module now owns a small copy of
the same Device-authority setup. That duplication is a new drift surface. The
catalog type should own canonical test fixture construction because authority
context is part of catalog state.

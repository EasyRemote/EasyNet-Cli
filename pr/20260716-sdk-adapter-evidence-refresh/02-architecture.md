# Architecture

SDK conformance reports are proof metadata over the adapter test suites and
their source references. The adapter reports do not own runtime behavior; they
attest that the current implementation files back each conformance case.

The repository-owned refresh script recalculates `sha256` values from the
referenced source files and rejects invalid report structure or escaping paths.
Using that script preserves a single evidence-refresh mechanism instead of
hand-maintained report edits.

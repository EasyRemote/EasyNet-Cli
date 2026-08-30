# Invariants

1. Axon owns protocol, ABI, and canonical SDK facts; CLI only locks and verifies them.
2. A lock names an immutable 40-hex Git revision and the SHA-256 of the contract at that revision.
3. `axon_release_version`, descriptor digest, ABI version, and SDK versions must equal the checked-out Axon contract exactly.
4. Cargo and Python dependency resolution must agree with the lock; a live sibling alone is not proof of a releasable artifact.
5. Pinned verification is read-only and fail-closed for dirty, wrong-revision, missing, malformed, or drifted Axon inputs.
6. Candidate verification may select another exact revision, but it cannot mutate or silently replace the pinned lock.
7. Artifact verification disables path sources and proves built packages resolve from publishable inputs.
8. CLI admission remains red until the full required suite passes; the compatibility manifest cannot override failing semantic tests.

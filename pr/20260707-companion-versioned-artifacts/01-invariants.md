# Invariants

- Installed companion artifacts are owned by `(package_id, package_version)`.
- Removing one package version must not delete supervisor state for another package version.
- Supervisor target checks must be exact enough to avoid false ownership.
- Missing launcher entries remain idempotent non-errors during remove.

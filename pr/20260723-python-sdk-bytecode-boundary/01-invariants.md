# Invariants

- Canonical SDK source trees contain source, manifests, tests, and fixtures only.
- Python `__pycache__` directories and `*.pyc` files are local build artifacts.
- Conformance gates must detect tracked SDK bytecode, not only untracked files.
- Removing bytecode must not change public SDK API behavior.

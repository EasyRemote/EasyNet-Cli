Static contract invariants

1. The Python SDK must keep explicit public exports that the public inventory
   can resolve.
2. Static checking must import SDK sources through the same local Python path as
   conformance gates.
3. Ruff must cover SDK facade sources, SDK tests and the strict contract file.
4. Mypy strict checking must cover `python_sdk_type_contract.py`, because it
   encodes public runtime model expectations with `typing.assert_type`.
5. The gate must be callable independently and from cutover readiness.
6. Self-test mode must avoid expensive repository-wide execution while proving
   the script and contract are syntactically valid.

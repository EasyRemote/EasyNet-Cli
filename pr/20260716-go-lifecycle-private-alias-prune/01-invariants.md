Invariants
==========

1. Exported `Daemon*` aliases stay intact until a SPEC cutover removes them.
2. Private lifecycle helpers use canonical `Runtime*` names only.
3. Runtime handle construction, readiness validation, state validation and
   transport error wrapping each have one internal implementation.
4. No fallback helper exists only to preserve the old internal daemon naming.
5. Public API inventory must not change for this deletion slice.

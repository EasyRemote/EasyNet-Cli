Invariants
==========

1. All-zero principal placeholders are never valid runtime identities.
2. The SDK core tuple builder owns tuple completeness and identity sentinel
   rejection; providers may add policy, but must not be the first guard.
3. Go, Python, Java, and Swift must converge on the same fail-closed behavior
   for caller/callee/subject sentinel values.
4. This is not URA parsing: canonical URA shape and descriptor resolution still
   belong to addressing/provider/runtime boundaries.

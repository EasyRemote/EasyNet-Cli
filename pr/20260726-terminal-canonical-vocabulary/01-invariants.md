# Invariants

1. The canonical runtime ability namespace is `terminal.*`.
2. PTY remains an implementation detail below the ability boundary.
3. Public wire ability values remain stable.
4. Internal names must not imply a second `pty_session.*` public API.
5. Tests should assert dispatchability through canonical terminal constants.

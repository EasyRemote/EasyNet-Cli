# Decisions Log

- The canonical Python helper remains in `_session_authority_subjects.py`.
- The private authorized-session wrapper is removed because it is not a
  semantic boundary; it is a compatibility-shaped indirection.
- Both session-history authorization and runtime-call authorization now call
  the shared helper directly.
- The v2 gate and the legacy architecture gate now reject reintroduced private
  wrappers in `authorized_runtime_session.py`.

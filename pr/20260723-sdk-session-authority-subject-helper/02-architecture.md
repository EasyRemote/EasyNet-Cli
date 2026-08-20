# Architecture

Before:

- `_session_authority_subjects.py` owned the structured owner rule.
- `authorized_runtime_session.py` preserved a private
  `_session_authority_admits_subject` shim that immediately delegated to the
  canonical helper.

After:

- `_session_authority_subjects.py` is the single Python SDK owner.
- `authorized_runtime_session.py` imports and calls
  `session_authority_admits_subject` directly in both history and runtime
  authorization paths.
- The convergence gate checks the direct calls and rejects the private wrapper.

# Invariants

- Hosted Agent display names are not unique unless the aggregate lookup proves a single matching row.
- A missing hosted Agent name is not equivalent to the local device identity when the caller explicitly requested an Agent.
- Resolved hosted Agent URAs must parse as Agent URAs before they can become child callee or delegation authority.
- This slice preserves existing public behavior while moving ownership of ambiguity semantics to the aggregate layer.
- Large teach/acquire/forget transaction paths remain for a separate slice because they couple Agent registry rows, hosted identities, grant state, and workspace mutation.
- Runtime and CLI consumers may translate aggregate lookup errors into their own UX wording, but they must not inspect `local-agents.json` directly for display-name resolution.

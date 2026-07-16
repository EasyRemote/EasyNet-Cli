Objective
=========

Converge hosted owner-URA resolution in user-facing discovery surfaces onto the
Agent aggregate identity projection.

Public behavior stays stable: local discover still uses the injected registry
provider for registered Agent rows, CLI ability catalogue filtering still
resolves `--agent` to the same hosted owner URA, and hosted identity lookup
remains a hosted-identity concern rather than requiring a second registry read.

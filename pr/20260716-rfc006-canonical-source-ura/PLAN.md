# RFC-006 Canonical Source and URA Naming

## Goal

Make RFC-006 active documents self-contained canonical sources by removing
historical backup snapshot authority and replacing remaining Capability-URI
wording with URA terminology.

## Expected Effect

- Architecture convergence: one active RFC source owns the current state-object
  and OpenAI-compatibility semantics.
- Naming convergence: RFC-006 Appendix C follows the repository-wide URA-only
  rule.
- Review clarity: deleted backup snapshots cannot be treated as normative input
  during implementation or conformance review.

## Scope

- `docs/rfc/AXON-RFC-006-B-easynet-webapp.tex`
- `docs/rfc/AXON-RFC-006-C-openai-compat.tex`
- deleted historical snapshots for RFC-006-B

This slice does not touch daemon SDK requirements or Rust implementation files.

## Invariants

- Active RFC-006-B v0.6 is the canonical specification.
- Deleted `.bak` snapshots are not compatibility layers or normative sources.
- Appendix C uses URA for capability-key identity and examples.
- No public behavior changes.

## Verification Plan

- Search active RFC files for retired backup authority language.
- Search Appendix C for URI/caller_agent_uri/callee_agent_uri leftovers.
- Run architecture convergence and project structure gates.
- Run diff hygiene before staging.

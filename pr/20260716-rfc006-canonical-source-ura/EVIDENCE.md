# Evidence

## Source Findings

- `AXON-RFC-006-B-easynet-webapp.tex` had a backup list and a statement that
  v0.5.1 backup details were canonical unless contradicted by v0.6. That made
  deleted snapshots a second normative source.
- The active v0.6 document now states that retained implementation details are
  consolidated in v0.6 and that deleted historical snapshots are not normative.
- `AXON-RFC-006-C-openai-compat.tex` used Capability-URI wording and example
  variables `caller_agent_uri` / `callee_agent_uri` even though the project
  architecture is URA-only.

## Root-Fork Closed

RFC-006 no longer asks implementers to consult deleted backup files to discover
normative behavior. Appendix C no longer introduces a URI naming fork for
capability API keys.

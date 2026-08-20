# Decisions Log

## 2026-07-16

- Selected governance teach hosted identity authorization as the next root-fork slice because CodeGraph showed it as the remaining production caller of `lookup_hosted_agent_by_name`.
- Kept teach transaction persistence and descriptor import/forget state-machine work out of scope; this slice only moves hosted identity read ownership to the aggregate.
- Removed `local_agents::lookup_hosted_agent_by_name` after migration because no production caller remained and keeping it would preserve the obsolete display-name source fork.

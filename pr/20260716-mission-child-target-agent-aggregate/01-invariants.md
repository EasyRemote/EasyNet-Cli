# Mission Child Target Agent Aggregate Invariants

1. Traditional EAL `call ... on "<agent-name>"` must still reject when the
   device node id collides with a registered Agent surface name.
2. Default-tenant Agent ids must still match both bare name and
   `default/<name>` forms.
3. Non-default tenant Agent ids must still match both bare name and
   `<tenant>/<name>` forms.
4. Mission child hosted Agent lookup must reject empty names.
5. Mission child hosted Agent lookup must reject missing and ambiguous names.
6. Mission child hosted Agent lookup must reject non-Agent URAs.
7. Mission execution modules must not directly call `agent_registry::load_agents`
   or `local_agents::load` for child-target proof reads.

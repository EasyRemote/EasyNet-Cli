# Intent

Converge registered-agent workspace ownership on the durable `AgentEntry`
row. After `load_agents()` has migrated registry data, steady-state readers
must use the row's canonical `root_path` instead of reconstructing
`agents_root()/name`.

This closes the source-of-truth fork where list, publish, and skill-management
surfaces could silently act on a synthesized path that was not the registered
agent root.

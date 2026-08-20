# API Contract

Public behavior remains unchanged.

- `agent.stop` accepts `name` or `agent_ura` and preserves the root directory.
- `agent.stop` rejects `purge`.
- `agent.purge` accepts `name` or `agent_ura`, requires the registered
  `root_path`, and removes only that root through the purge FSM.
- Catalog descriptors continue to mark `agent.purge` destructive and leave
  `agent.stop` non-destructive.

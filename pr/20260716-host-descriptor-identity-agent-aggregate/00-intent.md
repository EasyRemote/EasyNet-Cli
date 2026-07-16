# Intent

Converge host descriptor catalog identity derivation onto the Agent aggregate
hosted-identity projection.

The descriptor catalog needs the host device, consent, MCP, and LLM profile
owner URAs. Before this slice, it derived those directly from the
`local-agents.json` file shape. That made the catalog a second owner of hosted
identity projection rules.

Public behavior stays stable: descriptor publication still returns an empty
catalog when the hosted identity file is missing or contains invalid owners,
and valid hosted profile owners continue to produce the same descriptor set.

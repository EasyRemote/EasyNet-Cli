# Boot Discovery Agent Aggregate Provider

## Goal

Converge boot-time `agent.discover` and A2A bridge registry providers onto the Agent aggregate read model.

## Root Fork

`agent.list` and hot-added Agent discover providers already read registered Agent rows through `AgentAggregateRepository`, but boot catalog registration still injects closures that call `agent_registry::load_agents()` directly. That lets discovery/A2A read `agents.json` without the hosted identity projection that belongs to the same Agent read model.

## Expected Effect

Architecture convergence. The public ability surfaces stay the same, while boot-time discovery providers no longer own a registry-only read path.

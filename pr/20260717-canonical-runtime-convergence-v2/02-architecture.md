# Canonical Runtime Convergence V2 - Architecture

## Layering

Axon owns canonical invocation, descriptor-bound proof, admission, lifecycle,
and receipt semantics. EasyNet-Cli owns daemon policy, local resources,
providers, and product execution surfaces. Backend code submits complete
invocations and does not become a proof or receipt authority.

## Current Slice: Descriptor Governed Schema Projection

Owner: EasyNet-Cli daemon ability control plane.

The governed schema hash projection is a daemon descriptor fact. It is not an
Axon protocol primitive and it is not a public SDK compatibility surface.

The projection must keep all hash inputs explicit:

- input schema;
- output receipt schema;
- access policy;
- hints;
- receipt semantics;
- admission action;
- description;
- source; and
- metadata.

Those fields now move as one semantic projection object instead of as a loose
parameter list. This keeps the descriptor hash boundary cohesive while
preserving the exact JSON projection used by existing hash computation.

## Current Slice: Mission Terminal Transition Facts

Owner: EasyNet-Cli daemon Mission/EAL orchestration.

Mission/EAL remains a daemon-owned composite `AbilityImpl` strategy, not an
Axon invocation ontology. Its persisted run lifecycle still needs a real state
machine: `running` may transition to exactly one terminal state, and terminal
states are immutable.

The terminal transition now separates:

- run context: mission name, source file, trace id, start timestamp, duration,
  and parent invocation context;
- completion facts: total/completed/failed step counts and ability graph
  traces; and
- failure facts: total step count and error text.

This keeps the mission lifecycle transition explicit without turning Mission
state into a second invocation/proof model.

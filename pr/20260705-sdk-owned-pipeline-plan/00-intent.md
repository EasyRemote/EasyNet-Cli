# SDK-Owned EasyRemote Pipeline Plan

## Objective

Move EasyRemote Pipeline planning, EAL rendering, and child Invocation
conformance checks behind the EasyNet-Cli Python SDK facade while preserving the
public EasyRemote `Pipeline`, `Step`, and `StepOutput` API shape.

## Boundary

- The daemon remains the only Mission/EAL runtime.
- The SDK owns Pipeline plan validation, EAL rendering, child Invocation intents,
  and MissionStatus conformance.
- EasyRemote keeps only product-facing aliases, target coercion, mission
  submission, and public error taxonomy projection.

## Non-goals

- Do not change the daemon SDK requirements spec.
- Do not add a Python planner, scheduler, retry engine, or receipt policy.
- Do not accept arbitrary daemon step ids as equivalent to planned step aliases.

# SDK-owned EasyRemote AgentControl stop cutover

## Objective

Complete the EasyRemote-facing agent lifecycle bridge for daemon-owned hosted
agents by adding SDK-owned `agent.stop` dispatch/projection and exposing it
through EasyRemote `AgentControl.stop`.

## Boundary

- The SDK owns daemon system ability names, Admin DTO validation, and
  EasyRemote projection of the agent lifecycle result.
- EasyRemote keeps only product ergonomics and CLI presentation.
- Dispatch still flows through the daemon-owned `agent.stop` system ability via
  the SDK Admin profile bridge.

## Non-goals

- Do not add new daemon agent lifecycle semantics.
- Do not fabricate URA values.
- Do not edit `docs/spec/daemon-sdk-requirements-v1.md`.

# RemoteApp target picker host preflight plan

## Intent

Make the E2E-01 host target-picker freshness harness fail before launching
native sentinel windows when the local EasyNet daemon cannot accept Invocation
traffic.

## Boundary

- Do not change RemoteApp runtime behavior or public ability contracts.
- Do not weaken live target picker evidence: the harness must still open the
  native fixture after daemon boot, then call `resource.refresh_remote_targets`
  and select the known fixture row from that live response.
- Keep decoded-frame/WebRTC E2E ownership in
  `host-remoteapp-decoded-frame-e2e.sh`.

## Invariants

- Host fixtures are external side effects; harnesses must prove daemon
  invocation readiness before launching them.
- Runtime started-at evidence must still be captured before fixture launch.
- Failure reports must be structured so CI and operators can distinguish
  daemon preflight failure from target inventory failure.
- The live inventory refresh must run exactly once after the fixture is ready.

## Verification plan

- Run target-picker self-test.
- Run remoteapp E2E acceptance static gate and mutation self-test.
- Run the live target-picker command in the current environment and verify it
  now fails before fixture launch when daemon invocation is unavailable.
- Run `git diff --check` and CodeGraph status.

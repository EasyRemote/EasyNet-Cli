# Go Live Smoke SystemAgent Callee Binding

## Problem

The Go SDK live smoke still constructed daemon runtime invocations with the Device URA as both caller and callee. After the Axon SDK projection gate rejects Device-owned ability projection, this exposed a real architecture seam: runtime abilities such as `observe.health` and `session.attach` are hosted by SystemAgent owners, not by the Device identity itself.

## Design

- Keep the public Go SDK invocation builder behavior unchanged.
- Bind the smoke caller to the paired user identity.
- Bind the smoke callee to the concrete device-scoped SystemAgent owner:
  - `observe.health` -> `agent/device.<device>.runtime-health`
  - `session.attach` -> `agent/device.<device>.session`
- Keep the invocation subject as the Device resource because the health/session operation is about the paired runtime device.
- Add self-test guards so the smoke cannot regress to `WithCalleeURA(deviceURA)` or descriptor resolution with `CalleeURA: deviceURA`.

## Expected Effect

The Go live smoke now exercises the same architecture as production runtime invocation: user caller, SystemAgent callee, Device subject. This removes the hidden Device-as-ability-owner dependency and aligns Go with the Python live smoke.

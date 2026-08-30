# Go RuntimeAbility Deadline SystemAgent Binding

## Problem

The Go RuntimeAbility direct deadline conformance case still built its test invocation with a Device URA as callee and an unrelated `er.weather` ability. After Device-owned ability projection was removed, the test no longer reached the runtime provider: descriptor projection failed before dispatch.

## Design

- Keep the tested behavior: provider-owned direct runtime invoke timeout must surface as a typed retry-safe SDK timeout.
- Bind the callee to the device-sponsored runtime-health SystemAgent.
- Keep the Device URA as the subject because the health operation acts on the paired runtime host.
- Use the `observe.health` descriptor so callee, descriptor, and subject are semantically aligned.

## Expected Effect

The conformance case now proves timeout behavior on the clean Invocation tuple:

```text
caller = agent/alice.sdk
callee = agent/device.dev-a.runtime-health
ability = observe.health
subject = device/dev-a
```

This removes the last Go conformance dependency on direct Device-as-callee projection.

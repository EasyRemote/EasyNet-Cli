# RemoteApp input injection evidence intent

Date: 2026-08-23

## Problem

RemoteApp now has input consent, input readiness, stale geometry rejection,
client telemetry, backpressure, and view-only fail-closed behavior. Those are
necessary, but they do not prove that pointer/keyboard input is actually
injected into the host OS with correct permission, focus, coordinate mapping,
and latency.

Without a live artifact contract, policy-only evidence can be mistaken for
interactive remote desktop readiness.

## Intent

Add a runner-agnostic input injection evidence verifier that:

- requires macOS display input injection to pass for pointer and keyboard;
- requires permission, input-control consent, display-global scope, target
  geometry revision, focus validation, coordinate mapping, and input-applied
  events;
- enforces latency bounds from client send time to host applied time;
- allows Windows/Linux only when they pass or explicitly report product
  unsupported state;
- rejects policy-only, component-mock, stale-geometry, no-focus, and
  product-complete claims.

## Non-goals

- Do not implement Windows/Linux input injection in this change.
- Do not change the high-frequency input data plane into Invocation.
- Do not claim product completion from self-test or skipped reports.

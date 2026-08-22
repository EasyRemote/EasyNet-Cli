# RemoteApp network fallback evidence intent

Date: 2026-08-23

## Problem

RemoteApp currently has typed route models, ICE server projection, and frontend
route visibility. Those are required product plumbing, but they do not prove
that direct, STUN, TURN, or EasyNet relay paths are reachable in real network
conditions.

Without a live artifact contract, source-level route checks can be mistaken for
network product readiness.

## Intent

Add a runner-agnostic network fallback evidence verifier that:

- accepts evidence from a real two-device, network-namespace, or deployment
  runner;
- requires direct, STUN srflx, TURN relay, and EasyNet relay scenarios;
- validates WebRTC selected candidate-pair evidence and rendered media for
  each scenario;
- requires public RemoteApp session abilities and selected Resource URA subject
  binding;
- rejects component-mock evidence, leaked credentials, and product-complete
  claims.

## Non-goals

- Do not provision a TURN server or EasyNet relay deployment in this change.
- Do not claim product completion from self-test or skipped reports.
- Do not replace cross-device, frontend Browser/Tauri, OS capture, input, or
  codec/adaptation E2E evidence.

# RemoteApp Browser/Tauri lifecycle E2E intent

Date: 2026-08-23

## Problem

Frontend RemoteApp evidence already covered component/store behavior and a
combined product-flow harness, but the full Browser/Tauri lifecycle requirement
still lacked a dedicated evidence contract. Without that contract, component
tests or host-only scripts could be mistaken for real frontend product proof.

## Intent

Add a Browser/Tauri lifecycle evidence verifier that:

- accepts evidence from a real browser or Tauri runner;
- validates the lifecycle order from app load through target picker,
  permission preflight, consent, session creation, WebRTC attach, watch events,
  media presentation, input application or policy block, session end, and
  visible terminal receipt;
- requires public RemoteApp ability names and the selected Resource URA subject;
- rejects component-mock evidence and product-complete claims;
- remains runner-agnostic so Playwright, Tauri driver, or another real UI
  runner can produce the same artifact.

## Non-goals

- Do not add Playwright or Tauri dependencies to the frontend package.
- Do not claim product completion from self-test or skipped reports.
- Do not replace daemon/host/cross-device E2E evidence.

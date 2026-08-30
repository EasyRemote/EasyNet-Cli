# RemoteApp cross-platform capture evidence intent

Date: 2026-08-23

## Problem

RemoteApp has macOS ScreenCaptureKit target binding and fail-closed behavior for
non-macOS application/window capture. That is not enough to claim product-level
coverage across macOS, Windows, and Linux.

The product needs a live artifact contract that distinguishes:

- real display/window/application capture;
- explicit product unsupported state;
- invalid fallback, especially widening window/application capture to a display
  capture path.

## Intent

Add a runner-agnostic cross-platform capture evidence verifier that:

- requires macOS display, window, and application capture to pass with rendered
  frames and exact target binding proof;
- requires Windows and Linux to either pass display/window/application capture
  or report an explicit product unsupported state;
- rejects source-only proof, component mocks, missing rendered frames, missing
  terminal receipts, and display fallback for window/application targets;
- preserves public RemoteApp ability and selected Resource URA subject binding.

## Non-goals

- Do not implement Windows or Linux native capture in this change.
- Do not claim product completion from self-test or skipped reports.
- Do not replace frontend Browser/Tauri, network fallback, input injection, or
  codec/adaptation E2E evidence.

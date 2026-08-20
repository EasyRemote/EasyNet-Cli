# SDK Provider Loader Proof

## Goal

Close the provider-loading proof gap without changing public SDK behavior.
Production SDK code may accept an explicit provider library path or ask the
platform loader for an installed library name. It must not rediscover
repository build outputs such as `target/debug`, `target/release`, or `deps`.

## Root Fork

This belongs to the canonical SDK versus EasyNet provider fork. The SDK
runtime model cannot treat a developer checkout layout as a production provider
locator. Development paths belong in smoke scripts and tests, not in runtime
SDK loading logic.

## Scope

- Add an executable product-neutrality gate for development build-directory
  lookup in SDK production sources.
- Add self-test fixtures proving the gate catches Go and Python loader
  regressions.
- Leave public C ABI names and source-compatible aliases unchanged because ABI
  renaming requires a separate SPEC-backed major cutover.

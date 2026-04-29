# RFC-005 — Device Backend Selection (gating note for #197)

Status: **gating decision required**, no implementation work in
flight. PR3a (#199) closed the language-layer surface for the
eight RFC-005 v3.2 media abilities; PR3 (#197) is the platform-
backend integration that follows it. This note exists so the
follow-up does not get framed as a normal coding task — the
blocker is crate selection plus a hardware verification matrix,
not lines of Rust.

## Why this is a gate, not a backlog item

Three distinct platform-backend domains hide behind "PR3":

* **Audio** (mic capture, speaker playback) — cpal is the
  obvious pick because no other Rust crate currently spans
  CoreAudio + WASAPI + ALSA + PulseAudio with comparable
  maturity. The decision is not "which crate" but "do we accept
  cpal's known sample-rate-mismatch behavior on macOS, and do we
  ship a resampler in this PR or later?"
* **Camera** — nokhwa is the de-facto choice on Linux+macOS,
  but its Windows backend goes through MediaFoundation and that
  path is fragile in practice. The decision is whether to ship
  Windows in scope at all, ship it gated behind a feature flag,
  or defer.
* **Screen capture** — at least three viable crates
  (screenshots, xcap, scrap) with no clear winner. Each has
  different macOS Screen Recording permission ergonomics, and
  on macOS the permission prompt is a system-modal dialog that
  blocks the calling thread. The decision is which crate, plus
  who owns the permission flow.

None of these can be locked without attempting integration on
each platform. Doing the integration is exactly what PR3 is.
Picking the crate without trying it is exactly what we should
not do. So PR3 has to be preceded by a small selection pass.

## What this RFC is asking for

Three sub-decisions, one per backend domain. Each sub-decision is
a one-paragraph answer plus a verification matrix.

### A. Audio backend

* **Default proposal**: cpal.
* **Open question**: do we land a resampler (e.g. `rubato`) in
  PR3 or in a follow-up? cpal exposes the device's native sample
  rate; the ability schema offers 16 kHz / 24 kHz / 48 kHz. On
  macOS the default input device is typically 48 kHz, so a 16
  kHz request needs a downsample.
* **Reject reason for alternatives**: `coreaudio-rs` is
  Apple-only; `pipewire-rs` is Linux-only; rolling our own ALSA
  binding is not in scope.

### B. Camera backend

* **Default proposal**: nokhwa.
* **Open question**: Windows scope. Three options:
  1. Land Windows in PR3, accept MediaFoundation flakiness, ship
     a "known-issues" doc.
  2. Land Linux + macOS in PR3; gate Windows behind
     `--cfg easynet_camera_windows` until a maintainer-volunteer
     stress-tests it.
  3. Defer Windows entirely; document `camera.snapshot` and
     `camera.subscribe` as "platform: linux, macos" in the
     ability descriptor metadata.
* **Reject reason for alternatives**: `v4l` is Linux-only;
  `escapi` is Windows-only; `openpnp-capture-rs` is unmaintained.

### C. Screen capture backend

* **Default proposal**: TBD — needs a selection pass.
* **Open question**: which crate, plus who handles the macOS
  Screen Recording permission. The permission prompt:
  * is system-modal (blocks the thread that triggered it)
  * persists per-application (not per-invocation)
  * has no programmatic "are we authorized?" check before
    triggering capture; the call either succeeds or returns a
    blank frame
* **Candidates**:
  | Crate         | Linux | macOS | Windows | Maintenance |
  |---------------|-------|-------|---------|-------------|
  | `screenshots` | ✓     | ✓     | ✓       | Active      |
  | `xcap`        | ✓     | ✓     | ✓       | Active      |
  | `scrap`       | ✓     | ✓     | ✓       | Stale       |
* The choice should be made after a 30-minute spike per crate on
  one developer's macOS box, not from the README.

## Minimal verification matrix

Per backend, PR3 ships only after this matrix is green:

| Test                                  | mic | camera | screen | speaker |
|---------------------------------------|-----|--------|--------|---------|
| macOS arm64 — capture single frame    | ✓   | ✓      | ✓      | ✓       |
| Linux x86_64 (PulseAudio) — single    | ✓   | ✓      | ✓      | ✓       |
| Windows — single frame                | ?   | ?      | ?      | ?       |
| Resource unplugged mid-stream         | ✓   | ✓      | n/a    | ✓       |
| Permission denied surfaces correctly  | n/a | ✓      | ✓      | n/a     |

`?` cells block on the Windows-scope decision (B above).

## Failure-class boundary

Per **INV-MAC-CHAIN-TRANSMITTED** (plan v3.2): the platform-
backend layer's failure modes are NOT Axon semantic violations.
Specifically:

| Symptom                           | Class               | Owner    |
|-----------------------------------|---------------------|----------|
| cpal xrun under load              | Platform            | backend  |
| Camera unplugged mid-stream       | Platform            | backend  |
| macOS denies Screen Recording     | Platform permission | backend  |
| MAC chain breaks across frames    | Axon protocol bug   | runtime  |
| Subject not in resources.json     | Axon contract bug   | runtime  |
| Frame PTS gap (consumer side)     | Consumer concern    | consumer |

The PR3 author should resist any temptation to "fix" the
platform-class symptoms by injecting frames, reconnecting
silently, or papering over permission failures. The reason: the
audit chain is what it is precisely because it does not lie about
what was transmitted. Hiding device-layer failures inside
seemingly-successful receipts breaks the trust the whole
transition-receipt subsystem is built on.

## Three-layer backend switchability

PR3a's `SnapshotBackend` trait already shows the shape: the
dispatch / receipt / validation code is backend-agnostic, and
the backend is an `Arc<dyn Trait>` swapped at registration time.
PR3 should keep this shape uniform across all eight abilities,
producing three implementations per backend domain:

* **stub** — what `media_abilities.rs` already ships. Returns a
  terminal failure with `reason="device backend not yet wired"`.
  Used in tests that exercise registration / schema / dispatch
  without touching hardware.
* **synthetic / fake** — what `media::camera_snapshot::Synthetic
  Backend` ships. Deterministic, hardware-free, suitable for CI
  and for end-to-end tests that need a real receipt-shaped
  response.
* **real** — cpal / nokhwa / chosen-screen-crate. Hardware-bound,
  not runnable in CI, gated behind the verification matrix
  above.

The daemon picks one per process via a config knob (default:
real on a host, synthetic on `EASYNET_TEST_MODE=1`). Tests pin
synthetic explicitly. Stubs stay registered as the fallback for
abilities the host genuinely cannot satisfy (e.g. headless
container with no audio devices), so a caller gets a clear "not
wired" terminal receipt rather than a panic.

## Closeout shape

When PR3 lands, this RFC's status flips to **resolved** and the
sub-decisions move into a permanent "Backend choices" section
near the top of the file, recording exactly what got picked and
why. The verification matrix becomes a CI doc pointer (the
matrix itself runs in a hardware-attached test rig, not in
GitHub Actions).

Until then: do not start PR3.

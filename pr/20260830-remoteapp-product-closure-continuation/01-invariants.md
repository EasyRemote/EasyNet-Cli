# Invariants

## Architecture

- The device-sponsored RemoteDesktop SystemAgent owns the public descriptors;
  the plugin is the AbilityImpl and the selected Resource is the subject.
- Runtime Core owns Invocation authority, receipts, cancellation, timeout, and
  exactly-one terminal lifecycle. WebRTC/native helpers own only the bounded
  session data plane.
- Window/application capture and input must bind the same immutable target
  identity and current generation. No display fallback is permitted.

## Product behavior

- Every session, transport generation, media generation, and relay lease has
  bounded queues, explicit ownership, stale-generation fencing, and one
  terminal outcome.
- Permission revocation, target loss, helper crash, transport loss, cancellation,
  and timeout fail closed and remain visible to the browser.
- A platform or route is product-ready only after a real native/live runner
  observes the intended OS/media/network effect and a terminal receipt.
- Windows and Linux Window/Application authority must include one canonical
  process-instance proof shared by discovery, observation, capture, focus, and
  input. Advisory xcap PID metadata is never an authority fallback.
- Discovery and capture evaluate the same `CaptureEligibleSurface` predicate;
  an inventory row may not advertise a target that the native capture backend
  rejects by construction.
- Browser close during session creation is fenced by an operation generation;
  a late create response is compensated with idempotent `end_session`.
- An ambiguous end response preserves a closing aggregate until terminal state
  is reconciled. Inventory omission never replaces a session's bound subject.
- Permission verification pending pauses transport while lease and event
  supervision continue for the same session.

## Evidence

- Accepted reports must bind source revision, build identities, platform,
  selected Resource, session, route, timestamps, and artifact digests.
- Self-tests, source inspection, old missing artifacts, and dirty unsigned child
  proofs cannot produce `product_complete_claim=true`.

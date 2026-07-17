# Execution Checklist

## RF-5 / RF-3

- [x] Inventory plain admission, signature, and fallback signer surfaces.
- [x] Gate public plain admission/fallback signer reintroduction.
- [x] Require descriptor-bound invocation as canonical runtime admission input.
- [ ] Re-run gates against current worktree and close any newly exposed public
      surface.

## RF-8 / RF-7

- [x] Inventory unary, stream, bidi, loopback, exact-route, and remote dispatch
      paths.
- [x] Gate complete tuple ingress and LocalRuntime-only ability routes.
- [ ] Delete any remaining direct ability response synthesis that is not boot,
      health, status, or diagnostics.
- [ ] Re-run daemon migration gates against current worktree.

## RF-4

- [x] Normalize capability states to Unsupported, Seam, ProviderBacked, and
      CutoverReady.
- [x] Gate lifecycle vectors for start, dispatch, stream, bidi, child dispatch,
      cancel, deadline, terminal receipt, and restart recovery.
- [x] Bind a provider-backed unary `deadline` vector for Go and Python to
      concrete selectors and matrix evidence.
- [x] Bind a provider-backed native runtime `start` vector for Go and Python to
      concrete environment/process-root selectors and matrix evidence.
- [x] Bind Go and Python `runtime_environment`, `runtime_connection`, and
      `runtime_lifecycle` to the same direct runtime provider proof so the
      generic runtime concepts no longer remain seam-only while their concrete
      native runtime wrapper is provider-backed.
- [x] Require direct runtime bidi frame0 validation before session entry for
      Go and Python, then bind the bidi provider proof to shared matrix
      evidence.
- [x] Bind stream and bidi `cancel` vectors for Go and Python to shared
      conformance selectors that prove non-terminal cancel acknowledgement and
      rejection of synthetic terminal cancel acks.
- [x] Bind stream and bidi `deadline` vectors for Go and Python to direct
      runtime provider selectors that prove provider-owned invocation timeout,
      typed `TIMEOUT` projection, cleanup, and retryability of the parent
      runtime transport after a timed-out session.
- [x] Bind stream and bidi `dispatch` vectors for Go and Python to direct
      runtime provider selectors that prove runtime provider entry, observable
      non-terminal provider output, and terminal receipt projection after
      dispatch.
- [x] Bind Go and Python generic ability invocation facades to a
      provider-backed `child_dispatch` vector that derives child causality from
      a parent terminal receipt, dispatches through Runtime Core, and observes
      the parent receipt link in the child terminal receipt.
- [x] Bind Go and Python generic ability invocation facades to provider-backed
      `dispatch`, `stream_open`, `bidi_open`, `cancel`, and
      `terminal_receipt` vectors through Runtime Core selectors, while keeping
      `deadline`, `restart_recover`, and `start` open.
- [x] Bind Go and Python generic ability invocation facades to a
      provider-backed `deadline` vector that proves Runtime Core provider
      timeout ownership and retry after cleanup without adding a separate
      ability timeout model.
- [ ] Re-run SDK parity gates against current worktree.

## RF-6

- [x] Gate receipt constructors and parsers against omitted proof facts.
- [ ] Re-run focused Go/Python receipt tests against current worktree.

## RF-1 / RF-2

- [x] Gate product SDK feature families in canonical packages.
- [x] Gate daemon-owned Mission/EAL execution boundaries.
- [ ] Re-run product neutrality and Mission/EAL boundary gates.

## RF-9

- [x] Gate active URA terminology and transport URI classification.
- [x] Delegate schema-copy verification to the canonical Axon proto syncer.
- [ ] Re-run terminology and schema gates against current worktree.

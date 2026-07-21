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
- [x] Remove CLI-side boot port compatibility reconstruction so daemon
      `BootEvent::PortChosen` remains the only port-choice progress authority.
- [x] Remove MCP installer no-op compatibility flags so `mcp install` writes
      only the live `mcp serve --tenant [--agent]` contract.
- [x] Remove the `federation.advertise_abilities` missing-catalog success path
      so owner projection publication cannot return `ack=true` without writing
      to the daemon-owned ability catalog read model.
- [x] Remove the `federation.heartbeat` missing-catalog success path so owner
      projection lease refresh cannot return `membership_status=active` without
      a daemon-owned ability catalog read model.
- [x] Remove federation/namespace resolve missing-catalog read-path fallback
      so route and directory answers cannot treat absent projection authority
      as an empty ability set.
- [x] Remove FFI remote descriptor static-system-catalog synthesis so
      descriptor resolution cannot report a remote owner ability without route
      and signer authority evidence.
- [x] Remove `federation.revoke` missing-catalog cleanup fallback so revocation
      cannot acknowledge success without clearing the daemon-owned owner
      projection read model.
- [x] Remove agent registry load-time v1 migration so runtime reads cannot
      mutate agent roots, write `.v1.bak`, or infer canonical roots from
      retired registry rows.
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
- [x] Bind Go and Python generic ability invocation facades to a
      provider-backed `start` vector by borrowing the ability facade from the
      Native Runtime provider graph instead of introducing a product-specific
      ability lifecycle root.
- [x] Re-run SDK parity gates against current worktree.

## RF-6

- [x] Gate receipt constructors and parsers against omitted proof facts.
- [ ] Re-run focused Go/Python receipt tests against current worktree.

## RF-1 / RF-2

- [x] Gate product SDK feature families in canonical packages.
- [x] Gate daemon-owned Mission/EAL execution boundaries.
- [x] Remove Go/Python product-owned Directory wire DTOs from canonical SDK
      source and public API inventory.
- [x] Gate product-owned Directory wire DTO/file reintroduction in SDK
      product-neutrality.
- [x] Remove `federation.advertise_agent` top-level `host_ura` ingress repair
      and gate the retired request field.
- [x] Remove `agent.start` model_present inference from the `model` field and
      gate the explicit model-write intent.
- [ ] Re-run Mission/EAL boundary gates.

## RF-9

- [x] Gate active URA terminology and transport URI classification.
- [x] Delegate schema-copy verification to the canonical Axon proto syncer.
- [x] Remove Go SDK `Ura` compatibility alias from the canonical public API
      surface and conformance evidence.
- [ ] Re-run terminology and schema gates against current worktree.

# AXON-RFC-006 — Stateful EasyNet

**Status:** Draft, partial.
**Scope of this file:** Appendix B only.
**Author:** Silan Hu <silan.hu@u.nus.edu>

---

## Why this file exists in this shape

The main body of RFC-006 (sections 1–7) is owned by the protocol lead
and is **deliberately not drafted yet**. This appendix is written
**first**, as a pressure test: it picks the most adversarial worked
example (`easynet.web_app`) and pins down what RFC-006 main MUST
define in order for that example to be consistent.

Reading order, when the main body lands:

1. RFC-006 main (sections 1–7) — minimal stateful semantics.
2. Appendix A — `easynet.page`, the simple worked example.
3. Appendix B (this file) — `easynet.web_app`, the pressure test.
4. §B.8 of this file — the constraint list main MUST satisfy. If
   the main body conflicts with §B.8, it is the main body that
   needs revision, not §B.8.

```
Main sections 1–7              ← TBD
Appendix A: easynet.page       ← TBD
Appendix B: easynet.web_app    ← THIS FILE
```

---

## B.0  Local symbols

For purposes of this appendix only. RFC-006 main may give these
symbols broader or differently-named definitions; if it does, the
mapping must preserve every property listed below. The whole point of
writing the appendix first is to refuse to depend on main: any
"per §x.y of main" reference is forbidden in this file.

```
state_type
    A stable string identifier for a class of state objects.
    Format: "<namespace>.<noun>", lowercase.
    Example: "easynet.web_app".

state_key
    A tuple that uniquely identifies one state object instance
    within a state_type. Stable for the lifetime of the object;
    survives node restart, ownership transfer, and replay.
    For easynet.web_app: (realm, owner_agent_uri, app_slug).

canonical_state_hash
    A deterministic hash over the canonical fields of one state
    object's current value. Two state objects with identical
    canonical fields MUST hash to the same value, regardless of
    runtime overlay, observation order, or implementation node.
    Hash MUST exclude every field marked canonical:false in the
    schema (see TR-INV-3 in §B.8).

CanonicalReceipt
    A receipt that advances canonical state. Required fields:
      state_type
      state_key
      pre_state_hash    (canonical_state_hash before transition)
      post_state_hash   (canonical_state_hash after transition)
      state_version     (monotonic integer per state_key)
      owner_signature   (signature by canonical owner_agent;
                         see TR-INV-4 in §B.8)
    A CanonicalReceipt MUST be replayable offline: given a
    sequence of CanonicalReceipts and an empty starting state,
    a verifier reconstructs the same canonical_state_hash without
    consulting the network.

OperationalReceipt
    A receipt that records a runtime / overlay event. MUST NOT
    carry pre_state_hash or post_state_hash referring to the
    canonical state. MAY carry runtime_node, pid, port, log
    references, progress fractions, or other observability
    payload. NOT required for replay correctness; loss of any
    subset of OperationalReceipts MUST NOT affect canonical
    correctness (see TR-INV-2 in §B.8).
```

These five symbols are the entire vocabulary B.1–B.7 use. The rest
is pure construction.

---

## B.1  State object

```
state_type:  easynet.web_app

state_key:
    (realm, owner_agent_uri, app_slug)

      realm             logical namespace, e.g. "default" or
                        "acme"; carried in URAs.
      owner_agent_uri   the canonical authority for this state
                        object. Every CanonicalReceipt MUST carry
                        this agent's signature. The owner_agent
                        is named at create time and IMMUTABLE for
                        the life of the state_key (transferring
                        ownership is a delete + create, not a
                        canonical update).
      app_slug          human-readable identifier within the
                        owner's namespace, kebab-case.

object identity (URA form):
    easynet:///r/<realm>/agent/<owner>/web_app/<slug>
```

Why owner_agent is in the key, not a field: it makes "who can sign
canonical transitions for this object" a property of the key, not a
mutable field. A field-level owner would let a transition rewrite
its own authority — a privilege escalation primitive RFC-006 should
refuse to expose.

---

## B.2  Canonical state fields (enter canonical hash)

These fields participate in `canonical_state_hash`. Mutation of any
of them is a canonical transition and emits a CanonicalReceipt.

```
manifest_hash         hash of the user-visible manifest:
                      app_name, description, declared
                      build_command, declared serve_command,
                      declared port hint, declared visibility
                      policy. This is the document the owner
                      authored; subsequent fields are derived
                      facts about it.

source_snapshot_hash  hash of the source tree as captured at the
                      most recent webapp.build invocation. NULL
                      while state ∈ {Created}; non-NULL once
                      Building has been entered at least once.

build_artifact_hash   content hash of the produced static /
                      runnable artifact bundle. NULL until first
                      successful build. The artifact bytes are
                      external to the state object — see B.3 note
                      on content-as-blob; only the hash enters
                      canonical state.

visibility            ∈ {private, realm, public}. Hub view layers
                      MUST consult this; private and realm objects
                      are NOT served on public hub HTTP entry
                      points.

version               monotonic integer, equals the count of
                      CanonicalReceipts that have advanced this
                      state_key. Strictly canonical: a verifier
                      must be able to compute it from the receipt
                      log alone.
```

**Content-as-blob rule.** The artifact bytes (compiled JS bundle,
compressed tarball, whatever) are NOT in canonical state. Only
`build_artifact_hash` is. The bytes live in a content-addressed
blob store keyed by that hash, fetched on demand by the hub view
or by a runtime executor. This is the same rule pages-A intends
to apply to HTML payloads, surfaced here independently — RFC-006
main MUST give the rule once at the protocol level so both
appendices inherit it.

---

## B.3  Ephemeral runtime fields (excluded from canonical hash)

These fields describe **operational facts about an executor right
now**. They are NOT canonical: a fresh node loading the receipt log
must produce a state object whose canonical hash does not depend on
any of them.

```
runtime_node_uri      which node currently executes this app
                      (build, serve). MAY change between
                      OperationalReceipts; MUST NOT change
                      canonical hash.
process_id            OS-level pid of the currently-running serve
                      process. NULL when not Running.
bound_port            TCP port the dev/preview server is listening
                      on. NULL when not Running.
websocket_routes      array of (path_prefix, target) pairs the
                      dev server upgrades on. Recomputed each time
                      Running is entered; NEVER carried across
                      stop/start.
started_at            wall-clock instant the current Running
                      runtime overlay started. NULL when not
                      Running. Cleared on Stop.
last_health_at        wall-clock instant of the last successful
                      health probe. NULL when not Running.
build_progress        fractional completion of an in-flight build,
                      0.0..1.0; NULL when not Building. Reported
                      via OperationalReceipts; NEVER advances
                      canonical hash even when it crosses 1.0
                      (the canonical advance is the terminal
                      build.completed receipt, not the progress
                      reaching 1.0).
```

Note that `bound_port` is a runtime allocation **fact**, not a
semantic property of the app. Two consecutive `webapp.serve`
invocations may bind different ports; canonical hash MUST be
identical across them. This single field is the cleanest example of
why §B.8 demands schema-level canonicality marking (TR-INV-3): an
implementation that forgot to mark `bound_port` as canonical:false
would produce non-deterministic state hashes across reboots.

---

## B.4  State spaces — canonical + runtime overlay

Two **independent** state spaces. They compose; they do not merge.

### B.4.1  Canonical state space

```
WebAppState ::=
    Created   →   Building   →   Built
                        ↓
                        →   Failed
    *  →  Deleted   (terminal, irreversible)
```

```
Created     manifest written, no build attempted.
Building    a webapp.build is in flight (long-running canonical
            transition; see B.6).
Built       a successful build exists; build_artifact_hash is
            non-NULL and points to a fetchable blob.
Failed      the most recent build attempt failed; the state object
            persists so the owner can retry. May transition back
            into Building.
Deleted     terminal. Owner has tombstoned the object. No further
            transitions accepted. Receipts beyond the Deleted
            terminal MUST be rejected.
```

### B.4.2  Runtime overlay state space

```
WebAppRuntimeState ::=
    Stopped   →   Starting   →   Running
                        ↓               ↓
                        →   Crashed   ←
    Running / Crashed  →  Stopped
```

```
Stopped     no executor process associated with this app on any
            node.
Starting    webapp.serve has been invoked; an executor is
            launching the dev/preview server but it has not yet
            confirmed liveness.
Running     executor is up; bound_port and runtime_node_uri are
            populated; HTTP/WS reverse proxy can succeed.
Crashed     executor exited unexpectedly while previously
            Running; the canonical state is unchanged. May be
            recovered via webapp.serve again.
```

### B.4.3  Cross-product matrix (allowed combinations)

A `(canonical, runtime)` pair is admissible only if it appears in
this matrix. Implementations MUST refuse transitions that would
produce a forbidden pair.

```
                Stopped  Starting  Running  Crashed
    Created        ✓        ✗         ✗        ✗
    Building       ✓        ✗         ✗        ✗
    Built          ✓        ✓         ✓        ✓
    Failed         ✓        ✗         ✗        ✗
    Deleted        ✓        ✗         ✗        ✗   (transient on
                                                    teardown only)
```

Justification of the ✗ cells:

* `Created/Starting`, `Building/Starting`, `Failed/Starting`:
  serving requires an artifact. Without a successful build there
  is nothing to launch.
* `Building/Running`: a single state object cannot simultaneously
  produce a new artifact and serve the previous one under the same
  app_slug — concurrent builds against a serving app are a
  separate canonical operation (out of scope for B; §B.8 will note
  that RFC-006 main needs an explicit answer for "concurrent
  canonical transitions on the same key").

The two state spaces evolve under DIFFERENT receipt classes:
canonical transitions emit CanonicalReceipt, runtime transitions
emit OperationalReceipt. A single ability invocation MUST NOT emit
both classes for the same logical effect; if it did, the receipt
log would not be uniquely classifiable.

---

## B.5  Transition abilities

Two groups, separated by which state space they advance.

### B.5.1  Canonical group

These mutate canonical state and emit `CanonicalReceipt`. Owner
signature is mandatory (TR-INV-4).

```
webapp.create
    pre :  ∅
    post:  Created
    args:  manifest (the user-visible declaration)
    receipt fields (canonical):
        pre_state_hash   = 0 (empty)
        post_state_hash  = hash(manifest_hash || NULL || NULL || visibility || 1)
        state_version    = 1
        owner_signature  = required

webapp.build
    pre :  Created  |  Built  |  Failed
    post:  Building (transient) → Built | Failed
    args:  optional source_override
    receipts: see B.6 (long-running)

webapp.delete
    pre :  any non-Deleted state
    post:  Deleted
    args:  none
    side effect: any concurrently-Running runtime overlay is
                 forcibly torn down via implicit webapp.stop;
                 that stop emits an OperationalReceipt, the
                 deletion itself emits a CanonicalReceipt.
```

### B.5.2  Runtime group

These mutate runtime overlay only. They emit `OperationalReceipt`.
They MAY require canonical preconditions but MUST NOT change
canonical state.

```
webapp.serve
    canonical precondition:  WebAppState == Built
    runtime pre :  Stopped | Crashed
    runtime post:  Starting → Running | Crashed
    args:  optional port_hint, optional executor_node_uri
    receipt class: OperationalReceipt
        runtime_node_uri = chosen executor
        bound_port       = allocated
        started_at       = now
    canonical_state_hash MUST NOT change across this call.

webapp.stop
    canonical precondition:  WebAppState == Built (no-op
                              otherwise except in the implicit-
                              stop case noted under webapp.delete)
    runtime pre :  Starting | Running | Crashed
    runtime post:  Stopped
    args:  none
    receipt class: OperationalReceipt
    canonical_state_hash MUST NOT change across this call.
```

`webapp.serve` is the cleanest existence proof of TR-INV-X (now
TR-INV-1+5 in §B.8): a successful invocation observably mutates
which executor is running, which port it occupies, which WS routes
exist — yet the canonical hash of the state object is byte-identical
to what it was before the call.

---

## B.6  Long-running transition receipt model

`webapp.build` is the sole long-running transition in this
appendix. Its receipt sequence is the model for every long-running
canonical transition RFC-006 main may admit.

```
sequence on a successful build:
    [1]  build.started      OperationalReceipt
    [2]  build.progress*    OperationalReceipt   (0..N, lossy)
    [3]  build.completed    CanonicalReceipt     (exactly one,
                                                   pre=Building,
                                                   post=Built)

sequence on a failed build:
    [1]  build.started      OperationalReceipt
    [2]  build.progress*    OperationalReceipt   (0..N, lossy)
    [3]  build.failed       CanonicalReceipt     (exactly one,
                                                   pre=Building,
                                                   post=Failed,
                                                   reason_hash
                                                   required)
```

Properties this implies:

```
P1  Canonical transition Building → {Built, Failed} is NOT atomic
    in wall-clock time, but IS atomic in receipt order: there is
    exactly one terminal CanonicalReceipt, and the canonical hash
    advances at exactly that receipt.

P2  Replay of CanonicalReceipts ALONE reconstructs canonical
    state. Specifically, replay sees:
        webapp.create        → Created
        webapp.build (terminal completed)
                             → Built       (Building never appears
                                            in the canonical view;
                                            it is a transient marker
                                            visible only to live
                                            observers via the
                                            started+progress
                                            OperationalReceipts)
    A verifier with no OperationalReceipts at all reaches the same
    Built state with the same hash.

P3  Progress receipts are observability, not history. Subscribers
    that connected mid-build receive subsequent progress; those
    that reconnect after completion see only the canonical
    terminal receipt. RFC-006 main MUST allow this asymmetry.

P4  build.started is OperationalReceipt, NOT CanonicalReceipt,
    even though it transitions Created/Failed/Built → Building.
    This is the appendix's most subtle stance: Building is a
    canonical state in B.4.1, but its ENTRY is recorded
    operationally because the entry on its own carries no
    durably-meaningful information (a build that started but
    produced no terminal is indistinguishable from no build at
    all, after a node restart). Only the terminal receipt commits
    a canonical advance. Implementations that want a "Building is
    live" indicator derive it from the presence of a started
    receipt without a matching terminal.

    This is unusual enough that it MUST be explicitly endorsed or
    rejected by RFC-006 main (TR-INV-1). If main rejects it, B.6
    has to change: build.started becomes canonical, and replay
    rules need to handle "canonical state advanced into Building
    but no terminal exists" as a recoverable state.
```

---

## B.7  Hub view behavior matrix

The Hub is a node that exposes EasyNet state objects over public
HTTP. It performs a path-to-URA translation, queries canonical +
runtime state for the resolved object, and projects two views:

```
human_view   served on the public HTTP path; rendered for browsers.
agent_view   served at the URA itself; structured, machine-actionable,
             may embed EAL fragments and ability surface descriptors.
```

**Both views are pure projections.** No view computation mutates
state. Two views computed from the same `(canonical_state,
runtime_state)` pair are deterministic functions of that pair
(TR-INV-6).

For each `(canonical, runtime)` admissible pair from B.4.3:

```
Created / Stopped
    human_view   503  "App registered, not yet built. Owner action
                       required."
    agent_view   {
                   state_type: "easynet.web_app",
                   state_key: <key>,
                   canonical: { state: "Created", manifest_hash: ... },
                   runtime:   { state: "Stopped" },
                   ability_surface: ["webapp.build", "webapp.delete"],
                   eal_hint: "call webapp.build on <owner> with {} timeout 600"
                 }

Building / Stopped
    human_view   503  progress page. Body subscribes to
                      build.progress OperationalReceipts and
                      streams them to the browser (SSE or
                      long-poll).
    agent_view   {
                   canonical: { state: "Building", source_snapshot_hash: ... },
                   runtime:   { state: "Stopped",
                                build_progress: 0.62,
                                latest_log_ref: "blob:..." },
                   ability_surface: [],   // no canonical advance
                                          // possible while Building
                   subscriptions: [
                     "operational://<state_key>/build.progress"
                   ]
                 }

Built / Stopped
    human_view   200  static artifact reverse-proxied from the
                      build_artifact_hash blob (same code path
                      Appendix A's pages will use for HTML
                      payloads).
    agent_view   {
                   canonical: { state: "Built",
                                build_artifact_hash: ...,
                                visibility: "public",
                                version: 7 },
                   runtime:   { state: "Stopped" },
                   ability_surface: ["webapp.serve",
                                     "webapp.build",
                                     "webapp.delete"],
                   history: {
                     versions: 7,
                     latest_canonical_receipt_id: "...",
                     receipt_log_ura:
                       "easynet:///r/<realm>/agent/<owner>/web_app/<slug>/receipts"
                   },
                   eal_hint: "call webapp.serve on <owner> with {} timeout 60"
                 }

    Note on `history`: under TR-INV-11 every canonical state object
    is independently auditable. A peer agent reading this view can
    fetch receipt_log_ura, replay the chain, and prove "this app's
    artifact was authored by <owner> at version 7, signed at time T,
    and version 6's artifact_hash was X". This is what makes the
    web layer agent-AUDITABLE, not merely agent-readable. A
    shopping page exposed this way can be verified to have shown
    a particular price at a particular version, signed by its
    owner — the property a traditional web cache or CDN cannot
    offer.

Built / Starting
    human_view   503  "Starting up." Brief transient.
    agent_view   { canonical: { state: "Built", ... },
                   runtime:   { state: "Starting" },
                   ability_surface: [],
                   retry_after_seconds: 2 }

Built / Running
    human_view   200  HTTP and WebSocket reverse-proxied to
                      runtime_node_uri:bound_port. WS upgrade is
                      mandatory — dev servers like Vite require
                      HMR; a hub that strips Upgrade headers
                      breaks the demo.
    agent_view   {
                   canonical: { state: "Built", ... },
                   runtime:   { state: "Running",
                                endpoints: [
                                  { path: "/", proxies_to:
                                    "http://<node>:<port>/" },
                                  { path: "/__hmr",
                                    proxies_to:
                                    "ws://<node>:<port>/__hmr" }
                                ],
                                started_at: ... },
                   ability_surface: ["webapp.stop"]
                 }

Built / Crashed
    human_view   503  crash reason summary, plus a "retry" link
                      that POSTs webapp.serve via the hub's
                      authenticated agent path.
    agent_view   { canonical: { state: "Built", ... },
                   runtime:   { state: "Crashed",
                                last_reason_hash: ... },
                   ability_surface: ["webapp.serve",
                                     "webapp.stop",
                                     "webapp.delete"] }

Failed / Stopped
    human_view   503  build-failure page; references the failure
                      reason blob.
    agent_view   { canonical: { state: "Failed",
                                last_reason_hash: ... },
                   runtime:   { state: "Stopped" },
                   ability_surface: ["webapp.build",
                                     "webapp.delete"] }

Deleted / *
    human_view   404
    agent_view   404  (URA no longer resolves)
```

Two properties this matrix is meant to make undeniable:

```
H1  agent_view's `ability_surface` is a function of (canonical,
    runtime) — not of an external policy file. RFC-006 main MUST
    let an ability declare the (state, runtime) preconditions
    under which it is callable, and the view layer MUST be able
    to ask "given current state, which abilities are admissible?"

H2  agent_view's `eal_hint` is what makes pages-and-apps a true
    AGENT-NATIVE web. A peer agent reading the URA gets, in one
    document, both the structured state AND a ready-to-paste EAL
    invocation that advances it. This is the form of "human-and-
    agent dual interface" the system is aiming for: not "JSON
    next to HTML", but "the JSON IS the affordance".
```

---

## B.8  Stress-derived invariants — RFC-006 main MUST satisfy these

The whole purpose of this appendix. Each invariant is followed by
a "Why surfaced" pointer back to the section that forced it.

```
TR-INV-1   Long-running transition completion semantics
           A canonical transition MAY emit zero or more progress
           events (as OperationalReceipts) before emitting exactly
           one terminal event. The terminal event is the sole
           CanonicalReceipt for the transition. Canonical state
           advances at that receipt and only at that receipt.
           Why surfaced: B.6 P1, P4. Without this, webapp.build's
           Building → Built advance has no atomic point.

TR-INV-2   Progress receipts MUST NOT enter the canonical chain
           A verifier replaying CanonicalReceipts alone MUST
           reconstruct the same canonical_state_hash a verifier
           with full receipts reaches. Loss of any subset of
           OperationalReceipts MUST NOT affect canonical
           correctness.
           Why surfaced: B.6 P2. Without this, build.progress
           loss across a network partition would corrupt the
           state machine.

TR-INV-3   Schema-level canonicality marking
           Every field of every state object's schema MUST carry
           an explicit `canonical: bool` annotation. Implementations
           MUST refuse to compute canonical_state_hash over fields
           tagged canonical:false, AND MUST refuse to omit fields
           tagged canonical:true.
           Why surfaced: B.2 vs B.3. Without this, an implementor
           who forgets to exclude bound_port produces non-
           deterministic hashes; one who forgets to include
           manifest_hash produces hashes that miss real changes.

TR-INV-4   Owner authority over delegated execution
           Every CanonicalReceipt MUST carry the owner_agent's
           signature. A node executing a transition on behalf of
           another owner_agent (delegated execution) MUST obtain
           and embed the owner's signature before the receipt is
           valid. A delegated execution that produces a receipt
           without owner signature is operational-only and MUST
           NOT advance canonical state.
           Why surfaced: B.1 places owner_agent in state_key for
           exactly this reason; B.5.1 makes signature required
           on every canonical transition. Build executed on a
           remote build farm is the concrete delegated case.

TR-INV-5   Runtime overlay is observable but not canonical
           Subscribers MUST be able to subscribe to runtime
           overlay transitions independently of canonical
           transitions. A runtime crash (Running → Crashed) MUST
           NOT mutate canonical_state_hash. RFC-006 main MUST
           expose two distinct subscription channels (or one
           channel with a typed discriminator that consumers can
           filter on without consulting the body).
           Why surfaced: B.4.2, B.5.2. Without this, an
           implementation could accidentally couple "the dev
           server crashed" to "the app is now Failed", which is
           wrong: Failed means the build failed, not the runtime.

TR-INV-6   View ≠ State
           Views are pure projections of (canonical_state ∪
           runtime_state). No view computation MAY mutate any
           state. Two views derived from identical state MUST
           be deterministic functions of that state and MUST
           NOT consult side channels (network, time-of-day,
           load, locale) for their output content. (View
           CACHING may consult time; view CONTENT may not.)
           Why surfaced: B.7. Without this, the HTML body the
           hub returns becomes a hidden second source of truth
           and replay diverges from live observation.

TR-INV-7   Multi-view as first-class
           A state_type MAY declare N named views, with N ≥ 2
           required for any state_type intended for hub
           exposure. RFC-006 main MUST mandate at least:
             - human_view  : HTTP-renderable (HTML, image,
                             stream).
             - agent_view  : structured, machine-actionable.
                             MAY embed EAL fragments. MUST
                             include an explicit ability_surface
                             listing transitions admissible from
                             current state.
           Both views MUST derive deterministically from the
           same (canonical, runtime) pair without side-channel
           input. Implementations MUST NOT serve the same byte
           sequence to a human GET and an agent GET — the agent
           channel is structurally distinct, not a content-
           negotiation header on the human channel.
           Why surfaced: B.7 H1, H2. This is the invariant that
           makes EasyNet's web layer agent-native rather than
           "a web app with a JSON sidecar".

TR-INV-8   Cross-state-space composition
           A state_type MAY declare multiple parallel state
           spaces. The canonical + runtime overlay split in this
           appendix is the minimal case; future state_types MAY
           add more (e.g., review_state, billing_state). RFC-006
           main MUST give an explicit cross-product admissibility
           specification format and MUST require implementations
           to reject transitions that violate the cross-product
           matrix at receipt-validation time, not at apply time.
           Why surfaced: B.4.3. Without this, an implementation
           could naively model webapp as a single state space
           Created/Building/Built/Running/Stopped/Crashed/Failed
           — which conflates a runtime crash with a build
           failure and re-introduces the bug TR-INV-5 forbids.

TR-INV-9   Concurrent canonical transitions on one state_key
           If two canonical transitions on the same state_key
           are submitted concurrently, RFC-006 main MUST specify
           one of:
             (a) strict serialization — at most one in-flight
                 canonical transition per state_key,
             (b) optimistic concurrency — receipts carry the
                 pre_state_hash they expected, and a receipt
                 whose pre_state_hash does not match the
                 current state is rejected.
           Either is acceptable; silence is not. (b) is preferred
           because it preserves liveness under network partition.
           Why surfaced: the X cells of B.4.3, specifically
           Building/Running being forbidden, only make sense if
           "build while serving" cannot race "stop while
           building".

TR-INV-10  Content-as-blob boundary
           Any state object field whose semantically-meaningful
           value is large (e.g. HTML payload, build artifact,
           image bytes) MUST be represented in canonical state
           as a content hash, with the bytes themselves living
           in a content-addressed blob store external to the
           state object. RFC-006 main MUST give the blob store
           interface (put, get, exists, ref-count or GC rules)
           once at the protocol level so all appendices inherit
           it.
           Why surfaced: B.2's build_artifact_hash. Without this,
           Appendix A (pages) and Appendix B (web_app) would
           each invent their own blob model and diverge.

TR-INV-11  URA-receipt traceability
           Every canonical state object MUST be addressable by a
           URA derivable from its state_key alone, via a stable
           function specified in RFC-006 main. Every CanonicalReceipt
           MUST be retrievable by either:
             (a) the URA of the state object it advances, OR
             (b) the receipt's own globally-unique receipt_id.
           A verifier given a URA MUST be able to enumerate the
           full ordered CanonicalReceipt history of that state
           object up to a chosen version, without consulting any
           channel other than the receipt log addressed by that URA.
           Why surfaced: this is the property pre-stateful EasyNet
           offered at INVOCATION granularity — one call is traceable
           end-to-end. The stateful model promotes the property to
           OBJECT granularity — one state object's entire life is
           traceable end-to-end. RFC-006 main MUST NOT silently drop
           the property; it is the protocol's audit story, and it is
           the answer to "how do I prove this page said X at version
           N, and the change to Y was authored by owner Z at time T".
```

---

## B.9  What Appendix B is NOT

To prevent scope creep:

```
- B is not a webapp product spec. It does not specify:
    * which build tool (vite, next, parcel) is supported;
    * how source is uploaded (git push, archive upload, IPFS);
    * how the hub does TLS, rate limiting, or auth;
    * what the human_view's CSS looks like.
  These are implementation choices outside the RFC.

- B is not Appendix A. Appendix A (easynet.page) presents the
  simple case of a pure-canonical state object with no runtime
  overlay. B is deliberately the harder case because main needs
  to be tested against the harder case.

- B is not a commitment to ship webapp. It is a commitment that
  IF webapp is shipped, it will be shippable consistently with
  RFC-006 main as constrained by §B.8.
```

---

## Closing note

If a future RFC-006 main draft conflicts with §B.8, the appendix
wins and main must be revised. The appendix is the contract; the
main body is the abstraction the contract requires. This is the
opposite of the conventional ordering, and it is intentional:
EasyNet has paid the cost, more than once, of drafting abstractions
first and discovering at integration time that they did not survive
the hard cases.

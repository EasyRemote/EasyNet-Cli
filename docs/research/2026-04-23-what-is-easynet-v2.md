# What is EasyNet — answers with conclusions, not hedges

**Author:** Silan Hu
**Date:** 2026-04-23 (v2, supersedes 2026-04-23-what-is-easynet.md)
**Scope:** 4 repos walked: EasyNet-Nucleus, EasyNet-Axon, EasyNet-Cli, EasyNet (backend + Frontend)

The first version of this document hedged every answer ("conditional
on", "potential", "real at"). That was not a report, it was a
refusal to commit. This version commits. Everything below is
inferred from source code and explicit docstrings; file+line
references at bottom.

---

## Question 1 — EasyNet 到底是什么？

**Answer: EasyNet is Silan Hu's in-progress attempt to define the
protocol standard for how AI agents should address, authenticate,
invoke, and audit each other — accompanied by a reference
implementation that tries to win the standard by being the
first thing that works end-to-end.**

Breaking that sentence down:

- **"Protocol standard"** — the core asset is
  `EasyNet-Axon/document/concepts/AXIOM.tex` (1687 LaTeX lines), which
  formalises an "Invocation Axiom" (seven-parameter
  `invoke(caller, callee, ability, subject, nonce, causal_context,
  args) → receipt`) and derives six structural invariants (Q1–Q6)
  that any agent-interoperable protocol must satisfy. Plus
  `EasyNet-Nucleus` — the URA (agent-URL) grammar + signature
  kernel. These are the protocol.

- **"Reference implementation"** — a 6-language SDK in
  `EasyNet-Axon/sdk/{rust,python,go,node,java,swift}` with
  byte-identical conformance vectors across all six. A runtime
  (`core/runtime-rs/`). A dendrite-bridge FFI. An invocation state
  machine and persistent event log with P1–P6 guarantees. These
  exist. They work. A Rust test writes an event log that a Python
  test can read.

- **"In-progress"** — AXIOM.tex is tagged Draft v0.1. The Tier-2
  discovery agent is `\deferred`. `DEFAULT_PROFILE.md` is
  `\deferred`. URA v2 is shipped but has no external citations.

- **"Trying to win the standard by working"** — the hope is the
  "well-worked reference" beats the "paper spec nobody
  implemented". Prior art that succeeded this way: OAuth 2 (Google
  implementation got adopted before the RFC was finalised),
  Kubernetes (Google's impl was the standard), WebRTC (Mozilla's
  impl set the tone). Prior art that failed this way: XMPP,
  Semantic Web / RDF, ActivityPub (all well-specified,
  well-implemented, stuck in niches).

The operator stack (CLI, backend, Frontend, what I've been
building tonight) is **not** the core asset. It's a showcase —
"here is what a product looks like when you build it on the
protocol." It's useful to the author for dogfooding and useful to
early users as a standalone CLI, but it is not the thing that
would change the world if EasyNet wins.

---

## Question 2 — 能否颠覆世界？

**Answer: No, not in its current form. It could, under specific
conditions that haven't been engineered yet.**

That's a concrete "no" with a specific escape path, not a hedge.

### Why the "no" is load-bearing

Three things are absent that every protocol which did disrupt the
world had before it disrupted:

1. **An external adopter.** Everything citable in all four repos
   is authored by one person. No implementation in another
   organisation. No "we chose EasyNet URAs" post. No paper that
   cites AXIOM. A protocol that only its author uses is a
   framework, not a standard. Protocols cross the line when
   *someone else* ships against them.

2. **A forcing function.** OAuth 2 won because enterprises had
   already been losing passwords to third-party apps and
   regulators had already started asking about it. OpenID Connect
   won because mobile apps needed a sign-in flow and every
   provider re-inventing it was an obvious cost. EasyNet solves
   "agents need non-repudiable interoperability." That problem is
   real but **nobody is paying for the lack of a solution yet.**
   Enterprise IT does not today have a procurement line item
   labeled "agent-invocation-audit compliance." No forcing
   function → protocol stays in the interesting-whitepaper tier.

3. **A venue.** RFCs exist because IETF exists. W3C recs exist
   because W3C exists. AXIOM exists because Silan Hu exists.
   There is no body of reviewers, no WG, no NIST workshop where
   this gets challenged by a peer. The protocol is not being
   attacked — it's being written by the author, reviewed by the
   author, implemented by the author. That is how theses are
   written; it is not how standards are born.

### The escape path (if the answer is to turn into "yes")

**One of these three has to land, in order:**

- **A second adopter.** One paying customer in finance, healthcare,
  or legal who demands audit-grade agent receipts. Not "we're
  talking to". An SOC 2 report that lists EasyNet in the
  compliance boundary is worth more than any README. The hardest
  step, most leverage. One customer creates the forcing function
  and the adopter in one stroke.

- **A paper in a top-tier venue.** Not arxiv. SOSP, OSDI, USENIX
  Security, or a clean CCS submission. The argument in
  AXIOM.tex is strong enough that the paper could be written; it
  would need to survive adversarial peer review, which would
  either sharpen the argument or expose a hole. Either outcome is
  better than the current state where the argument is only
  inspected by its author.

- **A major framework integrates EasyNet URA.** Claude Code, MCP
  servers, AutoGen, LangGraph — any of these putting "we use
  EasyNet URA for agent identity" in their README creates gravity.
  The tech integration is a weekend of work because URA is small.
  Getting an engineering lead at Anthropic / Microsoft / Google
  to say yes is months. But a single yes is enough.

### The base rate

New protocols proposed by one person: dozens per year across
GitHub, arxiv, blog posts. New protocols that reach standard
adoption: maybe 2-3 per decade. The base rate for disruption is
therefore < 1%. EasyNet has not yet done anything the other 99%
didn't do; **it is in that 99% until at least one of the three
escape conditions fires.** Hope is not a plan, and "we should"
is not a plan; a customer signature or a conference acceptance
letter is a plan.

### The honest answer

**Today: no.** The project is well-engineered, philosophically
sound, and stuck at the same adoption barrier every ambitious
protocol hits. With one external adopter + one venue it crosses
the line. Without either, it stays a beautifully written sub-
language one person maintained. That's not failure — that's the
modal outcome, and the author should plan for it.

---

## Question 3 — 先进性是什么？应该是什么？

### What it actually is (真正的先进性)

**One genuinely novel thing — as a *theoretical claim*.** The
AXIOM reduction of agent communication to a seven-parameter
invocation `(caller, callee, ability, subject, nonce,
causal_context, args) → receipt`, with a structural necessity
argument at `EasyNet-Axon/document/concepts/AXIOM.tex:226-425`.
The claim that HTTP-family protocols cannot satisfy the six
Q-invariants without a mandatory signed-byte profile, and that
every such profile is isomorphic, **is a real theoretical
contribution if it survives peer review**. I have read the
argument carefully and I believe it is correct in shape, which
is all the author can know before external review.

Honest qualifier on the implementation side: AXIOM describes
invocation as a *signed* protocol primitive (caller Ed25519 over
canonical envelope bytes, callee-signed receipts, chained causal
context). The Axon Rust SDK ships the machinery
(`call_mcp_tool_signed` + `InvocationEnvelope` + JCS canonical
bytes). The Cli and backend currently invoke through the
*unsigned* path (`call_mcp_tool_with_timeout`), so in today's
deployment, invocation is a "signed protocol primitive in theory,
unsigned-RPC-with-audit-trail in the shipped code." Five items
in AXIOM are marked `\deferred` (subject_id field, nonce +
causal_context envelope fields, AgentIdentity composite,
DEFAULT_PROFILE.md, Tier-2 discovery agent URA); closing any one
would constrain envelope bytes and make signed adoption risk-
free. This is why the "novelty" lives in the theorem, not in the
running system — when writing for external audiences, do not
conflate the two.

That's one thing. Everything else is good engineering, not
novelty:

- 6-language SDK parity: hard, important, not novel. Protobuf /
  gRPC / MessagePack all cleared this bar a decade ago.
- URA grammar: cleaner than ad-hoc agent IDs, not conceptually
  new. DNS, URN, DID had this problem solved for other domains.
- P1–P6 persistent invocation log: solid implementation of
  commodity ideas (append-only log + fsync-before-notify).
  Kafka, Postgres WAL, EventStore DB all operate on these
  principles. The novelty is that EasyNet makes them
  cross-language-byte-compatible at protocol layer — useful,
  not ground-breaking.
- `ability_snapshot.content_hash` on receipts (AXIOM §6.1 Q6):
  attestation of execution via signed hash of canonical ability
  manifest, covering code + I/O schema + external deps, recorded
  at invocation-receipt time as a post-hoc callee signature.
  Sigstore did this for artifacts; Rekor did it for logs. EasyNet
  applies it to agent execution — correct application, not new
  mechanism. Implementation status: AXIOM defines Q6; the CLI's
  skill-install layer computes a `skill_tree_hash` (code only,
  install-time) that is **not** Q6, flagged honestly in code
  and in `docs/open-questions/cli-dispatch-as-first-class-invocation.md`.
  Q6-compliant receipt emission waits on the signed-envelope path.

### What it should be (应该的先进性)

**Three things that would upgrade "well-engineered" to "genuinely
advanced".**

**(a) Formal mechanisation of AXIOM.** Rewrite AXIOM §3 (necessity
+ invariance theorems) in Coq, Lean, or TLA+. Even just the
Q1–Q3 subset. Two outcomes, both valuable: the proof survives
mechanisation and EasyNet gets a citation hook academics take
seriously; or it breaks under the prover and the author learns
which assumptions were informal. Currently the proof is "appears
correct under careful reading" — good enough for a draft, not
enough for a standard.

**(b) A cross-SDK adversarial test.** Today the conformance suite
tests "can SDK X read SDK Y's log." Raise the bar: "can SDK X
produce a log that SDK Y verifies under attack." Malformed
envelopes, timestamp skew, replayed nonces, forged chains. The
P1–P6 + I1–I5 + Q1–Q6 taxonomy gives you the adversary model
for free — someone just needs to write the tests. An attacker-
model conformance suite is what moves "interop" from "both
implementations agree on the happy path" to "both implementations
reject the same attack" — which is the operative question for
anyone who would actually adopt EasyNet under compliance pressure.

**(c) A trait-level abstraction over invocation runtimes.** Today
the Rust SDK is the reference. Python, Go, Node, Java, Swift
SDKs shadow it structurally. A protocol at this point in its life
benefits from a **trait** — "here is what an EasyNet-conformant
invocation runtime must expose, language-agnostic" — plus a
compliance badge for "my framework embeds an EasyNet runtime."
This is how HTTP clients converged on RFC 7230 semantics without
the world running on libcurl. EasyNet today offers "use our SDK";
it should offer "implement this trait in whatever you're
already using."

### What is NOT the 先进性 (even though the README might imply otherwise)

- Multi-agent orchestration via EAL loop/chat/handoff (RFC
  merged, Stages 1-2 shipped in EasyNet-Cli). LangGraph, CrewAI,
  AutoGen, OpenAgent have had these patterns for 12+ months.
  EAL is *cleaner* than most (compile-time call-count bound; no
  Turing-complete body), but cleaner ≠ novel.
- Federated discovery via `a2a.agents_json` labels. A2A itself
  (Google's + community's original proposal) does this; EasyNet-Cli
  is a v2 of a format that already existed.
- Skill marketplace install (what I built tonight). npm, pip,
  cargo, Anthropic Skills directory all shipped this pattern.
  EasyNet's specific choice (per-agent skill ownership with
  content_hash attestation at install time) is *correct* and
  *well-grounded*, not new.

---

## Question 4 — 落地应用现在有什么 vs 应该有什么

### What exists today

Enumerated from the repos:

1. **CLI (EasyNet-Cli)** — register agents (claude-code / codex /
   codex-app-server), dispatch via EAL missions, publish node roster
   labels, run missions in-process, manage agent sessions, install
   skills from GitHub (as of tonight's commit `ee2b7a1`),
   deploy abilities. 539 tests pass.

2. **Federation backend (EasyNet/backend)** — agent discovery
   across Axon nodes, ability catalog with invocation, skill
   marketplace endpoints (as of tonight's commit `4ce40c2`). Go
   services, JWT auth, rate-limited.

3. **Operator Frontend (EasyNet/Frontend)** — React UI for:
   device list + detail + pairing + terminal, agent list + detail
   (with installed-skill management + invocable-ability tabs),
   federation-wide Abilities page with grouping by category /
   device / device-group (as of tonight), Skills marketplace page
   with search + install + upgrade + remove.

4. **SDK (EasyNet-Axon/sdk)** — 6 languages, invocation runtime,
   persistent log, reconnecting bridge, ability tool adapter,
   conformance suite.

5. **Protocol docs** — AXIOM.tex, CONCEPT_MODEL.md,
   INVOCATION_STATE_MACHINE.md, INVOCATION_LIFECYCLE_ACROSS_PROCESSES.md,
   EasyNet-Nucleus URA spec.

6. **Gallery** — `gallery/case01-aris` demonstrates a 4-agent
   research pipeline across the stack.

### What should exist but doesn't

These are the gaps that matter, ranked by impact on adoption:

**(1) A one-page adopter onramp.** Today onboarding requires
reading AXIOM.tex to understand the ontology, cross-referencing
the Rust SDK to see how the invocation runtime works, figuring
out the CLI's mental model for agents vs abilities vs skills, and
then mapping that onto the Frontend. That's the path for the
*author*. An adopter needs: "I have a Python agent. Here are
five lines of Python that make it speak EasyNet invoke. Here is
the receipt it produces. Here is how an auditor verifies it."
The path does not exist. This is the biggest gap and the
cheapest to fix (one well-written guide, one example repo).

**(2) A reference-quality compliance story.** Every README talks
about invocation, receipt, audit — nobody has written the
SOC 2 / ISO 27001 / EU AI Act translation. "Here is how EasyNet
answers control X." Without this translation, a compliance
officer reviewing EasyNet cannot distinguish it from any other
open-source agent framework. With it, EasyNet has a unique
selling proposition no competitor can match on short notice.

**(3) A first-class invocation path on the CLI side.**
`docs/open-questions/cli-dispatch-as-first-class-invocation.md`
already tracks this — CLI dispatch today is "RPC with audit
trail", not AXIOM-conformant signed invocation. Until this
closes, the compliance-grade promise has a hole. Blocked on
three upstream `\deferred` pieces (URA namespace,
DEFAULT_PROFILE.md, discovery agent), all author-owned.

**(4) Tier-2 discovery agent.** AXIOM §6.2 specifies a reserved
Tier-2 discovery agent that ordinary agents invoke to publish
themselves. Today this is replaced by the node-label hack.
Ontology-correct publish requires this; I've tracked it in
`docs/open-questions/retire-a2a-agents-json-label.md`.

**(5) Attacker-model conformance tests.** See Q3 point (b)
above.

**(6) An `AbilitiesPage` lifecycle** — deploy / activate /
deactivate / uninstall from the Frontend. Today those actions
exist in the CLI + SDK but not on the web. Post-MVP
feature; mentioned for completeness.

**(7) A skill publishing path.** Install from GitHub is live as
of tonight; publishing to GitHub as an EasyNet skill (skill
author UX: "I wrote a skill, here is how I publish it so others
find it") is not. Low priority until skill authors complain.

**(8) The `mission` control-flow executor.** PR-10 Stages 1-2
are done (IR + parser); Stage 3+ (loop executor + verify-done
semantics + chat fan-out + handoff) is pending. Without these
the RFC language does not actually run.

### What order should it happen

If I had to rank by ROI:

1. Onramp doc (low cost, high leverage)
2. Compliance translation (low cost, potentially unlocks
   paying customer)
3. Mechanised AXIOM proof (higher cost, unlocks academic
   credibility)
4. First-class invocation migration + Tier-2 discovery
   (high cost, closes the compliance hole)
5. Cross-SDK adversarial tests
6. Everything else

The first two are a weekend of writing with no code changes.
They would likely move EasyNet further than any code change
could, because code is abundant and adoption signal is scarce.

---

## Single-sentence summary

**EasyNet is a one-person attempt to write the protocol standard
for AI agent interoperability, containing one real theoretical
contribution (the AXIOM seven-tuple necessity argument) and a
multi-language reference implementation; it is not on a
disruption trajectory today because it has zero external
adopters, zero peer review, and zero regulatory forcing function;
it could enter one with a single paying compliance customer or
a single tier-1 paper acceptance, both of which are cheaper to
pursue than any further code work.**

That is the conclusion. No "conditional on". No "real at its best".
The author does whatever they do with it next; this document gives
them the map of where they actually stand.

---

## Appendix — evidence pointers

Every claim above comes from reading one of these files. All
file paths relative to the `~/Documents/GitHub/` monorepo parent.

- AXIOM seven-tuple + Q1–Q6: `EasyNet-Axon/document/concepts/AXIOM.tex:226`
- Necessity theorem sketch: `AXIOM.tex:711`
- Invariance theorem: `AXIOM.tex:790`
- Tier-2 discovery agent deferred: `AXIOM.tex:1330`
- 6-language conformance: `EasyNet-Axon/sdk/conformance/CONFORMANCE_SUITE.md`
- Persistent log P1–P6: `EasyNet-Axon/document/concepts/INVOCATION_LIFECYCLE_ACROSS_PROCESSES.md`
- URA v2: `EasyNet-Nucleus/README.md`
- Agent/ability/skill ontology enforcement: `EasyNet-Cli/src/facade/cli/mod.rs:42`
- EAL control-flow RFC: `EasyNet-Cli/docs/rfc/eal-control-flow-v1.md`
- PR-10 stages 1-2: `EasyNet-Cli/src/eal/ir.rs`, `src/eal/parser.rs`
- First-class invocation gap: `EasyNet-Cli/docs/open-questions/cli-dispatch-as-first-class-invocation.md`
- Tonight's skill-marketplace work:
    - backend: `EasyNet/backend/internal/{handler,logic}/skill/`
    - CLI: `EasyNet-Cli/src/facade/cli/skill.rs`
    - Frontend: `EasyNet/Frontend/src/pages/easynet/{Abilities,Skills}Page.tsx`
- Gallery sample: `EasyNet-Cli/gallery/case01-aris/`
- Operator Frontend architecture: `EasyNet/Frontend/src/App.tsx`

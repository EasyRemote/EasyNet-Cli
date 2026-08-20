# Delegation chain — research note and future-work record

Status: research note, not a spec. Nothing here is authoritative; RFC-001
(`document/rfcs/001-authority-binding-relation-evidence.md` in EasyNet-Axon)
is the authoritative source for `AuthorityBinding` shape.
Date: 2026-08-19.
Scope: what we learned trying to build multi-hop delegation for EasyNet, why
it was rolled back, what the real blocking question is, and what part of this
might be paper-shaped.

## Why this note exists

On 2026-08-19 we tried to extend EasyNet-Cli's `DelegationAuthorityClaims`
with `max_depth` + `re_delegate()` to support a two-hop demo (Alice → Agent A
→ Agent B → Tool C). The code worked, compiled, passed 6 new unit tests — and
was rolled back the same day after finding EasyNet-Axon's RFC-001, frozen the
same day, which independently confirmed the implementation was premature and
named it "dead scaffolding" by archaeology. This note exists so the next
person (including a future instance of this session) does not repeat the same
sequencing mistake: designing a re-delegation mechanism before the more basic
question underneath it is answered.

## The original motivation (why chain delegation matters at all)

The problem EasyNet is trying to solve is not "support delegation" — every
OAuth-adjacent system does that. It's specifically:

> An agent invocation is issued by some `caller`, exercises some `authority`,
> that authority was vouched for by some `issuer`, and the whole chain from
> root principal to the executing agent must be reconstructable and
> verifiable *after the fact* from the receipt alone — not just admissible at
> invocation time.

Concretely, the demo we were building toward (see conversation record,
2026-08-19) was:

```text
User Alice
   |
   v (delegates)
Agent A
   |
   v (re-delegates, narrower scope)
Agent B
   |
   v (invokes)
MCP Tool C
   |
   v
external service

=> Receipt should let a verifier answer:
   - who ultimately authorized this (Alice)
   - through which agents (A, then B)
   - under what narrowed constraints at each hop
   - was the narrowing ever violated (scope widening, expiry extension,
     depth overrun)
```

Plus one adversarial case: Agent C steals a delegation token signed for
Agent B and tries to use it — must be rejected because the signed claim
bytes bind the intended delegatee, not because of a separate stored field
that could be swapped.

This shape is not EasyNet-specific speculation. A same-day (2026-08-19)
literature check found at least three active IETF individual drafts
converging on the same invariant independently:

- `draft-niyikiza-oauth-attenuating-agent-tokens` — `par_hash` chains each
  hop cryptographically to its parent; `I4` enforces strict capability
  monotonicity (child ⊆ parent) across tools/budget/domain/expiry.
- `draft-prakash-aip-00` (Agent Identity Protocol) — Biscuit-based
  Invocation-Bound Capability Tokens; Block 0 = root authority, Blocks
  1..N-1 = delegation blocks (delegator, delegatee, attenuated capability),
  Block N = completion block (result_hash, verification_status). This is
  close to a published version of the receipt shape we were sketching.
  Individual draft, no verifiable production reference implementation.
- `draft-mw-oauth-actor-chain-00` — explicitly patches the gap that RFC 8693's
  nested `act` claim is "informational only": it records prior actors but
  never defines how the delegation path is *validated* end to end.

None of these are adopted standards. All are individual drafts from 2026.
Conclusion at the time: the *problem* is real and currently unsettled
industry-wide, not solved-elsewhere-therefore-skip. But that does not mean
EasyNet should invent its own attenuation calculus from scratch either — see
"Open question" below.

## What we actually built, and why it was wrong to build it then

`DelegationAuthorityClaims::re_delegate()` (rolled back, see
`src/daemon/ability/authority/mod.rs` history) attempted:

```text
child.issuer  = parent.caller        // chain linkage
child.scope   ⊆ parent.scope         // attenuation
child.window  ⊆ parent.window        // attenuation
child.max_depth < parent.max_depth   // depth budget, signed into payload
```

This mirrors the AIP/AAT attenuation shape almost exactly. It is not wrong as
math. It was wrong as *sequencing*, for a reason RFC-001 states precisely:

> Two independent questions... 1. **Issuer authenticity** — is this signature
> really from `evidence.issuer`? ... 2. **Issuer authority** — is
> `evidence.issuer` actually *entitled* to vouch for *this specific*
> `binding.authority`? This is NOT a protocol-level fact — it depends on
> realm-specific ownership/trust policy... `evidence.issuer ==
> binding.authority` is **not** required by the SDK layer.

Our `re_delegate()` silently assumed a fourth, unstated axiom on top of the
two RFC-001 names explicitly:

```text
3. Transitivity — a delegatee who has been validly granted authority may,
   by virtue of holding that grant, become an issuer of a narrower grant to
   someone else.
```

Nothing in the current protocol establishes (3). It is not implied by (1) or
(2). `max_depth` presumes the *mechanism* of transitivity (a depth counter)
without the system ever having decided the *policy* of transitivity (who is
allowed to become an issuer, under what realm rule, and what — if anything —
constrains what they can then grant beyond "less than what they were
granted"). Shipping `max_depth` would have made a policy commitment
(delegation is a strictly narrowing tree, depth is the right attenuation
axis, a bare grant-holder automatically gets re-issuance rights) via API
surface instead of via a reviewed design decision. RFC-001 caught this by
archaeology (grepped for callers, parent-proof fields, and admission
consumption of `max_depth` — found none) and explicitly scoped chain
semantics out of v1.

## The corrected dependency graph

Before this session, chain delegation was implicitly treated as sitting
directly under "delegation" as a single next step. The actual dependency
graph has one more layer than that:

```text
signature authenticity        (RFC-001: generic SDK prove_authority)
        |
issuer authority               (RFC-001: realm-specific, e.g.
        |                       verify_delegation_issuer_authorized in
        |                       admission_facade.rs — NOT a wire-format
        |                       concept, deliberately)
        v
delegation semantics           (RFC-001: relation + evidence, single-hop
        |                       DelegatedBy, issuer MAY != authority)
        v
re-delegation / attenuation /  (NOT YET DESIGNED. Explicitly out of scope
chain verification              in RFC-001. This is where max_depth /
                                 re_delegate() tried to live — one layer
                                 too deep, too early.)
```

## The actual open question for a future chain RFC

Not "how deep should delegation chains go" or "what's the right attenuation
field shape" — those are downstream mechanism questions. The blocking
semantic question, unanswered by any current EasyNet doc:

> **When may a delegatee legitimately become the issuer of another
> delegation?**

This has to be answered as realm/policy semantics (analogous to how
`verify_delegation_issuer_authorized` already answers "may this issuer vouch
for this authority" for single-hop `DelegatedBy`) before any wire shape
(parent-proof linkage, attenuation fields, depth, cycle prevention,
revocation) is designed. Designing the wire shape first is exactly the
mistake made and reverted today.

## Acceptance checklist for declaring single-hop delegation closed

Per discussion 2026-08-19, before anyone (human or agent) opens a chain-RFC
conversation, the following three items must be verified against whatever
RFC-001 migration lands in EasyNet-Cli's `admission_facade.rs` (in progress
as of this note, not yet verified — the migration was mid-flight and did not
compile when checked):

1. `evidence.issuer` authenticity verification is fully independent of
   `binding.authority` — i.e. the code never silently requires
   `issuer == authority` as a shortcut for "signature is valid."
2. Daemon admission has one clear, single location that answers "is this
   issuer authorized to speak for this authority" (RFC-001 names
   `verify_delegation_issuer_authorized` as the existing candidate) — not
   scattered/duplicated checks that could disagree.
3. The `relation` + `evidence` composition explains every currently-supported
   single-hop case (`Self_`+`Identity`, `DelegatedBy`+`Delegation`,
   `SessionOf`+`Session`) without needing an implicit `issuer == authority`
   or `subject == authority` shortcut anywhere in admission.

Only once these three close should "is chain delegation worth designing" be
revisited — and even then, start from the semantic question above, not from
an API.

## Possible research-shaped angle (not vetted, flagging only)

The literature check found real IETF activity (AAT, AIP, actor-chain) around
multi-hop attenuation *mechanism*, but none of it centers RFC-001's
authenticity/authority split as a first-class distinct layer — the
AIP/AAT chain designs bundle "was this signed by X" and "was X allowed to
sign this" into one verification pass keyed on the signing chain itself. If
that observation holds up under closer reading (it has NOT been verified
against the full AIP/AAT spec text beyond the summaries fetched 2026-08-19),
the possible distinct contribution is not "yet another attenuation token
format" but making *issuer authority* — the realm-policy question of who may
vouch for whom — a first-class, separately-verifiable layer in a multi-hop
chain, rather than folding it into signature verification. Whether this is
actually novel relative to AAT/AIP/actor-chain, and whether it survives
adversarial scrutiny, is unexamined. Flagging only; not a claim.

## Sources checked 2026-08-19 (search-summary depth only, not full spec reads except AIP)

- `draft-niyikiza-oauth-attenuating-agent-tokens-01` (IETF individual draft)
- `draft-mw-oauth-actor-chain-00` (IETF mailing list submission)
- `draft-prakash-aip-00` (IETF individual draft; full text fetched and
  summarized — see conversation record for field-level detail)
- `draft-nelson-agent-delegation-receipts` (IETF individual draft; covers a
  *different* trust boundary — user-to-operator consent provenance, not
  agent-to-agent authority chaining; complementary, not overlapping)
- RFC 8693 (OAuth 2.0 Token Exchange) — the `act` claim baseline these drafts
  extend
- SPIFFE/SPIRE delegated-identity docs — established prior art for
  attenuation-never-expands as a norm, no chain-receipt equivalent found

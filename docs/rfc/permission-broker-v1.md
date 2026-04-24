# Permission Broker v1 — Approval Broker, **Not** Capability Security

> Plan v10.1–v10.2. Pins the language so a reviewer cannot read
> "permission" in the codebase and assume capability security.

## 1. What v1 permission *is*

Interactive human approval before a sensitive agent action. The
broker pauses dispatch, publishes a `PermissionPrompt` to any
subscriber (local Client UI or a cross-machine UI observer), and
blocks the ongoing Invocation until a decision arrives.

This is a **UX feature** — it helps a human stay in the loop
during autonomous agent runs. It is not a security mechanism.

## 2. What v1 permission *is not*

v1 makes no promise of:

1. **Non-transferability.** A decision cannot be referenced by a
   subsequent Invocation as proof of prior authorisation.
2. **Cross-machine trust.** A decision made on a remote UI and
   transmitted over Axon is advisory; the `subject_host` node's
   local broker is the authoritative decider (see §4).
3. **Concurrency-strict allow_once.** Best-effort under
   concurrent pending requests; two near-simultaneous decisions
   on the same pending id are not guaranteed to be ordered.
4. **Audit-grade attribution.** v1 records the decision in the
   trace log; there is no signed record of who made it at what
   time. v2 signed invocation fixes this.

These are **not bugs in v1**; they are the consequence of v1
choosing "approval broker" over "capability security".

## 3. v1 shape

```rust
pub trait PermissionBroker {
    fn ask(&self, ctx: AskContext) -> Decision;
}

pub struct AskContext {
    pub prompt: String,
    /// Reserved for v2 signed invocation (AXIOM §6.3).
    /// v1 always None.
    pub capability_claim: Option<CapabilityClaim>,
}

pub enum Decision { Allow, Deny, AllowOnce }
```

Implementations:
- `AllowAllBroker` — default; preserves pre-PR behaviour (every
  ask returns `Allow`). Active whenever no subscriber is
  listening on `system.permission.subscribe`.
- `SubscriberBroker` — engages when at least one subscriber is
  listening. Publishes a `PermissionPrompt` and blocks on the
  decision RPC.

`requires_permission` is a boolean on the ability manifest. v2
replaces it with `required_capability: Vec<CapabilityRequirement>`;
v1 field is tolerated and treated as "always ask" when true.

## 4. Cross-machine decision semantics (advisory downgrade)

The plan v10.2 pins this explicitly because reviewers keep asking
about it.

- **Local case** (agent on node A, broker on node A, UI on node
  A) — full approval broker. The broker's decision is the
  runtime's decision.
- **Cross-machine case** (agent on node A, UI on node B) — the
  Client UI on B contributes an *advisory decision*; the final
  authority remains the local broker on A. v1 default policy is
  to accept B's advisory as the decision, but this is a policy
  choice, not a protocol guarantee. A hardened deployment can
  configure A's broker to ignore B's advisory.

Why? Because v1 has no way to verify that B's decision was
produced by the authorised human:
- `Invocation.caller_signature` is empty → no cryptographic
  binding between decision and identity;
- no receipt chain links the decision to later Invocations.

**Pretending B's decision is a v1 "permission grant" would leak
the trust boundary across the network.** v2 closes this with
signed invocation.

## 5. v2 evolution

- `AskContext.capability_claim` starts carrying a signed
  capability envelope.
- `requires_permission: bool` deprecates; new ability manifests
  declare `required_capability: [...]`.
- Remote decision + its signature + remote policy evaluation →
  advisory becomes grant.
- The `PermissionBroker` trait surface does not change; its
  internals grow the verification step.

## 6. Reviewer checklist when "permission" appears in a PR

- [ ] Is the language "broker" / "approval" (OK) or "grant" /
      "capability" (NOT OK in v1)?
- [ ] Does the code depend on any of the four v1 non-guarantees
      above? If yes, it belongs to v2.
- [ ] Does the PR populate `capability_claim`? If yes, reject —
      v1 always leaves it `None`.
- [ ] Does the PR treat a cross-machine decision as a grant? If
      yes, reject — it is advisory in v1.

# Open Question — Skill Marketplace Integration

**Status:** Open, customer surfaced · **Revisit:** when a concrete spec / user story lands · **Owner:** Silan Hu · **Date:** 2026-04-23

## The customer need (as stated)

> "这个 skill 你可以管理并且能搜索，来源来自 OpenSkill 或其他现有的 marketplace 源"

Concretely: an operator opens the EasyNet Frontend (or CLI), **searches** a skill by name / capability / tag across one or more upstream **marketplaces** (OpenSkill, Anthropic's skills directory, a future EasyNet-native skills registry), **browses** what's returned (description, author, dependencies, version history), **installs** a chosen skill into a specific agent's on-disk `skills/` directory, and later **upgrades** / **removes** it. The operator does not want to `git clone` by hand or copy folders into `~/.claude/skills/` manually.

This is a *product* feature — the marketplace layer is above AXIOM's concerns. AXIOM §6.1 (Q6) defines how a skill's identity is pinned inside a receipt (`ability_snapshot.content_hash` over the canonical ability manifest); it says nothing about where skills come from or how they are distributed. Marketplace integration is the CLI / backend / Frontend's job.

## Why skill ≠ ability here (re-stating the encapsulation boundary)

Earlier conversation threads settled that:

- **Skill** = an agent's private implementation asset. Cross-agent reuse goes through "agent wraps skill as ability" (see `src/facade/cli/mod.rs:48` and `CLAUDE.md` / `ARCHITECTURE.md`).
- **Ability** = the network-visible contract. Invoked via `<agent>.<ability>`; observable in `a2a.agents_json[*].skills[*]` (despite the field name, those entries are abilities, not skills).

Marketplace integration is about **skills**, not abilities. A marketplace ships skill bundles (code + SKILL.md + dependencies); an agent installs them privately; if the agent wants to expose one of them on the network, it declares an `ability.toml` that wraps the skill. No marketplace publishes abilities directly in this design — ability publishing is the separate (deferred) Axon §6.2 discovery-agent path.

## Retract the earlier retract

On 2026-04-22 I deleted task #12 "PR-A2 skill install `<owner>/<repo>`" with the reason "zero customer evidence." That retract was correct *at the time* — the customer was a plan-inertia bullet, not a real ask. The customer has now surfaced explicitly in this session. Reopening the question does not undermine the "ground-first" discipline; it means the trigger actually fired.

## What would need to be decided before coding

### 1. Which marketplace(s)

Candidate sources, as named by the customer request:

- **OpenSkill** (exact URL / API TBD — the customer used the term, I have not grounded it to a specific registry yet)
- **Anthropic `~/.claude/skills/` convention** — today a local-only directory; Anthropic has hinted at a remote registry but nothing public yet
- **GitHub repos** as a lowest-common-denominator source (owner/repo, tag, path)
- **A future EasyNet-native skill registry** (deferred; would need its own AXIOM-aligned design)

Each source has a different shape:

- OpenSkill (presumed): some HTTP API, some manifest schema, some auth story
- Anthropic: filesystem convention, no remote protocol
- GitHub: git URL + SHA + subpath, no search API unless we crawl
- EasyNet-native: subject to Axon discovery-agent design, which is itself still deferred

The minimum viable path is **one concrete source first, interface stays open for more**. My recommendation is GitHub first because:
- No new auth story (`gh` or anonymous API works).
- No third-party protocol to track for breaking changes.
- Every skill author publishes to GitHub anyway, so coverage is strictly a superset of any curated marketplace.
- Search is limited (GitHub search is keyword not semantic), which is an acceptable v1 constraint.

**Open decision.** I have not verified OpenSkill exists as a concrete product with a public API. The customer said "OpenSkill or other existing marketplace sources" — the disjunction suggests they are sketching a direction, not specifying. Before writing adapter code I need either the marketplace's docs or an explicit "just do GitHub for now."

### 2. Skill manifest format

The current `skills/easynet-ability-author/SKILL.md` is Anthropic's freeform-markdown convention. A marketplace wants something parseable:

```toml
# skill.toml (proposed)
name = "code-reviewer"
version = "1.2.3"
description = "..."
author = "..."
license = "Apache-2.0"
content_hash = "sha256-..."   # AXIOM §6.1 Q6 binding
dependencies = [...]          # other skills this one needs
compatible_runtimes = ["claude-code", "codex"]
```

Open question: do we invent our own manifest, or mirror whatever OpenSkill / the chosen marketplace already ships? Inventing locks us into a migration when marketplaces converge; mirroring ties us to the specific source's format. Best compromise is probably **store the canonical manifest as-fetched-from-source + compute `content_hash` ourselves** at install time, so AXIOM Q6 still applies even if the source's format is idiosyncratic.

### 3. Install target semantics

Three candidates for where a downloaded skill lives:

- **`~/.claude/skills/`** — Anthropic's global convention, one copy shared across all Claude Code sessions.
- **`<agent-root>/skills/`** — per-agent install (matches `AgentDirectory` layout; PR-3b.1 designed it for this).
- **Shared CLI cache `~/.easynet/skills/<name>@<version>/`** + symlinks into `<agent-root>/skills/` — download once, install to multiple agents cheaply. Handles upgrade consistency.

Recommendation: **shared cache + symlink**. Per-agent installs without duplication. Matches how package managers (npm, cargo) handle it. Requires a small amount of lockfile discipline to prevent stale cache + broken symlink states.

### 4. Frontend integration

Two legitimate UX paths:

**(a) SkillsPage — federation-wide skill catalog.** Search, browse, install. Server-side rendered list of what's available in each configured marketplace source. Install button POSTs to backend → backend tells the CLI (via what channel?) → CLI downloads + caches + symlinks. This is the "app store" mental model.

**(b) Per-AgentDetailPage skill tab.** On an agent's detail page, show installed skills + an "add skill" button that opens a marketplace picker. Same backend path, different UX entry point.

Both are useful; (b) is cheaper because the agent context is already on-page.

**Open:** how does Frontend talk to CLI? Today Frontend → backend → Axon node → CLI. Does "install skill on agent X" work as an Axon invocation to the CLI node? That's AXIOM §5 pattern-matching. Or is it a backend-mediated shell call (which breaks the "CLI is an Axon node" invariant)? Needs design.

### 5. Axon binding

A skill is installed to agent X on machine M. Once installed, can agent X's receipts attest the installed version via `ability_snapshot.content_hash`? Yes — that's exactly what AXIOM §6.1 Q6 specifies. But it requires:

- The CLI computes `content_hash` at install time.
- When agent X executes an ability, the CLI stamps the receipt with the `content_hash` of whichever skill was invoked.

This means skill marketplace integration **couples to the AXIOM first-class-invocation migration** tracked in `cli-dispatch-as-first-class-invocation.md`. Without that migration, receipts don't exist; without receipts, `content_hash` has nowhere to land. Marketplace v1 can ship without the AXIOM binding (skills get installed and used like today, no receipts) — but we should decide whether to *record* the hash at install time so the AXIOM binding is a future addition, not a migration.

Recommendation: **v1 records `content_hash` in a local index (`~/.easynet/skills/index.json`) at install time** even though it isn't used by receipts yet. Negligible cost, future-proofs.

## What would move this to a plan item

1. **A concrete marketplace target named and grounded** — either "OpenSkill's public API is at https://… and its manifest format is …", or "just GitHub for v1."
2. **A one-page spec** for the four open decisions above (marketplace protocol, manifest format, install target, Frontend API). Lives in `docs/spec/skill-marketplace-v1.md` when written.
3. **A first user story** concrete enough to verify: "Alice opens EasyNet Frontend, searches 'code-review', clicks install on `anthropic/code-reviewer@1.2.3`, and expects it to show up under her `claude-code` agent's skill list within 10 seconds." That story is what the tests reference.

Until all three exist, marketplace integration stays open. The existing `easynet skill-install` (bundled-only, hardcoded list of one skill) continues to work and can be superseded later.

## What does NOT belong here

- **Invoking skills directly from Frontend** — architecturally wrong, violates encapsulation invariant (`facade/cli/mod.rs:48`). Skills are private; ability is the public surface.
- **Publishing skills to a marketplace from Frontend/CLI** — upload path is separate from install path; most operators only consume, not publish. Defer.
- **Skill dependency resolution across upstream versions** — npm-style conflict resolution is its own engineering project. v1 assumes dependencies pin exact versions (or have none).

## Related open questions

- `cli-dispatch-as-first-class-invocation.md` — receipts, where `ability_snapshot.content_hash` lands. Skill marketplace can ship before this but should record the hash at install time so the later binding is zero-migration.
- `retire-a2a-agents-json-label.md` — same deferral block (AXIOM §6.2 discovery-agent). Ability publishing and skill installing are two different actions; the former blocks on discovery agent, the latter does not.

## Log

| Date       | Event                                                                           |
|------------|---------------------------------------------------------------------------------|
| 2026-04-22 | Task #12 "PR-A2 skill install owner/repo" deleted as plan-inertia (no customer). |
| 2026-04-23 | Customer surfaced in session. Opened this file. Retract of the retract.         |
| —          | Revisit: when (1) marketplace target grounded, (2) spec written, (3) user story.|

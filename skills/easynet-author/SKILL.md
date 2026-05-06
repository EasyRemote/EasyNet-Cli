---
name: easynet-author
description: Create reusable EasyNet abilities, skills, and EAL workflows — author by hand or via device.mission.think auto-curation. Use when the user asks to package a workflow, save a lesson, write an ability/skill/EAL mission — OR when YOU notice you've solved the same problem twice, learned a non-obvious rule, written a multi-step recipe by hand, or hit a tool gap that should be permanent rather than re-discovered next time.
allowed-tools: [Read, Write, Edit, Bash, mcp__easynet]
---

# EasyNet Author

Take a one-shot insight and turn it into something reusable: a published ability (deterministic or LLM-fulfilled tool), a skill (LLM context that activates by description), or an EAL workflow that composes existing abilities. Three paths. Pick by what kind of thing you're saving.

## When This Skill Activates

### User-prompted triggers

- The user says "create an ability for X" / "write an EAL mission" / "package this as a tool"
- The user says "remember this for next time" / "save this lesson"
- The user describes a recipe with concrete inputs/outputs and asks to formalize it
- The user gives a working-style correction or convention they want you to follow going forward
- The user asks "how does ability/skill/EAL work?" (this skill answers + you keep the conversation here)

### Self-prompted triggers — activate this skill yourself when YOU notice

- You just chained ≥3 ability calls (discover, fetch, parse, output) by hand. The chain is a Path A candidate (EAL exec ability).
- You solved the same kind of problem in this conversation that you solved earlier in another. Either path A (deterministic) or path C (let judge decide).
- The user gave you a non-obvious correction that overrules a default ("don't summarize at the end", "always lift env-reads to boot"). That's path B (skill).
- You hit a niche knowledge piece — a CLI flag order, a workspace convention, a fact-from-experience that wasn't in CLAUDE.md and isn't derivable from `git log`. Path C — let `device.mission.think`'s judge decide niche-ness.
- A long task wraps up and a "this would help future-me" feeling lingers. Path C.
- **Anti-trigger** (do NOT activate): the lesson is a single trivial fix already in the diff, the lesson is in CLAUDE.md, the lesson is "remember the user's name" (use memory, not skill).

## Process

### 1. Decide which kind of thing to author

Use this flowchart. Each path is described in detail in its own section below.

```
A reusable lesson exists. What is it?
│
├── A deterministic recipe with clear input/output JSON
│   (e.g. "fetch weather for <location>", "compose summarize+tag+save")
│   → Path A: hand-author an ability
│
├── LLM judgement / style / convention with no fixed I/O
│   (e.g. "always check the env first", "don't mock the database")
│   → Path B: hand-author a skill (Anthropic SKILL.md)
│
├── Not sure — emerged from a real task and you can't tell ability vs skill
│   → Path C: device.mission.think auto-curate
│
└── Multi-step recipe that calls several existing abilities
    → Path A with [exec] kind = "eal"
```

If the user gave an explicit instruction ("create an ability") — follow it. If you're self-triggered and uncertain, default to Path C.

### 2. Path A — hand-author an ability

Abilities are typed verbs other agents (and EAL programs) can invoke. They live in `<agent-root>/abilities/<verb>.ability.toml`.

**Choose an exec kind**:

| kind | when |
|---|---|
| `shell` | runs a subprocess via argv (curl, jq, awk, python script). Optional sandbox profile. |
| `http` | one in-process HTTP request, templated URL/headers/body, response capped. |
| `eal` | composes other abilities into a workflow. The body is an EAL `mission "x" { … }` program. |
| (no `[exec]` block) | LLM-fulfilled — the agent's chat handler synthesizes the response. Use when there's no deterministic recipe. |

**Author** the manifest (templates: `references/ability-toml-templates.md`):

```toml
schema_version = "1"
name = "weather"
description = "Fetch the current weather for a location."
[input_schema]
type = "object"
required = ["location"]
[input_schema.properties.location]
type = "string"
description = "City name."

[exec]
kind = "shell"
sandbox = "net_only"
argv = ["curl", "-s", "https://wttr.in/{{ location }}?format=3"]
```

**Validate** before publish:

```bash
easynet ability validate <agent-root>/abilities/weather.ability.toml
```

This catches: bad `schema_version`, missing required keys, unknown `[exec].kind`, EAL-source parse failure, EAL references to abilities that don't exist in this agent's catalog.

**Publish** — three options:

```bash
# Option 1: drop the file directly + refresh
cp my-ability.toml ~/.easynet/workspaces/claude/abilities/weather.ability.toml
easynet agent refresh

# Option 2: invoke device.ability.publish (works in-process, programmatic)
easynet ability invoke device.ability.publish --args '{
  "owner_agent_id": "claude",
  "manifest_toml": "<full TOML body>"
}'

# Option 3: cross-device deploy (publishes to <node>'s workspace)
easynet ability deploy ./my-ability-dir/
```

**Verify**:

```bash
easynet agent abilities claude          # claude's catalog
easynet ability invoke claude.weather --args '{"location":"Beijing"}'
```

### 3. Path B — hand-author a skill

Skills are SKILL.md files Claude Code's loader scans on every chat. They activate by `description` matching against the running prompt — not by explicit invocation.

**The Anthropic-canonical structure is load-bearing**. A skill missing `## When This Skill Activates`, missing `description ending in "Use when …"`, or missing `allowed-tools` will land on disk but never activate. The validator catches this.

Template: `references/skill-md-template.md`. Required shape:

```markdown
---
name: <kebab-case-slug>
description: <what it does>. Use when <user-prompted trigger> — OR when YOU notice <self-prompted trigger>.
allowed-tools: [Read, Bash]
---

# <Title Case>

One paragraph purpose.

## When This Skill Activates

### User-prompted triggers
- Asks for X
- Mentions Y

### Self-prompted triggers
- You notice you're about to Z

## Process
### 1. ...

## Examples
### Example: <scenario>
**Input:** ...
**Expected Output:** ...

## Notes
- Edge cases.
```

**Publish**:

```bash
easynet ability invoke device.skill.publish --args '{
  "owner_agent_id": "claude",
  "skill_name":     "<slug matching front-matter name>",
  "skill_md":       "<full SKILL.md body>"
}'
```

For claude-code agents this lands at `<root>/.claude/skills/<name>/SKILL.md` — the project-local path Claude Code's skill loader scans. For codex agents it lands at `<root>/skills/<name>/SKILL.md` (codex has no native skill auto-loader; the file is reachable for ad-hoc reads).

**Verify activation** (the only way to confirm the skill works):

```bash
easynet agent send claude "<prompt that should trigger the new skill>"
# Look in the response envelope for tool_calls: [{"ability":"Skill","args":{"skill":"<name>"}}]
# That confirms Claude Code's loader matched the description and activated.
```

### 4. Path C — device.mission.think auto-curate

When you can't decide ability vs skill, or the lesson emerged from doing a task and you want a structured judge to decide, run `device.mission.think`. It spins three sessions: worker (does the task), judge (classifies any sinkable lesson), curator (writes device.ability.toml or SKILL.md if judge greenlights).

**Always start with `--dry-run`** — see what the curator would publish before letting it touch the workspace.

```bash
easynet mission think \
  --agent claude \
  --prompt "<the task or the lesson recap>" \
  --max-cycles 5 \
  --dry-run
```

Output envelope to read:

```json
{
  "termination_reason": "judge_terminate",
  "final_verdict": {
    "memory_type": "feedback",       // feedback | project | reference | user | none
    "scope":       "private",        // team → ability ; private → skill
    "what_to_save": "<one sentence>",
    "why":          "<reason>",
    "how_to_apply": "<when this kicks in>",
    "exclusion_check": { ... }
  },
  "curator": {
    "attempted":     true,
    "ok":            true,
    "dry_run":       true,
    "target":        "skill",
    "authored_body": "<the full SKILL.md or device.ability.toml that WOULD have been published>"
  }
}
```

If the verdict + body look right, drop `--dry-run` and rerun to actually publish. If not, refine the prompt and retry.

**Limitations** (real, observed):

- The third in-process chat call (curator) can hit a chat-handler stderr panic on some daemon states. Failure-soft path catches it; you'll see `termination_reason: "worker_error"` or `"judge_error"`. Workaround: restart the daemon (`easynet runtime stop && easynet runtime start`) and retry.
- `device.mission.think` is heavy (3 LLM calls per cycle). For trivial lessons, hand-authoring (path A or B) is faster.

## Examples

### Example: Path A — wrap a CLI as an ability (user-prompted)

**Input:**
```
User: claude, create an ability that fetches the weather for a city.
```

**Process:**
1. Decide: deterministic recipe, clear I/O → Path A.
2. Author `weather.ability.toml` with `[exec] kind = "shell"`.
3. Drop into `~/.easynet/workspaces/claude/abilities/`.
4. `easynet agent refresh`.
5. Test: `easynet ability invoke claude.weather --args '{"location":"Beijing"}'`.

**Output:**
```
✓ Published claude.weather. Test: claude.weather(location:"Beijing") → "Beijing: 18°C ⛅️ partly cloudy"
```

### Example: Path B — capture a working-style correction (self-prompted)

**Input:**
```
User (third time correcting): no, when reviewing code, ALWAYS lead with what's right before what's wrong.
```

**Process:**
1. Self-trigger fires: "user gave the same correction multiple times → save it as a skill".
2. Decide: LLM judgement, no fixed I/O → Path B.
3. Author SKILL.md following template.
4. `easynet ability invoke device.skill.publish` with the body.
5. Verify by next code-review prompt — Claude Code should activate the skill and lead with strengths.

### Example: Path C — emerged from a long task (self-prompted)

**Input:**
```
You just finished a 4-cycle debugging session that uncovered: "ulimit -n 256
on macOS quietly truncates large file scans; raise it before recursive
walks." It's niche — not in CLAUDE.md, not derivable from code, but
genuinely useful next time.
```

**Process:**
1. Self-trigger: niche knowledge + emerged from real work + uncertain about ability vs skill → Path C.
2. Run `easynet mission think --agent claude --prompt "I learned: ulimit -n 256 on macOS truncates large file scans silently. Acknowledge briefly." --max-cycles 2 --dry-run`.
3. Read verdict — judge says `memory_type: feedback, scope: private` (workflow preference, not team policy) → curator writes a SKILL.md.
4. Re-run without `--dry-run` to publish.
5. Verify: next big-walk prompt should activate the new skill.

## Notes

- **Reserved verbs**: `chat` cannot be published or unpublished — it's the agent's baseline. The validator rejects this.
- **EAL exec abilities** must reference real verbs in the catalog. The validator runs the EAL parser and rejects member-calls (`<agent>.<verb>`) where `<agent>.<verb>` isn't in the owner's published catalog.
- **Skill activation is by description**. A SKILL.md missing `## When This Skill Activates` or with a `description` lacking "Use when …" will land on disk but never be picked up. `validate_authored_skill` enforces this.
- **Don't publish trivial fixes**. The exclusion list (in `device.mission.think`'s judge prompt and in this skill's anti-triggers) covers: derivable from code, in git log, debug recipe for a single bug, ephemeral task state, already in CLAUDE.md.
- **Hot reload**: published abilities are picked up by the in-daemon dispatcher's fallback resolver immediately; `easynet agent refresh` propagates them to axon-runtime's catalog. Skills are picked up by Claude Code on the next chat call (no daemon restart required) because the loader scans the workspace on each subprocess spawn.

## References

- `references/ability-toml-templates.md` — full templates for the four exec shapes
- `references/skill-md-template.md` — Anthropic-canonical SKILL.md template (mirrors `~/.claude/skills/shared/skill-creator/skill-template.md`)
- `references/eal-grammar.md` — EAL syntax reference, member-call form, traditional `call ... on ...` form

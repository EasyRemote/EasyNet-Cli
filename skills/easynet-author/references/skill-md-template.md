# SKILL.md template (Anthropic-canonical)

Mirrors `~/.claude/skills/shared/skill-creator/skill-template.md` with EasyNet-specific call-outs.

## Required structure

```markdown
---
name: <kebab-case-slug>
description: <One sentence describing what the skill does.> Use when <user-prompted trigger 1>, <user-prompted trigger 2> — OR when YOU notice <self-prompted trigger>.
allowed-tools: [<minimal tool set>]
---

# <Title Case Skill Name>

One-paragraph description of what this skill does and its primary purpose.

## When This Skill Activates

### User-prompted triggers
- The user asks/says/mentions <X>
- The user explicitly addresses <Y>

### Self-prompted triggers
- You notice you're about to <Z>
- You catch yourself <W>

## Process

### 1. <First Step>
- What to do
- What to check

### 2. <Second Step>
- ...

### 3. Output Format
<How to present results.>

## Examples

### Example: <Scenario name>

**Input:**
```
<concrete input>
```

**Expected Output:**
```
<concrete output>
```

## Notes
- <Edge cases>
- <Limitations>
```

## Field rules — these are LOAD-BEARING

The Claude Code skill loader uses these to decide when to activate the skill. Get any of them wrong and the skill lands on disk but never runs.

### `name`
- kebab-case, lowercase, ASCII alnum + `-`/`_` only
- length ≤ 100 bytes
- MUST match the directory name (`skills/<name>/SKILL.md`)
- MUST match what `skill.publish` is called with

### `description`
- 1–2 sentences
- First sentence: what the skill does
- **Second part MUST contain "Use when …"** — this is the activation hint the loader matches against the running prompt
- Recommended: include both user-prompted and self-prompted clauses ("Use when X — OR when YOU notice Y")

### `allowed-tools`
- minimum needed for the skill to function
- common combinations:
  - read-only analysis: `[Read, Glob, Grep]`
  - code modification: `[Read, Write, Edit]`
  - full access: `[Read, Write, Edit, Glob, Grep, Bash]`
  - web research: `[Read, WebFetch]`
  - EasyNet ability calls: `[mcp__easynet]`

### `## When This Skill Activates`
- This exact heading is required (the validator checks it literally — match must be exact)
- Both subsections (User-prompted / Self-prompted) recommended; agents that can self-trigger are more useful than ones that only react to user phrases
- Each bullet specific. "Asks for X" beats "asks for things" — vague triggers either over-activate or under-activate.

## Validation (what `validate_authored_skill` checks)

```
✓ Body starts with `---` front-matter delimiter
✓ Front matter has `name:`, `description:`, `allowed-tools:`
✓ description (case-insensitive) contains "Use when"
✓ Body contains the literal heading `## When This Skill Activates`
```

Anything else (tone, examples, length) is best-practice but not enforced.

## EasyNet-specific notes

### Where it lands on disk

`skill.publish` for a `claude-code` agent writes to `<agent-root>/.claude/skills/<name>/SKILL.md`. For a `codex` agent writes to `<agent-root>/skills/<name>/SKILL.md`. The dispatch is done by `skills_dir_for(root, agent_type)` — operators don't pick the path manually.

`<agent-root>/.claude/skills/` is the **load-bearing** location for claude-code: that's where Claude Code's skill loader scans on every chat subprocess spawn. Earlier versions wrote to `<root>/skills/`, which kept the file but never activated the skill. Don't bypass `skill.publish` and write the file by hand to a different location unless you know which path the running runtime scans.

### Recall test

After publish, the only way to confirm the skill *activates* (not just exists) is to send a chat that should trigger it and look for the entry in the response envelope's `tool_calls`:

```bash
easynet agent send claude "<a prompt that should match the description's 'Use when'>"
```

Look for:
```json
"tool_calls": [
  {
    "ability": "Skill",
    "args":    { "skill": "<your-skill-name>" }
  }
]
```

If you don't see it: the description's activation hint didn't match. Refine and re-publish.

### Don't write a SKILL.md when

- The lesson is in CLAUDE.md or AGENTS.md already
- The lesson is derivable from the codebase (file paths, function names, conventions)
- The "lesson" is a debug recipe for one specific bug (use the commit message)
- The "lesson" is ephemeral task context (use a Plan or memory, not a skill)
- The user asked you to remember a personal preference about THEM (use memory, not a skill)

The judge in `mission.think` enforces these as exclusion checks. When hand-authoring, apply them yourself.

## Naming conventions

| ✅ | ❌ |
|---|---|
| `code-reviewer` | `CodeReviewer` (PascalCase) |
| `rust-test-runner` | `rust_test_runner` (snake_case — for ability slugs but not skill slugs) |
| `easynet-collaborate` | `easynet collaborate` (whitespace) |
| `cargo-test-single-invocation` | `cts` (too abbreviated to be greppable) |

The skill slug appears in `tool_calls` envelopes and in directory listings. Operators read it; pick names that read well.

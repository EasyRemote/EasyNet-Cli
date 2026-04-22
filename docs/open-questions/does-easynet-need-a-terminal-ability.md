# Open Question — Does EasyNet need a terminal ability?

**Status:** Open · **No revisit trigger yet** · **Owner:** Silan Hu · **Date:** 2026-04-22

## Why this is an open question, not a plan item

Earlier drafts of the EasyNet-Cli implementation plan listed "terminal" under 不排期堆积 as a future feature. I cannot now produce evidence for why it was listed:

- No RFC, ontology section, AXIOM section, or open question in either `EasyNet-Cli/docs/` or `EasyNet-Axon/document/` names "terminal" as a requirement.
- No GitHub issue, user request, or Alive-product use case I can locate calls for a terminal ability.
- `rg terminal` in the CLI repo returns zero match outside incidental occurrences (ANSI terminal rendering, tty detection for styling).

That said, a terminal-like ability is plausibly useful:

- **An agent running on device A exposes a shell** so an operator on device B (or another agent) can execute a command over the federation.
- Alive or another product wants a "give me a shell into this agent's box" affordance.
- Mission debugging wants to drop into a subprocess shell on the executing node.

Each of these is speculation. None is grounded in a current request.

## What would move this to a plan item

Any one of the following:

1. **A concrete product ask**: Alive or a downstream consumer opens a feature request naming the use case and the required shape of the API.
2. **An AXIOM Tier 3b convention** naming a `terminal` or `shell` callee-side convention — analogous to how `chat` and `work` are named in AXIOM §7.3b.
3. **An RFC** in `EasyNet-Cli/docs/rfc/` or `EasyNet-Axon/document/rfcs/` that specifies the ability's input/output schema, security model (pty? command whitelist? rootfs isolation?), and streaming semantics.

Without one of the three, this stays open. The CLI does not grow a placeholder `src/terminal/` module and `easynet terminal` does not appear as a CLI verb in anticipation.

## If it becomes a plan item

The obvious shape is: add a `terminal.ability.toml` to an agent directory that wants to expose one, with an `input_schema` that carries `{ command: string, stdin?: string, timeout_seconds?: u32 }` and an output schema carrying `{ stdout, stderr, exit_code }`. But that shape is derivable at the moment the need appears; pre-committing to it now would be designing without a consumer.

## Log

| Date       | Event                                                                       |
|------------|-----------------------------------------------------------------------------|
| 2026-04-22 | Extracted from the CLI plan's "不排期堆积" bucket after a ground-first audit found no customer for the item. |

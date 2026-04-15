# Case 01: ARIS — Autonomous Research in Sleep

> Mapping a real-world multi-agent research system to the EasyNet ontology.

**Note:** This directory contains two iterations. The current ontology-aligned
design lives in `agents/`, `missions/`, and the non-prefixed `abilities/`
(e.g., `abilities/review/`). The older `aal/`, `eal/`, `runtime/`, and
`aris-*` prefixed abilities are from a prior prototype iteration and are
retained for reference only.

## What This Case Study Demonstrates

ARIS (Auto-Claude-Code-Research-in-Sleep) is a research automation harness
with 64 markdown skills, 5 MCP servers, and a cross-model adversarial
collaboration protocol. It orchestrates the full ML research lifecycle:
idea discovery, experiment implementation, iterative review, and paper writing.

This case study re-expresses ARIS using the EasyNet ontology — AAL for agent
definitions, EAL for mission orchestration, and ability.json for deployable
endpoints. It serves as:

1. **Ontology validation** — if the ontology cannot elegantly express ARIS,
   the ontology has a gap
2. **Migration template** — how to convert a "pre-EasyNet" multi-agent system
   into the native structure
3. **AAL/EAL design exploration** — concrete usage of both languages before
   their formal specification is locked

## Architecture

```
                          +---------------------------------------------+
                          |           EAL Missions (External)           |
                          |  research-pipeline.eal                      |
                          |  idea-discovery.eal                         |
                          |  auto-review-loop.eal                       |
                          |  paper-writing.eal                          |
                          |  nightmare-audit.eal                        |
                          +--------------------+------------------------+
                                               | compiles to Mission IR
                                               | dispatches via gRPC
                    +--------------------------+------------------------+
                    |                          |                        |
          +---------v--------+  +--------------v-----+  +---------------v-------+
          | silan/researcher |  | openai/reviewer    |  | silan/paper-writer    |
          |                  |  |                    |  |                       |
          | public:          |  | public:            |  | public:               |
          |  lit_survey      |  |  review            |  |  plan                 |
          |  idea_generate   |  |  review_reply      |  |  write_section        |
          |  novelty_verify  |  |  rule_on_rebuttal  |  |  generate_figures     |
          |  experiment_     |  |  adversarial_audit |  |  compile              |
          |   bridge         |  |                    |  |  improve              |
          |  analyze_results |  | private:           |  |                       |
          |  rebuttal        |  |  suspicion_        |  | private:              |
          |                  |  |   tracking         |  |  citation_discipline  |
          | private:         |  |  review_           |  |  latex_repair         |
          |  citation_       |  |   calibration      |  |  venue_formatting     |
          |   discipline     |  |  code_reading_     |  |  narrative_structure  |
          |  experiment_     |  |   strategy         |  |                       |
          |   integrity      |  +--------------------+  +-----------------------+
          |  gpu_deployment  |          |
          +------------------+  +-------v-----------+
                    |           | google/reviewer   |  (alternative reviewer)
                    |           |                   |
                    |           | public:           |
                    |           |  review           |
                    |           |  visual_review    |
                    |           +-------------------+
                    |
          +---------v------------------------------+
          |  Device Layer (hosting substrate)       |
          |  GPU servers, edge devices              |
          |  Not network-first-class (ontology S6)  |
          +----------------------------------------+
```

## Ontology Mapping

### ARIS Concepts -> EasyNet Concepts

| ARIS (before) | EasyNet (after) | Ontology |
|--------------|----------------|----------|
| SKILL.md (64 files) | Split into **abilities** (public) + **skills** (private) | S4.1 |
| /skill-name invocation | `agent.ability(...)` in EAL | S7.2 |
| research-pipeline chain | `mission "aris-research-pipeline" { ... }` | S5 |
| MCP server (gemini-review) | Agent with abilities (google/reviewer) | S4.1 |
| Cross-Model Protocol | Encapsulation invariant -- structural, not conventional | S4.4 |
| REVIEWER_MEMORY.md | Reviewer agent's private memory graph | S3.1 |
| Artifact contracts (IDEA_REPORT.md -> ...) | Ability input/output schemas | S2.1 |
| REVIEWER_DIFFICULTY levels | Ability parameter (difficulty: medium/hard/nightmare) | S4.3 |
| Codex MCP thread_id | review_reply ability with thread_id parameter | S7.2 |

### What stayed as skills (private)

These ARIS skills have no cross-agent value -- they are internal behavioral
constraints or knowledge:

| Skill | Belongs to | Why private |
|-------|-----------|------------|
| citation-discipline | researcher | Internal quality control |
| experiment-integrity | researcher | Self-imposed behavioral rules |
| writing-principles | researcher, paper-writer | Internal writing standards |
| suspicion-tracking | reviewer-gpt | Must be opaque to executor |
| review-calibration | reviewer-gpt, reviewer-gemini | Internal scoring strategy |
| latex-repair | paper-writer | Implementation detail |
| gpu-deployment | researcher | Infrastructure knowledge |

### What became abilities (public)

These ARIS skills have network-wide value -- other agents need to call them:

| Ability | Agent | Former ARIS skill |
|---------|-------|------------------|
| lit_survey | researcher | /research-lit |
| idea_generate | researcher | /idea-creator |
| novelty_verify | researcher | /novelty-check |
| review | reviewer-gpt | auto-review-loop Phase A |
| review_reply | reviewer-gpt | auto-review-loop round 2+ |
| adversarial_audit | reviewer-gpt | nightmare mode codex exec |
| plan | paper-writer | /paper-plan |
| write_section | paper-writer | /paper-write |
| compile | paper-writer | /paper-compile |

## Directory Structure

```
gallery/case01-aris/
|
+-- README.md                           <-- This file
|
+-- agents/                             <-- AAL: producer side (how agents are defined)
|   +-- researcher/
|   |   +-- agent.aal                   <-- Agent class: abilities + skills
|   |   +-- skills/
|   |       +-- citation-discipline.md  <-- Private skill
|   |       +-- experiment-integrity.md <-- Private skill
|   +-- reviewer-gpt/
|   |   +-- agent.aal
|   |   +-- skills/
|   |       +-- suspicion-tracking.md
|   +-- reviewer-gemini/
|   |   +-- agent.aal
|   +-- paper-writer/
|       +-- agent.aal
|
+-- abilities/                          <-- Deployable ability definitions
|   +-- chat/ability.json               <-- Default callable (ontology S7.1)
|   +-- lit-survey/ability.json
|   +-- idea-generate/ability.json
|   +-- novelty-verify/ability.json
|   +-- review/ability.json
|   +-- review-reply/ability.json
|   +-- experiment-bridge/ability.json
|   +-- experiment-run/ability.json
|   +-- paper-plan/ability.json
|   +-- paper-write/ability.json
|   +-- paper-compile/ability.json
|
+-- missions/                           <-- EAL: consumer side (how abilities compose)
|   +-- research-pipeline.eal           <-- Full pipeline: W1 -> W2 -> W3
|   +-- idea-discovery.eal              <-- Workflow 1: idea -> novelty -> review
|   +-- auto-review-loop.eal            <-- Workflow 2: review -> fix -> re-review
|   +-- paper-writing.eal               <-- Workflow 3: plan -> write -> compile
|   +-- nightmare-audit.eal             <-- Dual-reviewer adversarial audit
|   +-- agent-send-sugar.eal            <-- Ontology S7.2 sugar demonstration
|
+-- shared-references/                  <-- Cross-agent protocols and conventions
    +-- reviewer-independence.md
    +-- cross-model-protocol.md
    +-- three-time-scales.md
```

## How the Two Languages Relate

```
 AAL (Agent Abstraction Language)          EAL (EasyNet Ability Language)
 ---------------------------------         ------------------------------
 PRODUCER side                             CONSUMER side
 "How an agent class is defined"           "How abilities compose to do a task"

 class Researcher {                        mission "research-pipeline" {
   public:                                   let landscape = researcher.lit_survey(...)
     ability lit_survey(...) -> ...          let ideas = researcher.idea_generate(
     ability review(...)    -> ...               landscape: landscape.output)
   private:                                  let review = reviewer.review(
     skill citation_discipline                   artifacts: ideas.output)
     skill experiment_integrity              }
 }
                                           External EAL can ONLY see abilities.
 BCC interprets AAL.                       EasyNet interprets EAL.
 They are two independent                  The bridge is the gRPC ability endpoint.
 language/runtime pairs.
```

**Key separation (ontology S5.2):**
- Writing a new agent does NOT require knowing EAL
- Writing a new mission does NOT require knowing AAL

## The Degeneracy Test

The ontology defines ability = skill + self-evolution + network-wide invocation.
Remove any one component and the object degenerates:

| Remove | Degenerates to | ARIS example |
|--------|---------------|-------------|
| Graph (self-evolution) | A skill | ARIS's 64 static SKILL.md files |
| Self-evolution | A frozen function | A fixed script that never learns |
| Network invocation | A local tool | A CLI command on one machine |

ARIS in its current form is a **pure skill system** -- all 64 skills are static
markdown without memory graphs or network-wide addressability. This case study
shows what it looks like when those two missing components are added.

## Running the Missions

```bash
# Register agents
easynet agent add researcher --type claude-code --tenant silan
easynet agent add reviewer   --type codex       --tenant openai
easynet agent add writer     --type claude-code --tenant silan

# Run a mission
easynet mission run gallery/case01-aris/missions/idea-discovery.eal

# Or use agent send sugar (ontology S7.2)
easynet agent send researcher "What's the state of sparse MoE research?"
# desugars to: researcher.chat(prompt: "...")

# Inspect the mission
easynet mission list
easynet mission show <mission-id>

# Deploy abilities to devices
easynet ability deploy gallery/case01-aris/abilities/lit-survey/ --to gpu-server
```

## What the Ontology Buys ARIS

| Dimension | ARIS (before) | EasyNet (after) |
|-----------|--------------|----------------|
| **Discoverability** | Know the skill name to invoke it | `easynet ability list` -- network-wide discovery |
| **Cross-machine** | Bound to one Claude Code process | Any EasyNet node can `ability invoke` |
| **Evolution** | Edit SKILL.md, no memory | Ability graph evolves from traces |
| **Orchestration** | Markdown pipeline, Claude interprets | Compiled EAL mission: list, show, cancel |
| **Reviewer isolation** | Convention (can be violated) | Encapsulation invariant (structurally enforced) |
| **Multi-reviewer** | Manual MCP server switching | Two reviewer agents, same ability surface |

## Deferred Items (Ontology S13)

These are known gaps in this case study, matching the ontology's open recursion points:

1. **EAL loops/conditionals** -- The auto-review-loop mission is unrolled to 4 rounds.
   A future EAL extension with `while`/`if` would make this dynamic.
2. **Capability abstraction** -- The cross-process invocation interface between
   agent and device is not yet specified.
3. **Internal EAL trace format** -- The `ability_graph_traces` field exists but
   has no schema.
4. **Agent instantiation** -- Agents are implicit (provisioned by runtime + deploy).
   AAL/BCC would enable explicit `agent spawn`.
5. **AAL grammar** -- The `.aal` files in this case study use a pseudo-syntax
   inspired by the ontology's OOP view. The actual AAL grammar, type system,
   and inheritance model are unspecified.

# Skill Store Agent Aggregate Owner

## Goal

Route shared skill-package mutation owner resolution through the Agent aggregate instead of reopening `agents.json` in `resources::skills::store`.

## Expected Effect

- Architecture convergence: `skill.publish` and CLI/ability-backed `skill.install`, `skill.upgrade`, and `skill.remove` consume one registered-workspace projection.
- Effect convergence: install, upgrade, and removal preserve their command-specific missing-owner errors and their filesystem transaction behavior.
- Product acceleration: the skill store owns package mutation only; durable registered-Agent interpretation remains in its aggregate owner.

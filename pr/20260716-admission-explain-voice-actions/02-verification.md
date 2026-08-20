# Verification

- `cargo test -q admission_explain_projects_voice_actions_from_signed_descriptor_facts --lib`
- `cargo test -q admission_explain --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `git diff --cached --check`

## Delta

- Expanded `admission_explain_projects_voice_actions_from_signed_descriptor_facts`
  from a two-row spot check to the full voice action table:
  `invoke` for call mutations, `read` for call snapshots, and `stream` for
  subscribe/transcribe. This pins the intended owner rule: explain projection
  reads persisted signed descriptor facts, never a `voice.*` namespace
  heuristic.

# Decisions Log

## 2026-07-26

- Chose explicit provenance states over another string default.
- Kept `mission_run_id` optional to preserve public API compatibility.
- Chose `direct_publish:skill.publish` as the direct provenance source because
  it is generic runtime ability provenance and does not claim product lifecycle
  authorship.
- Added an architecture convergence guard that rejects restoring the retired
  hidden `mission.think` provenance fallback inside `publish_handler`.
- Kept `SkillSource` schema unchanged so existing install/list projections
  remain compatible while provenance semantics become explicit.

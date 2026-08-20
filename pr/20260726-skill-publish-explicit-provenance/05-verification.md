# Verification

Executed checks:

- `codegraph sync .` — OK, index already up to date.
- `codegraph status .` — OK, 1,034 files / 37,520 nodes / 144,946 edges.
- `codegraph explore SkillSource SkillPublishReceipt publish_skill` — identified
  publish/projection/store blast radius.
- `cargo test --features axon-pb publish_writes_skill_md_and_install_json -- --nocapture` — OK.
- `cargo test --features axon-pb publish_writes_codex_skill_to_runtime_project_dir -- --nocapture` — OK.
- `cargo test --features axon-pb publish_without_run_id_records_direct_publish_provenance -- --nocapture` — OK.
- `cargo fmt --check` — OK.
- `git diff --check` — OK.
- `bash tools/scripts/check-architecture-convergence.sh` — OK.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — OK.
- `bash tools/scripts/check-sdk-canonical-public-api.sh` — OK.

Observed unrelated check:

- `cargo test --features axon-pb publish_ -- --nocapture` hit broader
  `ability_management::publish` and CLI `agent publish` tests. Those failures
  pre-existed this seam and report non-canonical fixture registry keys such as
  `alice` instead of `default/alice`. The three targeted `skill.publish`
  provenance tests passed under exact filters.

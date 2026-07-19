# Verification

Planned checks:

- `tools/scripts/docker-media-bidi-e2e.sh --self-test`
- `tools/scripts/docker-media-bidi-e2e.sh --skip-build --keep`
- `cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo test runtime_descriptor_resolver_synthesizes_remote_system_ability_without_presence_probe --features axon-pb,remote-desktop -- --nocapture`
- `cargo test forwarded_remote_stream_times_out_without_terminal_event --features axon-pb,remote-desktop -- --nocapture`
- `cargo test managed_user_runtime_signer_signs_with_subject_bound_inventory_key --features axon-pb,remote-desktop -- --nocapture`

The browser tool should be used when the target is allowed by the browser URL
policy. In this environment, localhost and local file report URLs are rejected
by the in-app browser policy, so Docker/CLI report artifacts are the acceptance
evidence for this product path.

## Results

- `DOCKER_BIN=/usr/local/bin/docker PATH="/usr/local/bin:/opt/homebrew/bin:$HOME/.cargo/bin:$PATH" tools/scripts/docker-media-bidi-e2e.sh --self-test`
  passed.
- `DOCKER_BIN=/usr/local/bin/docker PATH="/usr/local/bin:/opt/homebrew/bin:$HOME/.cargo/bin:$PATH" tools/scripts/docker-media-bidi-e2e.sh --skip-build --keep --out-dir target/e2e/docker-media-bidi/codex-mutation-20260720-000021`
  passed.
- Report:
  `target/e2e/docker-media-bidi/codex-mutation-20260720-000021/report.md`.
- `cargo test runtime_descriptor_resolver_synthesizes_remote_system_ability_without_presence_probe --features axon-pb,remote-desktop -- --nocapture`
  passed.
- `cargo test forwarded_remote_stream_times_out_without_terminal_event --features axon-pb,remote-desktop -- --nocapture`
  passed.
- `cargo test managed_user_runtime_signer_signs_with_subject_bound_inventory_key --features axon-pb,remote-desktop -- --nocapture`
  passed.
- `DOCKER_BIN=/usr/local/bin/docker PATH=/Users/macbook.silan.tech/.cargo/bin:/usr/local/bin:$PATH tools/scripts/docker-media-bidi-e2e.sh --keep --out-dir target/e2e/docker-media-bidi/codex-terminality-20260720-002101`
  passed after rebuilding the Linux CLI artifact and runtime Docker images.
- Report:
  `target/e2e/docker-media-bidi/codex-terminality-20260720-002101/report.md`.
- `cargo check --features axon-pb,remote-desktop --bin easynet --bin easynet-daemon`
  passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  passed.
- `cargo fmt --check`
  passed.
- `git diff --check`
  passed.

The report's `mutation_facts` prove:

- stream record count: `2`;
- bidi record count: `2`;
- stream and bidi request ids are unique;
- stream and bidi invocation URAs are unique;
- all provider records preserve the expected provider callee URA and media
  ability URA;
- all receipt chains are verified;
- every stream and bidi receipt chain has exactly one completed terminal
  receipt; and
- every chain head hash matches that completed terminal receipt.

Additional 2026-07-20 removal terminality evidence:

- remove-after-discovery no longer accepts harness timeout `124` as success;
- removed stream route exits `1` with daemon `DeadlineExceeded`:
  `REMOTE_STREAM_TERMINAL_TIMEOUT`;
- removed bidi route exits `1` with typed `not wired on session.open` failure;
- provider catalog exposes neither `media.synthetic_stream` nor
  `media.synthetic_bidi` after removal; and
- FFI descriptor resolution now synthesizes deterministic runtime system
  descriptors for remote Device/Hub owners without probing owner presence,
  covering `invocation.history.list` descriptor resolution when the selected
  owner is offline.

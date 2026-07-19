# Verification

Planned checks:

- `tools/scripts/docker-media-bidi-e2e.sh --self-test`
- `tools/scripts/docker-media-bidi-e2e.sh --skip-build --keep`
- `cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

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

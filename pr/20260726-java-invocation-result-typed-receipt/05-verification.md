# Verification

Completed:

- `mvn -q -f sdk/java/pom.xml test` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — passed; 2 changed Java files synced.
- `/Users/macbook.silan.tech/.local/bin/codegraph status .` — index up to date.

# Verification

| Boundary | Success proof | Failure proof |
|---|---|---|
| Lock schema | Exact canonical document accepted | Unknown/missing keys, malformed revision/digest rejected |
| Axon checkout | Exact clean HEAD and contract match | Wrong/dirty checkout or contract drift rejected |
| Rust dependency | Cargo lock resolves locked Axon version | Version/path drift rejected |
| Python dependency | Constraint and uv lock resolve locked Axon version | Stale constraint/lock rejected |
| Workflow | Every referenced script exists; all checkout refs derive from lock | Hard-coded revision or missing script rejected |
| Candidate | Exact override revision runs full suite | Failure cannot update pinned coordinate |
| Artifact | Wheel/crate/binary install from no-path-source inputs | Editable/path-only success rejected |

## Reproduced baseline

- `uv lock --project sdk/python --check`: FAIL; `sdk/python/uv.lock` requires update.
- `cargo test --locked --features axon-pb --lib`: FAIL; 6297 passed, 117 failed, 10 ignored against Axon `2ad067dc` / `0.192.3`.

## Final evidence

- `cargo test --locked --features axon-pb --lib`: PASS; 6414 passed, 0 failed, 10 ignored against Axon `bf944455` / `0.192.3`.
- `cargo test --locked --features axon-pb --no-run`: PASS for all Rust unit and integration targets.
- `cargo fmt --all -- --check` and strict workspace Clippy: PASS.
- Go, Python, Node, Swift, and Java SDK suites: PASS.
- Lock, workflow-integrity, project-structure, canonical-public-API, package-metadata, RFC-001 baseline, and ability-model gates: PASS, including their self-tests where defined.

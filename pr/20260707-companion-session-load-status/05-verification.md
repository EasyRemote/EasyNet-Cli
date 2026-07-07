# Verification

Passed:
- `cargo test -q daemon::plugins::load_plan`
- `cargo test -q daemon::plugins::companion`
- `cargo test -q daemon::plugins::surface`
- `cargo fmt --check`
- `git diff --check`
- touched-file terminology audit for the forbidden address term

Known non-run items:
- No Linux tray service was launched; this slice only implements the load/status classification required before a Linux provider is implemented.

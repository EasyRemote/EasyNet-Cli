# Verification

Passed:
- `cargo test -q daemon::plugins::companion`
- `cargo test -q daemon::plugins::manifest`
- `cargo test -q daemon::plugins::load_plan`
- `cargo test -q daemon::plugins::surface`
- `cargo test -q daemon::plugins::install`
- `cargo test -q daemon::plugins::package`
- `cargo test -q plugin_host_install`
- `cargo fmt --check`
- `git diff --check`
- touched-file terminology audit for the forbidden address term

Known non-run items:
- Native macOS and Windows companion binaries were not launched in this slice; this change only redirects daemon observation to the planned status-file path that those apps already write.

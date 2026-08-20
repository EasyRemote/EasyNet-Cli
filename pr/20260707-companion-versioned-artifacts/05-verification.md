# Verification

Passed:
- `cargo test -q daemon::plugins::companion`
- `cargo test -q daemon::plugins::install`
- `cargo test -q plugin_host_install`
- `cargo test -q daemon::plugins::package`
- `cargo fmt --check`
- `git diff --check`
- touched-file terminology audit for the forbidden address term

Known non-run items:
- Native LaunchAgent and HKCU Run commands were not executed in this Linux/macOS-independent test slice; ownership logic is covered by deterministic path and target matching tests.

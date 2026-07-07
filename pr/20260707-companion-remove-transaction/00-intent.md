# Companion Remove Transaction

## Goal

Make desktop companion package removal an installer-owned transaction. The CLI must not stop/remove companion supervisor state before the package transaction has a rollback owner.

## Boundary

- `PluginInstaller` owns package state and package directory transactions.
- `DesktopCompanionManager` owns supervisor cleanup and desired-state cleanup.
- CLI command handlers invoke one transactional operation and render the result.

## Invariants

- Rollback paths are allocated before package state is mutated.
- Failed package removal restores the previous package state file.
- Failed package removal after companion cleanup attempts to restore the previous companion supervisor/desired state.
- A failed companion cleanup must not remove the active package record or directory.
- Successful companion removal stops/removes supervisor artifacts, removes desired state, removes the package directory, and removes the package state record.

## Verification

- Add tests for successful desktop companion remove through the installer transaction.
- Add tests that companion cleanup failure leaves package state and directory intact.
- Add tests that remove rollback path allocation failure leaves package state and directory intact.
- Run focused plugin install tests plus formatting/diff/terminology audits.

## Results

- Added successful desktop companion remove test through `PluginInstaller::remove_with_companion_manager`.
- Added companion supervisor cleanup failure test proving package state and directory stay active.
- Added package rollback allocation failure test proving package state and directory stay active.
- Added package remove failure after companion cleanup test proving companion supervisor/desired state is restored.
- `cargo fmt --check` passed.
- `cargo test -q daemon::plugins::install` passed.
- `cargo test -q plugin_host_install` passed.
- `cargo test -q daemon::plugins::companion` passed.
- `cargo test -q cli::commands::groups::plugin` passed.
- `cargo test -q companion_status_selection` passed.
- `git diff --check` passed.
- Touched-file terminology audit passed.

# Plugin control subject state convergence

## Goal

Remove message-sniffed credential compatibility from the plugin control
ability ingress. Plugin control should distinguish only explicit authority
states: paired device subject available, or no credential file yet. Existing
but malformed/incomplete credentials must fail closed instead of being treated
as "daemon/plugin control unavailable".

## Root abstraction problem

`src/cli/commands/groups/plugin.rs` currently calls `load_credentials()` and
then inspects error message text for `"no credentials found"` or
`"credentials file is incomplete"`. That couples product plugin control to
credential wording and collapses structurally different states:

- missing credentials: unpaired local product state, safe to report plugin
  daemon authority unavailable;
- malformed/incomplete credentials: corrupt authority state, must be surfaced
  and repaired, not hidden.

The daemon persistence layer already exposes the correct root abstraction via
`load_credentials_optional()`, where `None` means missing file and `Err` means
existing invalid state. Plugin control should consume that typed state instead
of reconstructing it from strings.

## Invariants

1. Missing credentials file remains non-fatal for plugin reload/status calls.
2. Existing malformed/incomplete credentials are errors.
3. Plugin control subject URA is derived only from validated credentials.
4. No string matching over credential error messages remains in plugin control.
5. Public CLI behavior remains compatible for unpaired machines.

## Boundary proof

```text
plugin command
  -> PluginControlSubject::resolve()
  -> config::load_credentials_optional()
     - None: Unpaired
     - Some(validated Credentials): Available(device URA)
     - Err: corrupt authority state, propagate
  -> LocalDaemonSystemAbilityIssuer only when Available
```

This preserves the daemon as the plugin control authority while removing a
product-level compatibility shim around credentials.

## Verification plan

- targeted plugin group tests:
  - missing credentials => unavailable subject
  - valid credentials => device subject
  - malformed/incomplete credentials => error
- grep gate preventing credential error string sniffing in plugin control
- `cargo fmt --check`
- `git diff --check`
- `check-canonical-runtime-convergence-v2.sh`
- `check-architecture-convergence.sh`


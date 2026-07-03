# Runtime Lifecycle Decisions

2026-07-03T00:00:00+08:00

Decision: place lifecycle source under `src/daemon/boot/lifecycle/` and
re-export it as `crate::daemon::lifecycle`.

Reason: the runtime lifecycle spec is behavioral, while
`project-structure-v1.md` is the layout authority. The structure guard rejects
new top-level daemon directories. `boot/` already owns boot sequencing and
shutdown handoff, so this placement preserves semantics and keeps the final
tree valid.

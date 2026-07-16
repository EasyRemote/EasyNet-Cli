# Invariants

1. `ability.publish` resolves owner roots through the aggregate repository path.
2. Test-only fixture setup may write the legacy registry directly, but that
   dependency must remain inside `#[cfg(test)]`.
3. Publish/unpublish wire names, input schema, output schema and errors remain
   compatible.
4. No production code path gains a new fallback to direct registry persistence.
5. This slice must not modify unrelated dirty files in the current worktree.

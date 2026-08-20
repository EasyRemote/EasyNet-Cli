Go lifecycle private alias prune
================================

Root fork
---------

The Go runtime lifecycle model has already moved to neutral `Runtime*` owner
names while public `Daemon*` aliases remain for source compatibility. A few
private helper functions still mirror the old daemon names even though all
callers have migrated to the runtime helpers.

Expected effect
---------------

This slice deletes obsolete private compatibility wrappers after migration. It
does not remove exported aliases or change public behavior.

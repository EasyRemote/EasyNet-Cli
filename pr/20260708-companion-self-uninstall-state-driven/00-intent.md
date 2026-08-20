# Intent

Make `easynet self uninstall` satisfy the desktop companion SPEC cleanup order
by driving companion cleanup from desired-state records instead of from the
currently loadable package index alone.

The package index remains the source for platform plans when a package is still
available. Desired-state records remain the enumeration source so uninstall can
also clean stale companion status files when a package record is missing or
malformed.

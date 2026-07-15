Boundary proof
==============

Removed private wrappers:

- `newDaemonHandle`
- `requireDaemonRuntimeReady`
- `daemonRuntimeReady`
- `validDaemonState`
- `wrapDaemonTransportError`

Each wrapper was definition-only and delegated directly to the canonical
runtime helper. Keeping them made the internal lifecycle boundary look like two
parallel models even though the public compatibility surface is already handled
by exported aliases.

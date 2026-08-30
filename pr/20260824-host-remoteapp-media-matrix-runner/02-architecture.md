# Architecture

`host-remoteapp-media-adaptation-e2e.sh` is the host orchestration adapter. It
executes the EasyNet Browser lifecycle runner three times and owns only fixture
application/reset. Raw per-scenario evidence remains immutable input to
`aggregate-remoteapp-media-adaptation-evidence.py`; the existing
`remoteapp-media-adaptation-e2e.sh` remains the fail-closed acceptance owner.

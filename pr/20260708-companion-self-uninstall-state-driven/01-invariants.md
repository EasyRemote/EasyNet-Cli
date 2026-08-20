# Invariants

- Self uninstall enumerates desktop companion desired-state records.
- A matched package is removed through the same manager/supervisor path as
  plugin remove.
- An orphan desired-state record does not survive uninstall.
- Orphan companion status directories are removed best-effort and reported as
  partial cleanup.
- Normal plugin remove/update transactions are unchanged.

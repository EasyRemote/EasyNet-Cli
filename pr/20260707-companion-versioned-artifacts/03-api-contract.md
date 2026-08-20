# API Contract

Internal contract:
- macOS installed app path: local state app root, package id, package version, app bundle name.
- Windows installed app path: local state app root, package id, package version, app directory name.
- A launcher entry that points elsewhere is treated as not owned by the plan.

Error behavior:
- Missing launcher entries are unchanged/idempotent.
- Filesystem delete errors for owned artifacts remain real cleanup failures.

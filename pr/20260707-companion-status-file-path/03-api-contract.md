# API Contract

Internal contract:
- `DesktopCompanionPlan.status_file` is an absolute path when `health = status_file`.
- Manifest paths under `state/` or `companions/` are state-relative.
- Other relative paths are package-relative.

Error behavior:
- Invalid status-file JSON or mismatched package fields project to `HealthError/status_file_invalid`.
- Missing status file returns no heartbeat observation so process fallback behavior stays controlled by the platform observer and health mode.

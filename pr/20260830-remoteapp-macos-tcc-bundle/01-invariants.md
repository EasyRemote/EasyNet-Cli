# Invariants

1. Screen Recording remains owned by the killable media-host process.
2. The macOS TCC identity has one Developer-ID-signed designated requirement
   and application bundle identifier: `run.easynet.remoteapp.media-host`.
3. macOS launches the sibling `.app` through LaunchServices; Linux and Windows
   retain their flat sibling executables.
4. Permission denial remains a typed `target_permission_missing` terminal
   result with `frontend_action=request_permission`.
5. Media-host build-id fencing hashes the exact executable that is spawned.
6. Linux and Windows binaries do not receive the macOS metadata section.
7. The daemon and target observer never infer media-host TCC state from their
   own distinct code identities.

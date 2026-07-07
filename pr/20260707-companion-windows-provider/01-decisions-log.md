# Decisions Log

## Installed App Ownership

The package root is an installable source, not the Windows user-session app location. The supervisor now copies `EasyNetTray.exe` into the user's EasyNet app directory and registers startup against that installed path.

## Process Fallback

The Windows app may not have a fresh heartbeat during early migration. The supervisor therefore keeps status-file observation as preferred and adds process fallback by image name, matching the SPEC acceptance rule without moving classification logic into SDKs or CLI.

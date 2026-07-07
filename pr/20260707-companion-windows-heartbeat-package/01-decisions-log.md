# Decisions Log

## Heartbeat In App Process

The heartbeat belongs in the companion process because it proves the actual UI process is alive. The daemon reads and classifies the file through the shared observer; the tray app does not project daemon DTOs.

## Release Script Dist Ownership

`plugin.toml` declares the Windows executable inside `plugins/desktop-menubar/dist/windows/EasyNetTray`. The Windows release helper now publishes directly to that directory so package hashing and install validation see the same surface the manifest declares.

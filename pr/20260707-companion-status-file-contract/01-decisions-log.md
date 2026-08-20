# Decisions Log

## Validate In Shared Observer

The status-file contract is language-neutral and platform-neutral. Enforcing it in `CompanionStatusFileObserver` keeps macOS, Windows, and future Linux support on a single classification model.

## Invalid File Blocks Fallback

A present but invalid status file is a health error, not absence. Falling back to process-name observation would hide corrupt or incompatible companion heartbeat payloads, so the observer returns an explicit health observation for invalid files.

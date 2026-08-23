# Intent

Enable real interactive macOS window/application RemoteApp sessions without
allowing global CGEvent injection to escape the selected target.

Acceptance requires an explicit input-control consent grant, a `target_local`
session scope, client geometry/focus epochs, and a fresh host-side identity,
visibility, focus, window-set, and geometry proof immediately before each OS
event is posted.

This change does not claim Windows/Linux input support or replace the live
input-effect E2E requirement.

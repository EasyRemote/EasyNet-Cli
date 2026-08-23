# Verification

Planned verification:

- compile the bundled receiver with `remote-desktop`;
- fixture smoke proving selected/unrelated AppKit logs are independently written;
- host runner self-test proving evidence joins reject missing, mismatched, or
  leaked events;
- existing input-injection verifier and lifecycle/input boundary gate;
- RemoteApp Rust tests affected by the receiver/session projections;
- a real macOS host run when Screen Recording and Accessibility permissions are
  available to the daemon process.

A runner that merely compiles is not sufficient. Readiness remains `partial`
until the real run contains both daemon applied events and AppKit observer
events for the selected target, with no matching event in the unrelated target.

Observed on 2026-08-24:

- the receiver compiled with the `remote-desktop` feature;
- the AppKit selected/unrelated fixture compiled, launched, focused the selected
  window, exported independent event-log paths, and cleaned up successfully;
- the input evidence verifier self-test and product-closure audit passed;
- the first real window runner attempt reached `remote_desktop.create_session`
  and failed closed with `target_permission_missing` because macOS Screen
  Recording was not granted to `target/debug/easynet-daemon`.

No live E2E-14 pass artifact exists yet. Accessibility and Screen Recording
must be granted to that exact daemon binary, the daemon restarted, and both
window and application runner modes executed before readiness can advance.

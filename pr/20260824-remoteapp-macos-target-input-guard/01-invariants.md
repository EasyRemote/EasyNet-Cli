# Invariants

1. Input authority remains bound to the admitted RemoteApp session, selected
   Resource URA, consent receipt, and current transport epoch.
2. Interactive window/application scope is granted only with explicit
   input-control consent on a platform with a target guard implementation.
3. The client-supplied geometry revision and focus epoch must match committed
   session state before host inspection begins.
4. The host re-enumerates the selected target immediately before CGEvent
   creation and rejects identity, visibility, focus, geometry, display, or
   application window-set drift.
5. Window focus means the selected window is the frontmost regular visible
   window of the frontmost process; process focus alone is insufficient.
6. Application input requires the frontmost window to belong to the exact
   committed display-scoped application window set.
7. No session/store lock is held while querying platform state or posting an
   OS event.
8. Input frames and rejection diagnostics remain bounded and replay-safe.
9. Raw or normalized pointer coordinates are clamped inside the committed
   target geometry; scroll events are assigned the mapped target location
   before posting and cannot reuse an unrelated global cursor location.

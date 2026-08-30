# Intent — RemoteApp Product Closure

The goal is to make RemoteApp a product-complete interactive remote desktop
surface while preserving EasyNet ontology and plugin boundaries.

This task must not redefine success around the already-completed v8/raw-stream
or targeted-session boundary work. Those are prerequisites, not product
completion.

## Required closure scope

- Real application/window/display selection and stable capture on supported OSes.
- Pointer/keyboard input injection that is safe, permissioned, low-latency, and
  target-scoped.
- Audio/video codec, frame-rate, bitrate adaptation, and drop policy that are
  tested as end-to-end product behavior.
- Multi-window and multi-application tracking as execution effects.
- Disconnect/reconnect, resume, consent revoke, cancel, timeout, and crash
  recovery.
- Direct, STUN, TURN, and EasyNet relay paths with real route evidence.
- Frontend full lifecycle: discover, authorize, start, display, control, end.
- Cross-device smoke/regression beyond local provider boundaries.

# Plan — RemoteApp Input Event Visibility Gate

## Product invariant

RemoteApp input execution must be inspectable at the product surface. A rejected
input frame is not merely a daemon log; it is user-visible interactive-session
state.

## Boundary

- EasyNet-Cli owns the RemoteApp product contract and cross-repo gate.
- The RemoteApp plugin owns OS input execution and emits session events.
- The EasyNet frontend owns user-visible status projection.
- Axon Invocation semantics remain unchanged.

## Required frontend behavior

1. `remoteDesktopSessionEventRecovery` consumes `INPUT_FRAME_REJECTED`.
2. `INPUT_FRAME_REJECTED` updates RemoteApp status with the daemon reason.
3. `INPUT_FRAME_REJECTED` does not close media/WebRTC transport by default.
4. `INPUT_CHANNEL_OPENED` with blocked activation surfaces the block reason.
5. Tests cover both rejection and blocked activation events.

## Gate updates

The frontend invocation boundary gate must reject frontend code that ignores
daemon input rejection/activation events or treats ordinary input rejection as a
terminal media failure.

## Product effect

Users and E2E harnesses can observe why interactive input did not take effect.
This still does not prove successful low-latency OS input injection across
macOS/Windows/Linux.

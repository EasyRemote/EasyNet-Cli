## Boundary

`DaemonStartConfig` is the daemon SDK boundary for process lifecycle. It owns
launch path materialization before the daemon process exists. Path defaults must
therefore be explicit runtime lifecycle state, not implicit compatibility with a
caller shell's current directory.

## Refactoring direction

- Introduce a typed daemon lifecycle error for unavailable home resolution.
- Make launch path resolution fallible.
- Keep endpoint, discovery, pid, and log paths derived from the same resolved
  home directory.
- Preserve public start API shape: `start()` already returns `Result`.

## Ownership

- `DaemonStartConfig` owns launch-time state directory resolution.
- `DaemonConfig` owns configured invocation endpoint parsing after the state
  root is known.
- FFI and language SDKs consume the typed daemon start failure through the
  existing `Result`/error projection surfaces.

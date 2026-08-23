# API Contract

`PluginBidiWireKind` supports:

- `json_frames`: JSON-only bidi control/data frames.
- `metadata_json_plus_binary`: JSON metadata/control frames plus raw binary media chunks.

RemoteApp `remote_desktop.attach` must use `metadata_json_plus_binary`.

The runtime wire adapter may still map this to the existing `JsonFrames` local adapter while the adapter is the only binary-capable mixed-frame execution path. The public product contract must not describe RemoteApp attach as JSON-only.

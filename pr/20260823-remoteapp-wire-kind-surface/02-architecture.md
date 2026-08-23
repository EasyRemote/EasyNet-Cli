# Architecture

## Boundary decision

`bidi_wire_kind` belongs in the EasyNet-Cli plugin product surface:

- `plugin.toml` owns the plugin package declaration.
- compiled RemoteApp registration owns the builtin executable binding.
- `PluginAbilitySurfaceRecord` owns frontend/operator discovery projection.
- Axon Invocation still owns lifecycle, receipt, sequence, terminal, and error
  semantics.

This change deliberately does not add a new Invocation primitive and does not
make frontend infer transport from an ability name.

## Product effect

RemoteApp attach remains `call_mode = bidi`, but frontend/catalog consumers can
now distinguish:

- `json_frames`: JSON-only bidi control frames.
- `metadata_json_plus_binary`: JSON metadata/control plus raw binary media
  payload chunks.

That distinction is required before a real RemoteApp UI can choose the correct
viewer pipeline for high-frequency video/window streams.


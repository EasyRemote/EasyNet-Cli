# RemoteApp wire-kind surface projection

## Intent

Make the RemoteApp `remote_desktop.attach` mixed media data-plane contract visible
to operator/frontend plugin discovery.

The prior boundary had the correct declaration in the plugin manifest and
compiled registration, but the product-facing `PluginAbilitySurfaceRecord`
projected only `call_mode = bidi`. A frontend or catalog consumer could not tell
whether a bidi ability was JSON-only control traffic or metadata JSON plus raw
binary media payloads.

## Non-goals

- Do not redefine Axon Invocation semantics.
- Do not move binary payload lifecycle ownership out of Runtime Core.
- Do not claim full RemoteApp product completion.
- Do not touch the parallel RemoteApp dispatcher/bidi dirty files.

## Required invariant

For every plugin ability with a declared `bidi_wire_kind`, the operator/frontend
plugin surface must project that exact declared wire kind. Missing projection is
a product discovery bug because it forces the UI to infer data-plane behavior
from `call_mode`.


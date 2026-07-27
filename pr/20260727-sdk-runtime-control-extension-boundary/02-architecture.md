Layering:
- Provider wire: daemon `control.json` may carry provider extension metadata.
- SDK control parser: validates canonical runtime discovery fields and tolerates named provider extensions.
- SDK public model: exposes only canonical runtime lifecycle facts.

Refactor:
- Remove `pagesPort` from Go `controlDiscovery`.
- Keep `PagesPort` only as an ignored wire field in the Go JSON struct.
- Remove `pages_port` from Python `_ControlDiscovery` state and from public `RuntimeControlDiscovery`.
- Keep `"pages_port"` in the Python allowed raw field set as an ignored provider extension.

Result:
- Current daemon discovery remains attachable.
- SDK users no longer see Pages product state as part of the canonical runtime environment.

# Intent

Publish the executable bidi frame contract of builtin plugins into the live
runtime catalog.

The real RemoteApp browser flow is currently disabled even though
`remote_desktop.attach` is a dedicated executable bidi route. Its compiled
plugin spec declares `metadata_json_plus_binary`, but the builtin contribution
adapter drops that field while constructing the registry manifest. The daemon
therefore publishes `bidi_wire_kind = null`, and the frontend correctly refuses
to open an ambiguous data plane.

Acceptance criteria:

- Builtin plugin registry manifests preserve their declared bidi wire kind.
- Remote Desktop publishes `metadata_json_plus_binary`.
- Browser CDP publishes `json_frames` through the same generic adapter.
- Non-bidi abilities remain unchanged.
- A restarted live daemon exposes the field through EasyNet's catalog and
  enables the Remote Desktop launcher.

# Intent

Goal: make the RemoteApp `remote_desktop.attach` bidi data-plane declaration match its real product behavior.

The attach path sends metadata JSON followed by raw binary media frames. Keeping the plugin manifest and compiled ability spec as `json_frames` makes the public ability contract look JSON-only even though RemoteApp depends on mixed metadata/binary frames for interactive desktop media.

Non-goals:

- Do not introduce a new Invocation primitive.
- Do not edit the in-flight dispatcher files in this checkout.
- Do not claim full RemoteApp product completion.
- Do not replace WebRTC/native media product evidence with diagnostic InvokeBidi evidence.

Acceptance criteria:

- The plugin manifest accepts and declares `metadata_json_plus_binary`.
- The compiled RemoteApp attach spec declares the same wire kind.
- The daemon ability wire registry maps the product declaration onto the existing binary-capable local adapter.
- Closure audit rejects RemoteApp attach drifting back to JSON-only.

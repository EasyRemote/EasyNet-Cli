# API contract

- `resource.refresh_remote_targets` publishes canonical display/application/window Resources.
- `remote_desktop.create_session` commits one selected Resource subject and binding.
- `remote_desktop.set_description` starts production WebRTC negotiation against that binding.
- Provider failure remains explicit; no display fallback is permitted for application/window subjects.

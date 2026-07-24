# Decisions Log

## 2026-07-24

- Decision: migrate lifecycle/transport to `providers.runtime`.
- Reason: these modules are runtime host provider seams, not EasyNet product features.
- Decision: remove the credentials identity adapter instead of moving it.
- Reason: it maps product credential fields (`device_id`, `hub_endpoint`, `username`) and should not become canonical SDK runtime surface.
- Decision: rename Python lifecycle DTOs to `RuntimeHostMode`, `RuntimeHostStartConfig`, and `RuntimeHostDiscoverConfig`.
- Reason: daemon-named DTOs would preserve product lifecycle naming after migration.
- Decision: move C ABI start payloads toward `runtime_instance_id` and `runtime_bin`.
- Reason: keeping `device_id` and `daemon_bin` in the SDK provider would retain product lifecycle vocabulary at the runtime provider boundary.

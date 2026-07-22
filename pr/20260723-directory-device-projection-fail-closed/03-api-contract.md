# API Contract

Internal contract changes:

- `presence_ura_to_directory_entry` returns `Result<DirectoryEntry, String>`.
- `presence_ura_to_directory_agent_summary` returns `Result<DirectoryAgentSummary, String>`.
- `presence_uras_to_directory_snapshot` returns `Result<DirectoryEvent, String>`.
- `presence_event_to_directory_event(_at)` returns `Result<DirectoryEvent, String>`.
- `DirectoryView::apply_frame` returns `Result<(), String>` and commits snapshot changes atomically after validation.

Public product behavior remains a directory stream of canonical Device URA rows. Non-canonical presence rows now fail the stream route instead of being published with synthetic fallback node ids.

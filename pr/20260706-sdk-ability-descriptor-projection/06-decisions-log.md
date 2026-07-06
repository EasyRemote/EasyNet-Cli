# Decisions Log

## 2026-07-06

- Keep descriptor metadata interpretation out of SDK. Metadata keys such as display labels, runtime labels, or product categories remain downstream product conventions.
- Implement pure projection rather than importing Axon SDK into the public Go SDK because SDK import-boundary explicitly bans raw Axon dependencies.
- Add the projection to both Go and Python SDKs in the same turn to keep the capability matrix convergent instead of creating a Go-only architectural branch.

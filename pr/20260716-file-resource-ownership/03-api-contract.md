# API Contract

- `fs.read`, `fs.write`, `fs.stat`, `fs.list`, `fs.edit`, and `fs.transfer` are
  system descriptors with Device authority.
- `files.put`, `files.get`, and `files.list` are user-owned, owner-local
  abilities.
- Files resource URAs keep the canonical object shape
  `easynet:///r/<realm>/resource/<user>.files/<sha256>`.
- OpenAI compatibility exposes `openai.files.*` as Device-owned facade
  abilities and does not publish `files.*` as Device-owned abilities.

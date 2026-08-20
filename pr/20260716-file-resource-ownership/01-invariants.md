# Invariants

- Device filesystem abilities act on host filesystem ResourceRefs and remain
  Device-owned system abilities.
- User blob files are state objects under a user resource URA; their transition
  abilities are user-owned and executed by a daemon-native files Agent.
- Ability names are owner-local (`files.put`, `files.get`, `files.list`) unless
  the ability contract is intentionally project-qualified, as with Pages fetch
  and API abilities.
- No hidden fallback from owner-local names to legacy `<user>.files.*` names.

# Invariants

- Teach grant authority must be bound to the advertising Agent URA or a signed hosted-Agent delegation for that Agent.
- Hosted display names must never silently select one profile when more than one hosted row has the same name.
- Hosted identity URAs must parse as Agent URAs before authority checks consume them.
- Signing authority remains persisted hosted identity data, but callers consume it through the aggregate projection rather than the local file shape.
- Public teach/acquire/forget request and response contracts remain unchanged.

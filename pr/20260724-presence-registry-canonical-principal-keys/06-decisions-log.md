# Decisions Log

## 2026-07-24

- Decision: allow Device, User, and Agent URAs as canonical presence principals.
- Reason: Device sessions, user-authority revocation, and hosted-agent revocation paths all use presence liveness; the defect is accepting malformed strings, not the existence of multiple principal kinds.

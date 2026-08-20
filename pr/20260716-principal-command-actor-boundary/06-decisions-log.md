# Decisions Log

- 2026-07-16: Keep source-compatible CLI omission behavior, but move it into a
  named `PrincipalCommandActor` state. This preserves operator workflows while
  making audit actor source inspectable before daemon dispatch.
- 2026-07-16: The architecture gate checks the serializer boundary rather than
  banning subject-self behavior globally. The concrete use case is preserving
  bootstrap/enrollment ergonomics while preventing `principal_command` from
  owning fallback policy.

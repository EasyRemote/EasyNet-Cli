# Intent

Goal: make Linux desktop companion load planning report unsupported graphical sessions instead of projecting a false loaded state.

Non-goals:
- Do not block daemon boot when a graphical session is unavailable.
- Do not implement Linux tray installation in this slice.
- Do not probe macOS LaunchAgent or Windows registry state from the load planner.

Acceptance criteria:
- Linux companion packages with valid Linux specs and no graphical session produce `CompanionUnsupportedSession`.
- The load plan still carries the companion plan so status surfaces can expose package metadata.
- Linux supervisor and load planner share one session-probe abstraction.

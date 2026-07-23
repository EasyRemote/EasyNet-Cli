# API Contract

## Public Behavior

Until a real browser provider exists, the canonical daemon runtime does not
publish browser abilities.

## Error Shape

Callers attempting browser abilities should fail at discovery/route resolution
because the abilities are absent, not because a placeholder handler returns mock
or timeout state.

## Reintroduction Rule

Any future browser ability descriptor must be paired with a production handler
and lifecycle tests in the same convergence slice. A descriptor-only addition is
invalid.

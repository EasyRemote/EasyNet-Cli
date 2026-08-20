# Implementation Plan

1. Add `LocalAgentsLoadState`.
2. Add `load_with_state`.
3. Add `load_for_fresh_host_projection`.
4. Migrate production local-agent identity callers to the projection helper.
5. Add missing-state and first-boot projection tests.
6. Update SPEC v2 gate and self-test fixtures.

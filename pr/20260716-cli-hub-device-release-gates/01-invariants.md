# Invariants

1. Product runtime process ownership stays in `easynet-daemon`; the E2E harness
   must not depend on the Go backend or raw Axon runtime.
2. Key material custody stays in the daemon key service process; local install
   flows must include `easynet-keyring` whenever they install daemon binaries.
3. `control.sock` remains boot/status only. Product calls in the harness must use
   CLI commands that lower to daemon Invocation.
4. The full E2E may be expensive, but the checked-in harness must have a cheap
   self-test that validates syntax, prerequisites, and core command coverage.
5. Release contract drift must fail before live SDK cutover gates consume the
   package shape.
6. No compatibility fallback is allowed for missing `easynet-keyring`; missing
   binary is a build/install contract error.

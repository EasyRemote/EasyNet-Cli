# Execution checklist

- [x] Read mandatory repository and engineering-contract instructions.
- [x] Confirm all sibling worktrees were clean before cross-repository edits.
- [x] Reproduce Python lock drift and full CLI lib-suite failure against Axon `2ad067dc`.
- [x] Identify and repair the smallest common roots of the 117 CLI failures.
- [x] Re-run full CLI lib admission against committed Axon contract revision `bf944455`.
- [x] Add and self-test `compatibility/axon.lock.json` plus `check-axon-lock.py`.
- [x] Replace repeated workflow revision/version checks with manifest resolution.
- [x] Add workflow-integrity and stable `main-admission` aggregation.
- [x] Add explicit Candidate and Artifact channels.
- [x] Run focused and full CLI verification.
- [x] Commit semantic units with the required identity.

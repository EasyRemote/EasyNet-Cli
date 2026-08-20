# Decisions Log

## 2026-07-27

- Chose a semantic rename instead of behavior change because the current code already uses `_system.local` and trusted local-system admission; the defect is the stale transport-derived authority vocabulary.
- Kept genuine network-loopback vocabulary out of scope. This slice only retires loopback wording where it described daemon-local system authority.
- Updated convergence gates with the same vocabulary so the architecture guard does not preserve the retired `LocalDaemonLoopback*` abstraction.

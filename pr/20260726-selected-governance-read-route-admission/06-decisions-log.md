Decisions
=========

- 2026-07-26: Treat governance-read admission as selected-route policy rather
  than remote-carrier policy. The product failure mode shows LocalRuntime
  selected routes can otherwise defer receipt-history subject mismatch to Axon
  admission, recreating a legacy second authority path.
- 2026-07-26: Kept the public error code stable but normalized the target-owned
  receipt-history message to point at the canonical invocation history read
  path. This gives product callers one remediation path instead of separate
  remote/local wording.

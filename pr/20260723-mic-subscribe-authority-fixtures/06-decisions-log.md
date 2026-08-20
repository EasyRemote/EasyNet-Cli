# Decisions Log

- Decision: keep the Device authority literal module-local.
  Rationale: `mic.subscribe` tests exercise one Device-hosted media stream
  surface; centralizing this in the module avoids widening production API while
  making the authority invariant explicit.

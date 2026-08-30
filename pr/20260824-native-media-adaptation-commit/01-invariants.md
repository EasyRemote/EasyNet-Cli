# Invariants

- Feedback observation may propose a bitrate but cannot publish it as active.
- The encoder applies the proposal before the controller commits it.
- Only an applied proposal emits `bitrate_downshift` or `bitrate_upshift`.
- A rejected proposal retains the last active bitrate and emits an operational
  failure event with requested and active values.
- This change does not claim live degraded-network or backpressure product
  evidence; those still require a real impairment runner.

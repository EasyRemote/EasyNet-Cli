# API Contract

Accepted terminal-state strings:

- `Completed`
- `Failed`
- `TimedOut`
- `Cancelled`

Rejected terminal-state strings include non-canonical capitalization such as `completed`, `FAILED`, `TIMED_OUT`, and `cancelled`.

Error behavior:

- A non-canonical state returns the existing non-terminal/invalid terminal projection error.
- Public output names are unchanged.

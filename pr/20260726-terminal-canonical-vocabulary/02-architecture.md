# Architecture

The Terminal subsystem has three cohesive layers:

- lifecycle RPC: `terminal.create`, `terminal.list`, `terminal.close`;
- unary data-plane RPC: `terminal.input`, `terminal.read`, `terminal.resize`;
- BIDI data plane: `terminal.attach`.

All three layers use a PTY backend, but the backend name is not the runtime concept. This refactor keeps PTY types where they describe OS handles and removes PTY-session vocabulary from ability-level ownership.

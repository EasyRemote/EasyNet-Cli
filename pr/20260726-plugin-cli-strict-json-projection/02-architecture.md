# Architecture

The boundary is:

1. Daemon plugin runtime constructs canonical output reports.
2. CLI receives those reports as JSON values through the plugin control ability.
3. CLI JSON mode prints the value unchanged.
4. CLI table mode validates the required fields it renders and then projects rows.

The CLI does not own daemon report state machines, and it does not repair malformed daemon report rows.


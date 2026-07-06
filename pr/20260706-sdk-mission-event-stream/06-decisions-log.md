# Decisions Log

- 2026-07-06: Treat Mission event stream as a Mission profile typed adapter over Runtime Core `StreamHandle`, not a new Mission-owned stream lifecycle.
- 2026-07-06: Keep remaining Mission gaps explicit: daemon-side child Invocation execution, scheduler/retry policy, and backend automation cutover are not completed by this stream adapter.

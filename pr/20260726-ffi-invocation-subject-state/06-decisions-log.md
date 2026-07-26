Decision log:

- Treat control discovery Ready as a runtime state-machine contract, not an
  attach convenience flag bag.
- Preserve public attach shape while making invalid Device/Both readiness fail
  before the daemon advertises Ready.
